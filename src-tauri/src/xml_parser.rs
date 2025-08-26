use std::collections::HashMap;
use tracing::{debug, warn};
use crate::errors::{KvmError, Result};
use crate::types::*;

pub struct XmlParser;

impl XmlParser {
    /// Parse VM properties from libvirt XML
    pub fn parse_vm_from_xml(xml: &str) -> Result<VmXmlInfo> {
        debug!("Parsing VM XML: {} chars", xml.len());
        
        let mut vm_info = VmXmlInfo::default();
        
        // Parse basic info
        vm_info.name = Self::extract_between_tags(xml, "name")
            .unwrap_or_else(|| "unknown".to_string());
        
        vm_info.uuid = Self::extract_between_tags(xml, "uuid")
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        
        // Parse memory (in KiB, convert to MB)
        if let Some(memory_str) = Self::extract_between_tags(xml, "memory") {
            if let Ok(memory_kib) = memory_str.parse::<u64>() {
                vm_info.memory_mb = memory_kib / 1024;
            }
        }
        
        // Parse vCPUs
        if let Some(vcpus_str) = Self::extract_between_tags(xml, "vcpu") {
            if let Ok(vcpus) = vcpus_str.parse::<u32>() {
                vm_info.vcpus = vcpus;
            }
        }
        
        // Parse OS info
        let (os_type, os_variant) = Self::parse_os_info(xml);
        vm_info.os_type = os_type;
        vm_info.os_variant = os_variant;
        
        // Parse storage devices
        vm_info.storage_devices = Self::parse_storage_devices(xml);
        vm_info.disk_size_gb = vm_info.storage_devices.iter()
            .map(|d| d.size_gb)
            .sum();
        
        // Parse network interfaces
        vm_info.network_interfaces = Self::parse_network_interfaces(xml);
        
        // Parse graphics ports
        vm_info.vnc_port = Self::parse_vnc_port(xml);
        vm_info.spice_port = Self::parse_spice_port(xml);
        
        // Parse description from metadata
        vm_info.description = Self::extract_description(xml);
        
        debug!("Parsed VM info: name={}, memory={}MB, vcpus={}, disks={}", 
               vm_info.name, vm_info.memory_mb, vm_info.vcpus, vm_info.storage_devices.len());
        
        Ok(vm_info)
    }
    
    fn extract_between_tags(xml: &str, tag: &str) -> Option<String> {
        let start_tag = format!("<{}>", tag);
        let end_tag = format!("</{}>", tag);
        
        if let Some(start_pos) = xml.find(&start_tag) {
            let content_start = start_pos + start_tag.len();
            if let Some(end_pos) = xml[content_start..].find(&end_tag) {
                return Some(xml[content_start..content_start + end_pos].trim().to_string());
            }
        }
        None
    }
    
    fn parse_os_info(xml: &str) -> (String, Option<String>) {
        // Look for libosinfo metadata first
        if xml.contains("libosinfo:os") {
            if let Some(os_id) = Self::extract_attribute_value(xml, "libosinfo:os", "id") {
                if os_id.contains("debian") {
                    return ("linux".to_string(), Some("debian".to_string()));
                } else if os_id.contains("ubuntu") {
                    return ("linux".to_string(), Some("ubuntu".to_string()));
                } else if os_id.contains("fedora") {
                    return ("linux".to_string(), Some("fedora".to_string()));
                } else if os_id.contains("rhel") || os_id.contains("centos") {
                    return ("linux".to_string(), Some("rhel".to_string()));
                } else if os_id.contains("windows") {
                    return ("windows".to_string(), Some("win10".to_string()));
                }
            }
        }
        
        // Fallback to content-based detection
        let xml_lower = xml.to_lowercase();
        if xml_lower.contains("windows") || xml_lower.contains("win10") || xml_lower.contains("win11") {
            ("windows".to_string(), Some("win10".to_string()))
        } else if xml_lower.contains("freebsd") || xml_lower.contains("openbsd") {
            ("bsd".to_string(), Some("generic".to_string()))
        } else {
            ("linux".to_string(), Some("generic".to_string()))
        }
    }
    
    fn parse_storage_devices(xml: &str) -> Vec<StorageDevice> {
        let mut devices = Vec::new();
        
        // Find all disk elements
        let disk_pattern = r#"<disk\s+[^>]*>"#;
        if let Ok(regex) = regex::Regex::new(disk_pattern) {
            for disk_match in regex.find_iter(xml) {
                let disk_start = disk_match.start();
                
                // Find the closing </disk> tag
                if let Some(disk_end) = xml[disk_start..].find("</disk>") {
                    let disk_xml = &xml[disk_start..disk_start + disk_end + 7];
                    
                    if let Some(device) = Self::parse_single_disk(disk_xml) {
                        devices.push(device);
                    }
                }
            }
        }
        
        devices
    }
    
    fn parse_single_disk(disk_xml: &str) -> Option<StorageDevice> {
        let device_type = Self::extract_attribute_value(disk_xml, "disk", "device")?;
        
        if device_type != "disk" {
            return None; // Skip CD-ROM, floppy, etc.
        }
        
        let driver_type = Self::extract_attribute_value(disk_xml, "driver", "type")
            .unwrap_or_else(|| "raw".to_string());
        
        let target_dev = Self::extract_attribute_value(disk_xml, "target", "dev")
            .unwrap_or_else(|| "vda".to_string());
        
        let target_bus = Self::extract_attribute_value(disk_xml, "target", "bus")
            .unwrap_or_else(|| "virtio".to_string());
        
        let source_file = Self::extract_attribute_value(disk_xml, "source", "file");
        
        // Try to get disk size from file if available
        let size_gb = if let Some(file_path) = &source_file {
            Self::get_disk_size_from_file(file_path).unwrap_or(0.0)
        } else {
            0.0
        };
        
        Some(StorageDevice {
            device: target_dev,
            type_: driver_type,
            size_gb,
            path: source_file,
            bus: target_bus,
            cache: Self::extract_attribute_value(disk_xml, "driver", "cache"),
        })
    }
    
    fn get_disk_size_from_file(file_path: &str) -> Option<f64> {
        use std::process::Command;
        
        let output = Command::new("qemu-img")
            .args(["info", "--output=json", file_path])
            .output()
            .ok()?;
        
        if output.status.success() {
            let info_json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
            let size_bytes = info_json["virtual-size"].as_u64()?;
            Some(size_bytes as f64 / 1024.0 / 1024.0 / 1024.0)
        } else {
            None
        }
    }
    
    fn parse_network_interfaces(xml: &str) -> Vec<NetworkInterface> {
        let mut interfaces = Vec::new();
        
        // Find all interface elements
        let interface_pattern = r#"<interface\s+[^>]*>"#;
        if let Ok(regex) = regex::Regex::new(interface_pattern) {
            for interface_match in regex.find_iter(xml) {
                let interface_start = interface_match.start();
                
                // Find the closing </interface> tag
                if let Some(interface_end) = xml[interface_start..].find("</interface>") {
                    let interface_xml = &xml[interface_start..interface_start + interface_end + 12];
                    
                    if let Some(interface) = Self::parse_single_interface(interface_xml) {
                        interfaces.push(interface);
                    }
                }
            }
        }
        
        interfaces
    }
    
    fn parse_single_interface(interface_xml: &str) -> Option<NetworkInterface> {
        let interface_type = Self::extract_attribute_value(interface_xml, "interface", "type")?;
        
        let mac_address = Self::extract_attribute_value(interface_xml, "mac", "address");
        let network_source = Self::extract_attribute_value(interface_xml, "source", "network");
        let bridge_source = Self::extract_attribute_value(interface_xml, "source", "bridge");
        let model_type = Self::extract_attribute_value(interface_xml, "model", "type")
            .unwrap_or_else(|| "rtl8139".to_string());
        
        Some(NetworkInterface {
            type_: interface_type,
            mac_address,
            source: network_source.or(bridge_source).unwrap_or_else(|| "default".to_string()),
            model: model_type,
            connected: true, // Assume connected if defined
        })
    }
    
    fn parse_vnc_port(xml: &str) -> Option<u16> {
        if xml.contains("type='vnc'") {
            Self::extract_attribute_value(xml, "graphics", "port")
                .and_then(|port_str| {
                    if port_str == "-1" {
                        None // Auto-allocated port
                    } else {
                        port_str.parse().ok()
                    }
                })
        } else {
            None
        }
    }
    
    fn parse_spice_port(xml: &str) -> Option<u16> {
        if xml.contains("type='spice'") {
            Self::extract_attribute_value(xml, "graphics", "port")
                .and_then(|port_str| {
                    if port_str == "-1" {
                        None // Auto-allocated port
                    } else {
                        port_str.parse().ok()
                    }
                })
        } else {
            None
        }
    }
    
    fn extract_description(xml: &str) -> Option<String> {
        // Look for description in metadata
        Self::extract_between_tags(xml, "description")
            .or_else(|| Self::extract_between_tags(xml, "title"))
    }
    
    fn extract_attribute_value(xml: &str, element: &str, attribute: &str) -> Option<String> {
        let pattern = format!(r#"<{}\s+[^>]*{}=['""]([^'"]*)['""]"#, element, attribute);
        if let Ok(regex) = regex::Regex::new(&pattern) {
            if let Some(captures) = regex.captures(xml) {
                return captures.get(1).map(|m| m.as_str().to_string());
            }
        }
        
        // Try alternative pattern with different quote order
        let pattern2 = format!(r#"<{}\s+[^>]*{}=([^>\s]*)"#, element, attribute);
        if let Ok(regex) = regex::Regex::new(&pattern2) {
            if let Some(captures) = regex.captures(xml) {
                return captures.get(1).map(|m| m.as_str().trim_matches('"').trim_matches('\'').to_string());
            }
        }
        
        None
    }
}

#[derive(Debug, Default, Clone)]
pub struct VmXmlInfo {
    pub name: String,
    pub uuid: String,
    pub memory_mb: u64,
    pub vcpus: u32,
    pub os_type: String,
    pub os_variant: Option<String>,
    pub disk_size_gb: f64,
    pub storage_devices: Vec<StorageDevice>,
    pub network_interfaces: Vec<NetworkInterface>,
    pub vnc_port: Option<u16>,
    pub spice_port: Option<u16>,
    pub description: Option<String>,
}
