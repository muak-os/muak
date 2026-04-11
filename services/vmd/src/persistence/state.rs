use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::VMS_DIR;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmPersisted {
    pub name: String,
    pub cpus: u32,
    pub memory_mb: u64,
    pub kernel: String,
    pub initrd: String,
    pub cmdline: String,
    pub disks: Vec<DiskConfigPersisted>,
    pub hypervisor: i32,
    pub root_disk_size_mb: u64,
    pub state: i32,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub tap_device: Option<String>,
    pub mac_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConfigPersisted {
    pub path: String,
    pub readonly: bool,
}

pub fn load_vms() -> Result<HashMap<String, VmPersisted>> {
    let dir = Path::new(VMS_DIR);
    if !dir.exists() {
        return Ok(HashMap::new());
    }

    let mut vms = HashMap::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        match load_vm_from_path(&path) {
            Ok(vm) => {
                vms.insert(stem.to_string(), vm);
            }
            Err(e) => eprintln!("Failed to load VM state {}: {}", path.display(), e),
        }
    }

    Ok(vms)
}

fn load_vm_from_path(path: &Path) -> Result<VmPersisted> {
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

pub fn save_vm(vm_id: &str, vm: &VmPersisted) -> Result<()> {
    let path = Path::new(VMS_DIR).join(format!("{}.json", vm_id));
    let json = serde_json::to_string_pretty(vm)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn delete_vm(vm_id: &str) -> Result<()> {
    let path = Path::new(VMS_DIR).join(format!("{}.json", vm_id));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
