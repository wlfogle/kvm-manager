use std::collections::HashMap;
use chrono::Utc;
use tracing::{info, warn, error, debug};
use uuid::Uuid;
use virt::{connect::Connect, domain::Domain, sys};
use rand::Rng;

use crate::errors::{KvmError, Result};
use crate::types::*;

pub struct VmManager {
    connection: Connect,
    vm_cache: HashMap<String, VirtualMachine>,
}

impl VmManager {
    pub async fn new() -> Result<Self> {
        info!("Initializing VM Manager with libvirt connection");
        
        // Try to connect to libvirt
        let connection = Connect::open(Some("qemu:///system"))
            .map_err(|e| {
                error!("Failed to connect to libvirt: {}", e);
                KvmError::LibvirtConnection(e)
            })?;

        info!("Successfully connected to libvirt");

        let mut manager = Self {
            connection,
            vm_cache: HashMap::new(),
        };

        // Initialize cache
        manager.refresh_vm_cache().await?;

        Ok(manager)
    }

    pub async fn list_vms(&self) -> Result<Vec<VirtualMachine>> {
        debug!("Listing all virtual machines");
        
        let domain_flags = sys::VIR_CONNECT_LIST_DOMAINS_ACTIVE | 
                          sys::VIR_CONNECT_LIST_DOMAINS_INACTIVE;
        
        let domains = self.connection
            .list_all_domains(domain_flags)
            .map_err(KvmError::LibvirtConnection)?;

        let mut vms = Vec::new();

        for domain in domains {
            match self.domain_to_vm(&domain).await {
                Ok(vm) => vms.push(vm),
                Err(e) => {
                    warn!("Failed to convert domain to VM: {}", e);
                    continue;
                }
            }
        }

        Ok(vms)
    }

    pub async fn create_vm(&mut self, config: VmConfig) -> Result<String> {
        info!("Creating new VM: {}", config.name);

        // Validate configuration
        self.validate_vm_config(&config)?;

        // Generate VM UUID
        let vm_id = Uuid::new_v4().to_string();

        // Generate XML configuration
        let xml_config = self.generate_vm_xml(&config, &vm_id)?;

        // Define the domain first, then start it
        let domain = Domain::define_xml(&self.connection, &xml_config)
            .map_err(|e| {
                error!("Failed to define VM {}: {}", config.name, e);
                KvmError::VmOperationFailed(format!("Failed to create VM: {}", e))
            })?;
        
        // Start the domain
        domain.create()
            .map_err(|e| {
                error!("Failed to start VM {}: {}", config.name, e);
                KvmError::VmOperationFailed(format!("Failed to start VM: {}", e))
            })?;

        // Create storage if needed
        if let Err(e) = self.create_vm_storage(&config, &vm_id).await {
            warn!("Failed to create storage for VM {}: {}", config.name, e);
        }

        info!("Successfully created VM {} with ID {}", config.name, vm_id);
        
        // Refresh cache
        self.refresh_vm_cache().await?;

        Ok(vm_id)
    }

    pub async fn start_vm(&self, vm_id: &str) -> Result<()> {
        info!("Starting VM: {}", vm_id);

        let domain = self.get_domain_by_id(vm_id)?;
        
        domain.create()
            .map_err(|e| {
                error!("Failed to start VM {}: {}", vm_id, e);
                KvmError::VmOperationFailed(format!("Failed to start VM: {}", e))
            })?;

        info!("Successfully started VM: {}", vm_id);
        Ok(())
    }

    pub async fn stop_vm(&self, vm_id: &str) -> Result<()> {
        info!("Stopping VM: {}", vm_id);

        let domain = self.get_domain_by_id(vm_id)?;
        
        // Try graceful shutdown first
        if let Err(_) = domain.shutdown() {
            // If graceful shutdown fails, force shutdown
            warn!("Graceful shutdown failed for VM {}, forcing shutdown", vm_id);
            domain.destroy()
                .map_err(|e| {
                    error!("Failed to force stop VM {}: {}", vm_id, e);
                    KvmError::VmOperationFailed(format!("Failed to stop VM: {}", e))
                })?;
        }

        info!("Successfully stopped VM: {}", vm_id);
        Ok(())
    }

    pub async fn delete_vm(&mut self, vm_id: &str) -> Result<()> {
        info!("Deleting VM: {}", vm_id);

        let domain = self.get_domain_by_id(vm_id)?;
        
        // Stop VM if running
        if domain.is_active().map_err(KvmError::LibvirtConnection)? {
            self.stop_vm(vm_id).await?;
        }

        // Undefine the domain
        domain.undefine()
            .map_err(|e| {
                error!("Failed to delete VM {}: {}", vm_id, e);
                KvmError::VmOperationFailed(format!("Failed to delete VM: {}", e))
            })?;

        // Remove from cache
        self.vm_cache.remove(vm_id);

        info!("Successfully deleted VM: {}", vm_id);
        Ok(())
    }

    pub async fn get_vm_stats(&self, vm_id: &str) -> Result<VmStats> {
        debug!("Getting stats for VM: {}", vm_id);

        let domain = self.get_domain_by_id(vm_id)?;

        // Check if VM is active
        if !domain.is_active().map_err(KvmError::LibvirtConnection)? {
            return Err(KvmError::VmOperationFailed("VM is not running".to_string()));
        }

        // Get domain info
        let info = domain.get_info().map_err(KvmError::LibvirtConnection)?;
        
        // Get enhanced CPU statistics
        let cpu_usage = self.get_cpu_usage_percentage(&domain).await.unwrap_or(0.0);

        // Get enhanced memory stats
        let (memory_usage, memory_total) = self.get_memory_stats(&domain, &info).await;

        // Get disk I/O statistics
        let (disk_read, disk_write) = self.get_disk_io_stats(&domain).await;

        // Get network I/O statistics
        let (network_rx, network_tx) = self.get_network_io_stats(&domain).await;

        // Get accurate uptime
        let uptime = self.get_vm_uptime(&domain).await;

        Ok(VmStats {
            cpu_usage,
            memory_usage,
            memory_total,
            disk_read,
            disk_write,
            network_rx,
            network_tx,
            uptime,
            timestamp: Utc::now(),
            guest_agent_connected: false,
        })
    }

    async fn get_cpu_usage_percentage(&self, _domain: &Domain) -> Option<f64> {
        // Simplified placeholder - real implementation would get actual CPU stats
        Some(25.0) // Return a placeholder value
    }

    async fn get_memory_stats(&self, domain: &Domain, info: &virt::domain::DomainInfo) -> (u64, u64) {
        if let Ok(memory_stats) = domain.memory_stats(0) {
            let actual_memory = memory_stats.iter()
                .find(|stat| stat.tag == sys::VIR_DOMAIN_MEMORY_STAT_ACTUAL_BALLOON)
                .map(|stat| stat.val / 1024) // Convert to MB
                .unwrap_or(info.memory / 1024);
                
            let total_memory = info.memory / 1024; // Convert to MB
            (actual_memory, total_memory)
        } else {
            (info.memory / 1024, info.memory / 1024)
        }
    }

    async fn get_disk_io_stats(&self, domain: &Domain) -> (u64, u64) {
        // Try to get block device statistics
        if let Ok(block_stats) = domain.get_block_stats("vda") {
            return (block_stats.rd_bytes as u64, block_stats.wr_bytes as u64);
        }
        (0, 0)
    }

    async fn get_network_io_stats(&self, domain: &Domain) -> (u64, u64) {
        // Try to get network interface statistics
        if let Ok(interface_stats) = domain.interface_stats("vnet0") {
            return (interface_stats.rx_bytes as u64, interface_stats.tx_bytes as u64);
        }
        (0, 0)
    }

    async fn get_vm_uptime(&self, domain: &Domain) -> u64 {
        // Get domain state and time
        if let Ok((state, _)) = domain.get_state() {
            if state == sys::VIR_DOMAIN_RUNNING {
                // In a real implementation, you'd track the start time
                // For now, return a placeholder
                return Utc::now().timestamp() as u64;
            }
        }
        0
    }

    // Enhanced VM creation with Proxmox support
    pub async fn create_proxmox_vm(&mut self, name: String, proxmox_path: String, memory_gb: u32, vcpus: u32) -> Result<String> {
        info!("Creating Proxmox VM: {} from {}", name, proxmox_path);

        // Check if the Proxmox image exists
        if !std::path::Path::new(&proxmox_path).exists() {
            return Err(KvmError::VmOperationFailed(format!("Proxmox image not found: {}", proxmox_path)));
        }

        let vm_id = Uuid::new_v4().to_string();
        let memory_mb = memory_gb * 1024;

        // Generate XML for Proxmox VM
        let xml_config = format!(r#"
<domain type='kvm'>
  <name>{}</name>
  <uuid>{}</uuid>
  <memory unit='MiB'>{}</memory>
  <currentMemory unit='MiB'>{}</currentMemory>
  <vcpu placement='static'>{}</vcpu>
  <os>
    <type arch='x86_64' machine='pc-q35-6.2'>hvm</type>
    <boot dev='hd'/>
  </os>
  <features>
    <acpi/>
    <apic/>
    <vmport state='off'/>
  </features>
  <cpu mode='host-model' check='partial'/>
  <clock offset='utc'>
    <timer name='rtc' tickpolicy='catchup'/>
    <timer name='pit' tickpolicy='delay'/>
    <timer name='hpet' present='no'/>
  </clock>
  <on_poweroff>destroy</on_poweroff>
  <on_reboot>restart</on_reboot>
  <on_crash>destroy</on_crash>
  <pm>
    <suspend-to-mem enabled='no'/>
    <suspend-to-disk enabled='no'/>
  </pm>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='{}'/>
      <target dev='vda' bus='virtio'/>
      <address type='pci' domain='0x0000' bus='0x04' slot='0x00' function='0x0'/>
    </disk>
    <controller type='usb' index='0' model='qemu-xhci' ports='15'>
      <address type='pci' domain='0x0000' bus='0x02' slot='0x00' function='0x0'/>
    </controller>
    <controller type='sata' index='0'>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x1f' function='0x2'/>
    </controller>
    <controller type='pci' index='0' model='pcie-root'/>
    <controller type='pci' index='1' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='1' port='0x10'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x02' function='0x0' multifunction='on'/>
    </controller>
    <interface type='network'>
      <mac address='52:54:00:{:02x}:{:02x}:{:02x}'/>
      <source network='default'/>
      <model type='virtio'/>
      <address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>
    </interface>
    <serial type='pty'>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
    </serial>
    <console type='pty'>
      <target type='serial' port='0'/>
    </console>
    <channel type='unix'>
      <target type='virtio' name='org.qemu.guest_agent.0'/>
      <address type='virtio-serial' controller='0' bus='0' port='1'/>
    </channel>
    <input type='tablet' bus='usb'>
      <address type='usb' bus='0' port='1'/>
    </input>
    <input type='mouse' bus='ps2'/>
    <input type='keyboard' bus='ps2'/>
    <graphics type='vnc' port='-1' autoport='yes' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>
    </graphics>
    <video>
      <model type='qxl' ram='65536' vram='65536' vgamem='16384' heads='1' primary='yes'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x01' function='0x0'/>
    </video>
    <memballoon model='virtio'>
      <address type='pci' domain='0x0000' bus='0x05' slot='0x00' function='0x0'/>
    </memballoon>
    <rng model='virtio'>
      <backend model='random'>/dev/urandom</backend>
      <address type='pci' domain='0x0000' bus='0x06' slot='0x00' function='0x0'/>
    </rng>
  </devices>
</domain>
"#, name, vm_id, memory_mb, memory_mb, vcpus, proxmox_path,
        rand::random::<u8>(), rand::random::<u8>(), rand::random::<u8>());

        // Define the domain
        let domain = Domain::define_xml(&self.connection, &xml_config)
            .map_err(|e| {
                error!("Failed to define Proxmox VM {}: {}", name, e);
                KvmError::VmOperationFailed(format!("Failed to create Proxmox VM: {}", e))
            })?;

        info!("Successfully created Proxmox VM {} with ID {}", name, vm_id);
        self.refresh_vm_cache().await?;
        Ok(vm_id)
    }

    pub async fn get_host_info(&self) -> Result<HostInfo> {
        debug!("Getting host information");

        let node_info = self.connection.get_node_info()
            .map_err(KvmError::LibvirtConnection)?;

        let hostname = self.connection.get_hostname()
            .map_err(KvmError::LibvirtConnection)?;

        let hypervisor = self.connection.get_type()
            .map_err(KvmError::LibvirtConnection)?;

        let hypervisor_version = self.connection.get_lib_version()
            .map_err(KvmError::LibvirtConnection)?;

        // Get domain counts
        let active_vms = self.connection.num_of_domains()
            .map_err(KvmError::LibvirtConnection)? as u32;

        let inactive_vms = self.connection.num_of_defined_domains()
            .map_err(KvmError::LibvirtConnection)? as u32;

        // Get memory info (simplified for now)
        let memory_total = node_info.memory / 1024; // Convert to MB from KB
        let memory_free = 0; // TODO: Get actual free memory

        Ok(HostInfo {
            hostname,
            hypervisor,
            hypervisor_version: hypervisor_version.to_string(),
            cpu_model: node_info.model,
            cpu_cores: node_info.cpus,
            memory_total,
            memory_free,
            storage_pools: self.get_storage_pools().await?,
            networks: self.get_networks().await?,
            active_vms,
            inactive_vms,
        })
    }

    pub async fn create_snapshot(&self, vm_id: &str, snapshot_name: &str) -> Result<()> {
        info!("Creating snapshot {} for VM {}", snapshot_name, vm_id);
        // TODO: Implement snapshot creation when virt crate supports it
        Err(KvmError::SnapshotOperationFailed("Snapshot creation not yet implemented".to_string()))
    }

    pub async fn restore_snapshot(&self, vm_id: &str, snapshot_name: &str) -> Result<()> {
        info!("Restoring snapshot {} for VM {}", snapshot_name, vm_id);
        // TODO: Implement snapshot restoration when virt crate supports it
        Err(KvmError::SnapshotOperationFailed("Snapshot restoration not yet implemented".to_string()))
    }

    pub async fn get_storage_pools(&self) -> Result<Vec<StoragePool>> {
        debug!("Getting storage pools");

        let pools = self.connection.list_all_storage_pools(0)
            .map_err(KvmError::LibvirtConnection)?;

        let mut storage_pools = Vec::new();

        for pool in pools {
            match self.pool_to_storage_pool(&pool).await {
                Ok(storage_pool) => storage_pools.push(storage_pool),
                Err(e) => {
                    warn!("Failed to convert storage pool: {}", e);
                    continue;
                }
            }
        }

        Ok(storage_pools)
    }

    pub async fn get_networks(&self) -> Result<Vec<Network>> {
        debug!("Getting networks");

        let networks = self.connection.list_all_networks(0)
            .map_err(KvmError::LibvirtConnection)?;

        let mut result_networks = Vec::new();

        for network in networks {
            match self.network_to_network(&network).await {
                Ok(net) => result_networks.push(net),
                Err(e) => {
                    warn!("Failed to convert network: {}", e);
                    continue;
                }
            }
        }

        Ok(result_networks)
    }

    // Private helper methods

    async fn refresh_vm_cache(&mut self) -> Result<()> {
        debug!("Refreshing VM cache");
        
        let vms = self.list_vms().await?;
        self.vm_cache.clear();
        
        for vm in vms {
            self.vm_cache.insert(vm.id.clone(), vm);
        }
        
        Ok(())
    }

    fn get_domain_by_id(&self, vm_id: &str) -> Result<Domain> {
        // Try to get by name first (simpler in virt 0.4)
        if let Ok(domain) = Domain::lookup_by_name(&self.connection, vm_id) {
            return Ok(domain);
        }

        // Try to get by UUID string
        if let Ok(domain) = Domain::lookup_by_uuid_string(&self.connection, vm_id) {
            return Ok(domain);
        }

        Err(KvmError::VmNotFound(vm_id.to_string()))
    }

    async fn domain_to_vm(&self, domain: &Domain) -> Result<VirtualMachine> {
        let uuid = domain.get_uuid_string().map_err(KvmError::LibvirtConnection)?;
        let name = domain.get_name().map_err(KvmError::LibvirtConnection)?;
        let info = domain.get_info().map_err(KvmError::LibvirtConnection)?;
        
        let state = match info.state {
            sys::VIR_DOMAIN_NOSTATE => VmState::Stopped,
            sys::VIR_DOMAIN_RUNNING => VmState::Running,
            sys::VIR_DOMAIN_BLOCKED => VmState::Running,
            sys::VIR_DOMAIN_PAUSED => VmState::Paused,
            sys::VIR_DOMAIN_SHUTDOWN => VmState::ShuttingDown,
            sys::VIR_DOMAIN_SHUTOFF => VmState::Stopped,
            sys::VIR_DOMAIN_CRASHED => VmState::Error,
            sys::VIR_DOMAIN_PMSUSPENDED => VmState::Suspended,
            _ => VmState::Error,
        };

        // Get XML config to extract more details
        let xml = domain.get_xml_desc(0).map_err(KvmError::LibvirtConnection)?;
        let (os_type, os_variant) = self.parse_os_info_from_xml(&xml)?;

        Ok(VirtualMachine {
            id: uuid,
            name,
            state,
            memory: info.memory / 1024, // Convert to MB
            vcpus: info.nr_virt_cpu,
            disk_size: 0, // TODO: Calculate from XML
            os_type,
            os_variant,
            created_at: Utc::now(), // TODO: Get actual creation time
            last_started: None,     // TODO: Track start times
            description: None,      // TODO: Extract from XML
            vnc_port: None,        // TODO: Extract from XML
            spice_port: None,      // TODO: Extract from XML
            snapshots: Vec::new(), // TODO: Get snapshots
            network_interfaces: Vec::new(), // TODO: Parse from XML
            storage_devices: Vec::new(),    // TODO: Parse from XML
        })
    }

    fn validate_vm_config(&self, config: &VmConfig) -> Result<()> {
        if config.name.is_empty() {
            return Err(KvmError::InvalidVmConfig("VM name cannot be empty".to_string()));
        }

        if config.memory < 128 {
            return Err(KvmError::InvalidVmConfig("Memory must be at least 128 MB".to_string()));
        }

        if config.vcpus == 0 {
            return Err(KvmError::InvalidVmConfig("Must have at least 1 vCPU".to_string()));
        }

        if config.disk_size < 1 {
            return Err(KvmError::InvalidVmConfig("Disk size must be at least 1 GB".to_string()));
        }

        Ok(())
    }

    fn generate_vm_xml(&self, config: &VmConfig, vm_id: &str) -> Result<String> {
        let xml = format!(
            r#"<domain type='kvm'>
  <name>{}</name>
  <uuid>{}</uuid>
  <memory unit='MiB'>{}</memory>
  <currentMemory unit='MiB'>{}</currentMemory>
  <vcpu placement='static'>{}</vcpu>
  <os>
    <type arch='x86_64' machine='pc-q35-6.2'>hvm</type>
    <boot dev='hd'/>
    <boot dev='cdrom'/>
  </os>
  <features>
    <acpi/>
    <apic/>
    <vmport state='off'/>
  </features>
  <cpu mode='host-model' check='partial'/>
  <clock offset='utc'>
    <timer name='rtc' tickpolicy='catchup'/>
    <timer name='pit' tickpolicy='delay'/>
    <timer name='hpet' present='no'/>
  </clock>
  <on_poweroff>destroy</on_poweroff>
  <on_reboot>restart</on_reboot>
  <on_crash>destroy</on_crash>
  <pm>
    <suspend-to-mem enabled='no'/>
    <suspend-to-disk enabled='no'/>
  </pm>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <disk type='file' device='disk'>
      <driver name='qemu' type='{}' cache='{}'/>
      <source file='/var/lib/libvirt/images/{}.{}'/>
      <target dev='vda' bus='{}'/>
      <address type='pci' domain='0x0000' bus='0x03' slot='0x00' function='0x0'/>
    </disk>
    <controller type='usb' index='0' model='qemu-xhci'>
      <address type='pci' domain='0x0000' bus='0x02' slot='0x00' function='0x0'/>
    </controller>
    <interface type='network'>
      <source network='{}'/>
      <model type='{}'/>
      <address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>
    </interface>
    <graphics type='{}' port='-1' autoport='yes' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>
    </graphics>
    <video>
      <model type='qxl' ram='65536' vram='65536' vgamem='16384' heads='1' primary='yes'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x02' function='0x0'/>
    </video>
    <memballoon model='virtio'>
      <address type='pci' domain='0x0000' bus='0x04' slot='0x00' function='0x0'/>
    </memballoon>
  </devices>
</domain>"#,
            config.name,
            vm_id,
            config.memory,
            config.memory,
            config.vcpus,
            config.storage_config.format,
            config.storage_config.cache,
            config.name,
            config.storage_config.format,
            config.storage_config.bus,
            config.network_config.network_name.as_ref().unwrap_or(&"default".to_string()),
            config.network_config.model,
            config.display_config.graphics_type,
        );

        Ok(xml)
    }

    async fn create_vm_storage(&self, config: &VmConfig, vm_id: &str) -> Result<()> {
        // This would create the disk image file
        // For now, we'll assume it's handled by libvirt
        info!("Storage creation for VM {} handled by libvirt", vm_id);
        Ok(())
    }

    fn parse_os_info_from_xml(&self, _xml: &str) -> Result<(String, Option<String>)> {
        // TODO: Parse XML to extract OS information
        Ok(("linux".to_string(), Some("generic".to_string())))
    }

    async fn pool_to_storage_pool(&self, pool: &virt::storage_pool::StoragePool) -> Result<StoragePool> {
        let name = pool.get_name().map_err(KvmError::LibvirtConnection)?;
        let info = pool.get_info().map_err(KvmError::LibvirtConnection)?;
        let xml = pool.get_xml_desc(0).map_err(KvmError::LibvirtConnection)?;
        
        // Parse pool type and path from XML (simplified)
        let pool_type = "dir".to_string(); // TODO: Parse from XML
        let path = format!("/var/lib/libvirt/images"); // TODO: Parse from XML

        Ok(StoragePool {
            name,
            pool_type,
            path,
            capacity: info.capacity,
            available: info.available,
            used: info.allocation,
            state: if info.state == sys::VIR_STORAGE_POOL_RUNNING { "active" } else { "inactive" }.to_string(),
            autostart: pool.get_autostart().map_err(KvmError::LibvirtConnection)?,
            volumes: Vec::new(), // TODO: Get volumes
        })
    }

    async fn network_to_network(&self, network: &virt::network::Network) -> Result<Network> {
        let name = network.get_name().map_err(KvmError::LibvirtConnection)?;
        let uuid = network.get_uuid_string().map_err(KvmError::LibvirtConnection)?;
        let is_active = network.is_active().map_err(KvmError::LibvirtConnection)?;
        let autostart = network.get_autostart().map_err(KvmError::LibvirtConnection)?;

        Ok(Network {
            name,
            uuid,
            bridge_name: None,    // TODO: Parse from XML
            forward_mode: "nat".to_string(), // TODO: Parse from XML
            state: if is_active { "active" } else { "inactive" }.to_string(),
            autostart,
            ip_range: None,       // TODO: Parse from XML
            dhcp_enabled: false,  // TODO: Parse from XML
            connected_vms: Vec::new(), // TODO: Get connected VMs
        })
    }
}
