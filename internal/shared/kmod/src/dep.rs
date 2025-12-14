use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug)]
pub struct DepDb {
    modules: HashMap<String, ModuleInfo>,
}

#[derive(Debug)]
struct ModuleInfo {
    path: String,
    deps: Vec<String>,
}

impl DepDb {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut modules = HashMap::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            let Some((module_path, deps_str)) = line.split_once(':') else {
                continue;
            };
            let module_path = module_path.trim();
            let Some(name) = get_module_name(module_path) else {
                continue;
            };

            let deps: Vec<String> = deps_str
                .split_whitespace()
                .filter_map(get_module_name)
                .collect();

            modules.insert(
                name,
                ModuleInfo {
                    path: module_path.to_string(),
                    deps,
                },
            );
        }

        Ok(Self { modules })
    }

    pub fn get_path(&self, module_name: &str) -> Option<&str> {
        self.modules.get(module_name).map(|m| m.path.as_str())
    }

    pub fn resolve_load_order(&self, module_name: &str) -> Option<Vec<String>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        self.resolve_deps_recursive(module_name, &mut result, &mut visited);
        Some(result)
    }

    fn resolve_deps_recursive(
        &self,
        module_name: &str,
        result: &mut Vec<String>,
        visited: &mut HashSet<String>,
    ) {
        if visited.contains(module_name) {
            return;
        }
        visited.insert(module_name.to_string());

        let Some(info) = self.modules.get(module_name) else {
            return;
        };

        for dep in &info.deps {
            self.resolve_deps_recursive(dep, result, visited);
        }

        result.push(info.path.clone());
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }
}

pub(crate) fn get_module_name(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?;
    let name = filename
        .strip_suffix(".ko.zst")
        .or_else(|| filename.strip_suffix(".ko"))?;
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_module_name() {
        assert_eq!(
            get_module_name("kernel/drivers/net/ethernet/intel/igc/igc.ko.zst"),
            Some("igc".to_string())
        );
        assert_eq!(
            get_module_name("kernel/drivers/virtio/virtio.ko"),
            Some("virtio".to_string())
        );
        assert_eq!(get_module_name("kernel/fs/ext4/ext4.ko.xz"), None);
    }
}
