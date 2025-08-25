use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualMachine {
    pub id: String,
    pub name: String,
    pub state: VmState,
    pub memory: u64,        // Memory in MB
    pub vcpus: u32,
    pub disk_size: u64,     // Disk size in GB
    pub os_type: String,
    pub os_variant: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_started: Option<DateTime<Utc>>,
    pub description: Option<String>,
    pub vnc_port: Option<u16>,
    pub spice_port: Option<u16>,
    pub snapshots: Vec<Snapshot>,
    pub network_interfaces: Vec<NetworkInterface>,
    pub storage_devices: Vec<StorageDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VmState {
    Running,
    Stopped,
    Paused,
    Suspended,
    ShuttingDown,
    Creating,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub name: String,
    pub memory: u64,
    pub vcpus: u32,
    pub disk_size: u64,
    pub os_type: String,
    pub os_variant: Option<String>,
    pub description: Option<String>,
    pub network_config: NetworkConfig,
    pub storage_config: StorageConfig,
    pub display_config: DisplayConfig,
    pub boot_config: BootConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub bridge: Option<String>,
    pub network_name: Option<String>,
    pub mac_address: Option<String>,
    pub model: String, // e1000, virtio, etc.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub pool_name: String,
    pub format: String, // qcow2, raw, etc.
    pub bus: String,    // virtio, sata, ide, etc.
    pub cache: String,  // none, writeback, writethrough, etc.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub graphics_type: String, // vnc, spice
    pub listen: String,
    pub password: Option<String>,
    pub autoport: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootConfig {
    pub boot_order: Vec<String>, // cdrom, hd, network
    pub iso_path: Option<String>,
    pub kernel: Option<String>,
    pub initrd: Option<String>,
    pub cmdline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmStats {
    pub cpu_usage: f64,        // Percentage
    pub memory_usage: u64,     // Used memory in MB
    pub memory_total: u64,     // Total memory in MB
    pub disk_read: u64,        // Bytes read per second
    pub disk_write: u64,       // Bytes written per second
    pub network_rx: u64,       // Bytes received per second
    pub network_tx: u64,       // Bytes transmitted per second
    pub uptime: u64,           // Uptime in seconds
    pub timestamp: DateTime<Utc>, // When these stats were collected
    pub guest_agent_connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    pub hypervisor: String,
    pub hypervisor_version: String,
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub memory_total: u64,     // Total host memory in MB
    pub memory_free: u64,      // Free host memory in MB
    pub storage_pools: Vec<StoragePool>,
    pub networks: Vec<Network>,
    pub active_vms: u32,
    pub inactive_vms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub state: String,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub mac_address: String,
    pub network_name: String,
    pub interface_type: String,
    pub model: String,
    pub link_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageDevice {
    pub device: String,       // vda, vdb, etc.
    pub source: String,       // file path or device
    pub format: String,       // qcow2, raw, etc.
    pub size: u64,           // Size in bytes
    pub bus: String,         // virtio, sata, etc.
    pub cache: String,       // cache mode
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoragePool {
    pub name: String,
    pub pool_type: String,   // dir, disk, netfs, etc.
    pub path: String,
    pub capacity: u64,       // Total capacity in bytes
    pub available: u64,      // Available space in bytes
    pub used: u64,          // Used space in bytes
    pub state: String,       // active, inactive
    pub autostart: bool,
    pub volumes: Vec<StorageVolume>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageVolume {
    pub name: String,
    pub format: String,
    pub capacity: u64,
    pub allocation: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub name: String,
    pub uuid: String,
    pub bridge_name: Option<String>,
    pub forward_mode: String, // nat, route, bridge, etc.
    pub state: String,        // active, inactive
    pub autostart: bool,
    pub ip_range: Option<String>,
    pub dhcp_enabled: bool,
    pub connected_vms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmTemplate {
    pub name: String,
    pub description: String,
    pub os_type: String,
    pub os_variant: String,
    pub default_memory: u64,
    pub default_vcpus: u32,
    pub default_disk_size: u64,
    pub recommended_settings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationTask {
    pub id: String,
    pub vm_id: String,
    pub source_host: String,
    pub target_host: String,
    pub state: MigrationState,
    pub progress: f64,       // Percentage
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MigrationState {
    Preparing,
    Migrating,
    Completed,
    Failed,
    Cancelled,
}
