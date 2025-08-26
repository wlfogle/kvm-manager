// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod vm_manager;
mod storage;
mod network;
mod monitoring;
mod system_monitor;
mod types;
mod errors;
mod xml_parser;

use tracing::{info, error};
use std::sync::Arc;
use tokio::sync::RwLock;

use vm_manager::VmManager;
use types::*;

type AppState = Arc<RwLock<VmManager>>;

#[tauri::command]
async fn get_vms(state: tauri::State<'_, AppState>) -> Result<Vec<VirtualMachine>, String> {
    let manager = state.read().await;
    manager.list_vms().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_vm(
    state: tauri::State<'_, AppState>,
    config: VmConfig,
) -> Result<String, String> {
    let mut manager = state.write().await;
    manager.create_vm(config).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_vm(
    state: tauri::State<'_, AppState>,
    vm_id: String,
) -> Result<(), String> {
    let manager = state.read().await;
    manager.start_vm(&vm_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_vm(
    state: tauri::State<'_, AppState>,
    vm_id: String,
) -> Result<(), String> {
    let manager = state.read().await;
    manager.stop_vm(&vm_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_vm(
    state: tauri::State<'_, AppState>,
    vm_id: String,
) -> Result<(), String> {
    let mut manager = state.write().await;
    manager.delete_vm(&vm_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_vm_stats(
    state: tauri::State<'_, AppState>,
    vm_id: String,
) -> Result<VmStats, String> {
    let manager = state.read().await;
    manager.get_vm_stats(&vm_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_host_info(state: tauri::State<'_, AppState>) -> Result<HostInfo, String> {
    let manager = state.read().await;
    manager.get_host_info().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_snapshot(
    state: tauri::State<'_, AppState>,
    vm_id: String,
    snapshot_name: String,
) -> Result<(), String> {
    let manager = state.read().await;
    manager.create_snapshot(&vm_id, &snapshot_name).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn restore_snapshot(
    state: tauri::State<'_, AppState>,
    vm_id: String,
    snapshot_name: String,
) -> Result<(), String> {
    let manager = state.read().await;
    manager.restore_snapshot(&vm_id, &snapshot_name).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_storage_pools(state: tauri::State<'_, AppState>) -> Result<Vec<StoragePool>, String> {
    let manager = state.read().await;
    manager.get_storage_pools().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_networks(state: tauri::State<'_, AppState>) -> Result<Vec<Network>, String> {
    let manager = state.read().await;
    manager.get_networks().await.map_err(|e| e.to_string())
}

// Enhanced Proxmox-specific commands
#[tauri::command]
async fn create_proxmox_vm(
    state: tauri::State<'_, AppState>,
    name: String,
    proxmox_path: String,
    memory_gb: u32,
    vcpus: u32,
) -> Result<String, String> {
    let mut manager = state.write().await;
    manager.create_proxmox_vm(name, proxmox_path, memory_gb, vcpus).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn import_vm_from_xml(
    state: tauri::State<'_, AppState>,
    xml_path: String,
) -> Result<String, String> {
    let mut manager = state.write().await;
    manager.import_vm_from_xml(&xml_path).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_vm_from_qcow2(
    state: tauri::State<'_, AppState>,
    qcow2_path: String,
    vm_name: String,
    memory_mb: u64,
    vcpus: u32,
    passthrough_device: Option<String>,
) -> Result<String, String> {
    let mut manager = state.write().await;
    manager.create_vm_from_qcow2(&qcow2_path, &vm_name, memory_mb, vcpus, passthrough_device.as_deref()).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn refresh_vms(state: tauri::State<'_, AppState>) -> Result<Vec<VirtualMachine>, String> {
    let mut manager = state.write().await;
    manager.refresh_vm_list().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_qcow2_info(path: String) -> Result<QcowInfo, String> {
    use std::process::Command;
    
    info!("Getting QCOW2 info for: {}", path);
    
    // Check if file exists
    if !std::path::Path::new(&path).exists() {
        return Err(format!("File not found: {}", path));
    }
    
    // Use qemu-img info to get details
    let output = Command::new("qemu-img")
        .args(["info", "--output=json", &path])
        .output()
        .map_err(|e| format!("Failed to run qemu-img: {}", e))?;
    
    if !output.status.success() {
        return Err(format!("qemu-img failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    
    let info_json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse qemu-img output: {}", e))?;
    
    let filename = std::path::Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    
    let size_gb = info_json["actual-size"]
        .as_u64()
        .unwrap_or(0) as f64 / 1024.0 / 1024.0 / 1024.0;
    
    let virtual_size_gb = info_json["virtual-size"]
        .as_u64()
        .unwrap_or(0) as f64 / 1024.0 / 1024.0 / 1024.0;
    
    let format = info_json["format"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    
    let cluster_size = info_json["cluster-size"].as_u64();
    
    let backing_file = info_json["backing-filename"]
        .as_str()
        .map(|s| s.to_string());
    
    Ok(QcowInfo {
        path: path.clone(),
        filename,
        size_gb,
        format,
        virtual_size_gb,
        cluster_size,
        backing_file,
    })
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct QcowInfo {
    pub path: String,
    pub filename: String,
    pub size_gb: f64,
    pub format: String,
    pub virtual_size_gb: f64,
    pub cluster_size: Option<u64>,
    pub backing_file: Option<String>,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting KVM Manager application");

    // Initialize VM Manager
    let vm_manager = match VmManager::new().await {
        Ok(manager) => Arc::new(RwLock::new(manager)),
        Err(e) => {
            error!("Failed to initialize VM Manager: {}", e);
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(vm_manager)
        .invoke_handler(tauri::generate_handler![
            get_vms,
            create_vm,
            start_vm,
            stop_vm,
            delete_vm,
            get_vm_stats,
            get_host_info,
            create_snapshot,
            restore_snapshot,
            get_storage_pools,
            get_networks,
            create_proxmox_vm,
            import_vm_from_xml,
            create_vm_from_qcow2,
            refresh_vms,
            get_qcow2_info,
            system_monitor::get_system_statistics,
            system_monitor::get_proxmox_info,
            system_monitor::get_system_history,
            system_monitor::start_system_monitoring
        ])
        .setup(|app| {
            info!("Application setup complete");
            
            // Start system monitoring
            tokio::spawn(async {
                if let Err(e) = system_monitor::start_system_monitoring().await {
                    error!("Failed to start system monitoring: {}", e);
                }
            });
            
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
