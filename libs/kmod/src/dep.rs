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

fn parse_dep_line(line: &str) -> Option<(String, ModuleInfo)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let (module_path, deps_str) = line.split_once(':')?;
    let module_path = module_path.trim();
    let name = get_module_name(module_path)?;

    let deps: Vec<String> = deps_str
        .split_whitespace()
        .filter_map(get_module_name)
        .collect();

    Some((
        name,
        ModuleInfo {
            path: module_path.to_string(),
            deps,
        },
    ))
}

impl DepDb {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let modules = reader
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| parse_dep_line(&line))
            .collect();

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

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
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
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn get_module_name_zst() {
        // ARRANGE
        let path = "kernel/drivers/net/ethernet/intel/igc/igc.ko.zst";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, Some("igc".to_string()));
    }

    #[test]
    fn get_module_name_ko() {
        // ARRANGE
        let path = "kernel/drivers/virtio/virtio.ko";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, Some("virtio".to_string()));
    }

    #[test]
    fn get_module_name_unsupported_extension() {
        // ARRANGE
        let path_xz = "kernel/fs/ext4/ext4.ko.xz";
        let path_gz = "kernel/fs/ext4/ext4.ko.gz";

        // ACT & ASSERT
        assert_eq!(get_module_name(path_xz), None);
        assert_eq!(get_module_name(path_gz), None);
    }

    #[test]
    fn get_module_name_no_extension() {
        // ARRANGE
        let path = "kernel/drivers/some_module";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, None);
    }

    #[test]
    fn get_module_name_empty() {
        // ARRANGE
        let path = "";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, None);
    }

    #[test]
    fn depdb_load_empty_file() {
        // ARRANGE
        let file = NamedTempFile::new().expect("Failed to create temp file");

        // ACT
        let db = DepDb::load(file.path()).expect("load failed");

        // ASSERT
        assert!(db.is_empty());
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn depdb_load_single_module_no_deps() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/drivers/net/igc/igc.ko.zst:").expect("write failed");

        // ACT
        let db = DepDb::load(file.path()).expect("load failed");

        // ASSERT
        assert_eq!(db.len(), 1);
        assert_eq!(
            db.get_path("igc"),
            Some("kernel/drivers/net/igc/igc.ko.zst")
        );
    }

    #[test]
    fn depdb_load_module_with_deps() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(
            file,
            "kernel/drivers/net/igc/igc.ko.zst: kernel/drivers/ptp/ptp.ko.zst kernel/drivers/net/libphy.ko.zst"
        )
        .expect("write failed");
        writeln!(file, "kernel/drivers/ptp/ptp.ko.zst:").expect("write failed");
        writeln!(file, "kernel/drivers/net/libphy.ko.zst:").expect("write failed");

        // ACT
        let db = DepDb::load(file.path()).expect("load failed");

        // ASSERT
        assert_eq!(db.len(), 3);
        assert!(db.get_path("igc").is_some());
        assert!(db.get_path("ptp").is_some());
        assert!(db.get_path("libphy").is_some());
    }

    #[test]
    fn depdb_load_nonexistent_file() {
        // ACT
        let result = DepDb::load(Path::new("/nonexistent/modules.dep"));

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn depdb_load_malformed_lines() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/drivers/broken.ko.zst").expect("write failed");
        writeln!(file, "kernel/drivers/valid.ko.zst:").expect("write failed");

        // ACT
        let db = DepDb::load(file.path()).expect("load failed");

        // ASSERT
        assert_eq!(db.len(), 1);
        assert!(db.get_path("valid").is_some());
        assert!(db.get_path("broken").is_none());
    }

    #[test]
    fn depdb_get_path_exists() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/drivers/net/e1000e/e1000e.ko.zst:").expect("write failed");

        // ACT
        let db = DepDb::load(file.path()).expect("load failed");

        // ASSERT
        assert_eq!(
            db.get_path("e1000e"),
            Some("kernel/drivers/net/e1000e/e1000e.ko.zst")
        );
    }

    #[test]
    fn depdb_get_path_not_exists() {
        // ARRANGE
        let file = NamedTempFile::new().expect("Failed to create temp file");
        let db = DepDb::load(file.path()).expect("load failed");

        // ACT & ASSERT
        assert_eq!(db.get_path("nonexistent"), None);
    }

    #[test]
    fn resolve_load_order_no_deps() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/a.ko.zst:").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order, vec!["kernel/a.ko.zst"]);
    }

    #[test]
    fn resolve_load_order_single_dep() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/a.ko.zst: kernel/b.ko.zst").expect("write failed");
        writeln!(file, "kernel/b.ko.zst:").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order, vec!["kernel/b.ko.zst", "kernel/a.ko.zst"]);
    }

    #[test]
    fn resolve_load_order_chain() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/a.ko.zst: kernel/b.ko.zst").expect("write failed");
        writeln!(file, "kernel/b.ko.zst: kernel/c.ko.zst").expect("write failed");
        writeln!(file, "kernel/c.ko.zst:").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(
            order,
            vec!["kernel/c.ko.zst", "kernel/b.ko.zst", "kernel/a.ko.zst"]
        );
    }

    #[test]
    fn resolve_load_order_diamond() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/a.ko.zst: kernel/b.ko.zst kernel/c.ko.zst").expect("write failed");
        writeln!(file, "kernel/b.ko.zst: kernel/d.ko.zst").expect("write failed");
        writeln!(file, "kernel/c.ko.zst: kernel/d.ko.zst").expect("write failed");
        writeln!(file, "kernel/d.ko.zst:").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order.len(), 4);
        assert!(order.iter().filter(|x| x.contains("/d.")).count() == 1);
        let d_pos = order
            .iter()
            .position(|x| x.contains("/d."))
            .expect("d not found");
        let b_pos = order
            .iter()
            .position(|x| x.contains("/b."))
            .expect("b not found");
        let c_pos = order
            .iter()
            .position(|x| x.contains("/c."))
            .expect("c not found");
        let a_pos = order
            .iter()
            .position(|x| x.contains("/a."))
            .expect("a not found");
        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
        assert!(b_pos < a_pos);
        assert!(c_pos < a_pos);
    }

    #[test]
    fn resolve_load_order_circular_deps() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/a.ko.zst: kernel/b.ko.zst").expect("write failed");
        writeln!(file, "kernel/b.ko.zst: kernel/a.ko.zst").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order.len(), 2);
        assert!(order.iter().any(|x| x.contains("/a.")));
        assert!(order.iter().any(|x| x.contains("/b.")));
    }

    #[test]
    fn resolve_load_order_missing_dep() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "kernel/a.ko.zst: kernel/b.ko.zst").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order, vec!["kernel/a.ko.zst"]);
    }

    #[test]
    fn resolve_load_order_unknown_module() {
        // ARRANGE
        let file = NamedTempFile::new().expect("Failed to create temp file");
        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db
            .resolve_load_order("nonexistent")
            .expect("resolve failed");

        // ASSERT
        assert!(order.is_empty());
    }

    #[test]
    fn resolve_load_order_multiple_deps() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(
            file,
            "kernel/a.ko.zst: kernel/b.ko.zst kernel/c.ko.zst kernel/d.ko.zst"
        )
        .expect("write failed");
        writeln!(file, "kernel/b.ko.zst:").expect("write failed");
        writeln!(file, "kernel/c.ko.zst:").expect("write failed");
        writeln!(file, "kernel/d.ko.zst:").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order.len(), 4);
        let a_pos = order
            .iter()
            .position(|x| x.contains("/a."))
            .expect("a not found");
        assert_eq!(a_pos, 3);
    }

    #[test]
    fn real_modules_dep_format() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(
            file,
            "kernel/drivers/net/ethernet/intel/igc/igc.ko.zst: kernel/drivers/ptp/ptp.ko.zst"
        )
        .expect("write failed");
        writeln!(
            file,
            "kernel/drivers/ptp/ptp.ko.zst: kernel/drivers/pps/pps_core.ko.zst"
        )
        .expect("write failed");
        writeln!(file, "kernel/drivers/pps/pps_core.ko.zst:").expect("write failed");
        writeln!(file, "kernel/drivers/virtio/virtio.ko.zst:").expect("write failed");
        writeln!(
            file,
            "kernel/drivers/virtio/virtio_net.ko.zst: kernel/drivers/virtio/virtio.ko.zst kernel/drivers/net/net_failover.ko.zst"
        )
        .expect("write failed");
        writeln!(file, "kernel/drivers/net/net_failover.ko.zst:").expect("write failed");

        let db = DepDb::load(file.path()).expect("load failed");

        // ACT
        let igc_order = db.resolve_load_order("igc").expect("resolve failed");
        let vnet_order = db.resolve_load_order("virtio_net").expect("resolve failed");

        // ASSERT
        assert_eq!(db.len(), 6);
        assert_eq!(igc_order.len(), 3);
        assert!(igc_order[0].contains("pps_core"));
        assert!(igc_order[1].contains("ptp"));
        assert!(igc_order[2].contains("igc"));

        assert_eq!(vnet_order.len(), 3);
        let vnet_pos = vnet_order
            .iter()
            .position(|x| x.contains("virtio_net"))
            .expect("virtio_net not found");
        assert_eq!(vnet_pos, 2);
    }
}
