use tracing::{info, error};
use virt::{connect::Connect, storage_pool::StoragePool as LibvirtPool, storage_vol::StorageVol};
use crate::errors::{KvmError, Result};
use crate::types::*;

pub struct StorageManager {
    connection: Connect,
}

impl StorageManager {
    pub fn new(connection: Connect) -> Self {
        Self { connection }
    }

    pub async fn create_volume(&self, pool_name: &str, volume_config: &VolumeConfig) -> Result<String> {
        info!("Creating volume {} in pool {}", volume_config.name, pool_name);
        
        // Get the storage pool
        let pool = LibvirtPool::lookup_by_name(&self.connection, pool_name)
            .map_err(|e| {
                error!("Failed to find storage pool {}: {}", pool_name, e);
                KvmError::StorageOperationFailed(format!("Storage pool not found: {}", e))
            })?;
        
        // Generate volume XML
        let volume_xml = self.generate_volume_xml(volume_config)?;
        
        // Create the volume
        let volume = StorageVol::create_xml(&pool, &volume_xml, 0)
            .map_err(|e| {
                error!("Failed to create volume {}: {}", volume_config.name, e);
                KvmError::StorageOperationFailed(format!("Failed to create volume: {}", e))
            })?;
        
        let vol_name = volume.get_name().map_err(KvmError::LibvirtConnection)?;
        info!("Successfully created volume: {}", vol_name);
        Ok(vol_name)
    }

    pub async fn resize_volume(&self, pool_name: &str, volume_name: &str, new_size: u64) -> Result<()> {
        info!("Resizing volume {} in pool {} to {} bytes", volume_name, pool_name, new_size);
        
        // Get the storage pool
        let pool = LibvirtPool::lookup_by_name(&self.connection, pool_name)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Storage pool not found: {}", e)))?;
        
        // Get the volume
        let volume = StorageVol::lookup_by_name(&pool, volume_name)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Volume not found: {}", e)))?;
        
        // Resize the volume
        volume.resize(new_size, 0)
            .map_err(|e| {
                error!("Failed to resize volume {}: {}", volume_name, e);
                KvmError::StorageOperationFailed(format!("Failed to resize volume: {}", e))
            })?;
        
        info!("Successfully resized volume {} to {} bytes", volume_name, new_size);
        Ok(())
    }

    pub async fn clone_volume(&self, source_pool: &str, source_volume: &str, target_pool: &str, target_volume: &str) -> Result<()> {
        info!("Cloning volume {}/{} to {}/{}", source_pool, source_volume, target_pool, target_volume);
        
        // Get source pool and volume
        let src_pool = LibvirtPool::lookup_by_name(&self.connection, source_pool)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Source pool not found: {}", e)))?;
        
        let src_volume = StorageVol::lookup_by_name(&src_pool, source_volume)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Source volume not found: {}", e)))?;
        
        // Get target pool
        let target_pool_obj = LibvirtPool::lookup_by_name(&self.connection, target_pool)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Target pool not found: {}", e)))?;
        
        // Get source volume info for cloning
        let src_info = src_volume.get_info()
            .map_err(|e| KvmError::StorageOperationFailed(format!("Failed to get source volume info: {}", e)))?;
        
        // Create clone configuration
        let clone_config = VolumeConfig {
            name: target_volume.to_string(),
            format: "qcow2".to_string(), // Default to qcow2 for clones
            capacity: src_info.capacity,
            allocation: Some(0), // Thin provisioning
        };
        
        // Generate clone XML with backing file reference
        let clone_xml = self.generate_clone_volume_xml(&clone_config, &src_volume)?;
        
        // Create the clone
        StorageVol::create_xml(&target_pool_obj, &clone_xml, 0)
            .map_err(|e| {
                error!("Failed to create volume clone {}: {}", target_volume, e);
                KvmError::StorageOperationFailed(format!("Failed to create volume clone: {}", e))
            })?;
        
        info!("Successfully cloned volume {}/{} to {}/{}", source_pool, source_volume, target_pool, target_volume);
        Ok(())
    }
    
    pub async fn delete_volume(&self, pool_name: &str, volume_name: &str) -> Result<()> {
        info!("Deleting volume {} from pool {}", volume_name, pool_name);
        
        let pool = LibvirtPool::lookup_by_name(&self.connection, pool_name)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Storage pool not found: {}", e)))?;
        
        let volume = StorageVol::lookup_by_name(&pool, volume_name)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Volume not found: {}", e)))?;
        
        volume.delete(0)
            .map_err(|e| {
                error!("Failed to delete volume {}: {}", volume_name, e);
                KvmError::StorageOperationFailed(format!("Failed to delete volume: {}", e))
            })?;
        
        info!("Successfully deleted volume: {}", volume_name);
        Ok(())
    }
    
    pub async fn get_volume_info(&self, pool_name: &str, volume_name: &str) -> Result<VolumeInfo> {
        let pool = LibvirtPool::lookup_by_name(&self.connection, pool_name)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Storage pool not found: {}", e)))?;
        
        let volume = StorageVol::lookup_by_name(&pool, volume_name)
            .map_err(|e| KvmError::StorageOperationFailed(format!("Volume not found: {}", e)))?;
        
        let info = volume.get_info()
            .map_err(|e| KvmError::StorageOperationFailed(format!("Failed to get volume info: {}", e)))?;
        
        let path = volume.get_path()
            .map_err(|e| KvmError::StorageOperationFailed(format!("Failed to get volume path: {}", e)))?;
        
        Ok(VolumeInfo {
            name: volume_name.to_string(),
            path,
            format: "unknown".to_string(), // TODO: Parse from XML
            capacity: info.capacity,
            allocation: info.allocation,
        })
    }
    
    fn generate_volume_xml(&self, config: &VolumeConfig) -> Result<String> {
        let allocation = config.allocation.unwrap_or(config.capacity);
        
        let xml = format!(
            r#"<volume type='file'>
  <name>{}</name>
  <key>{}</key>
  <source>
  </source>
  <capacity unit='bytes'>{}</capacity>
  <allocation unit='bytes'>{}</allocation>
  <target>
    <format type='{}'/>
  </target>
</volume>"#,
            config.name,
            config.name, // Use name as key for simplicity
            config.capacity,
            allocation,
            config.format
        );
        
        Ok(xml)
    }
    
    fn generate_clone_volume_xml(&self, config: &VolumeConfig, source_volume: &StorageVol) -> Result<String> {
        let source_path = source_volume.get_path()
            .map_err(|e| KvmError::StorageOperationFailed(format!("Failed to get source path: {}", e)))?;
        
        let xml = format!(
            r#"<volume type='file'>
  <name>{}</name>
  <key>{}</key>
  <source>
  </source>
  <capacity unit='bytes'>{}</capacity>
  <allocation unit='bytes'>{}</allocation>
  <target>
    <format type='{}'/>
  </target>
  <backingStore>
    <path>{}</path>
    <format type='qcow2'/>
  </backingStore>
</volume>"#,
            config.name,
            config.name,
            config.capacity,
            config.allocation.unwrap_or(0),
            config.format,
            source_path
        );
        
        Ok(xml)
    }
}
