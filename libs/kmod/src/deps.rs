//! Module dependency database support.

extern crate alloc;

use alloc::borrow::Cow;
use std::fs;
use std::path::Path;

use crate::text::{self, Span};

const KO: &str = ".ko";
const KO_ZST: &str = ".ko.zst";

/// Parsed `modules.dep` database.
#[derive(Debug)]
pub struct DepDb {
    text: String,
    modules: Vec<Entry>,
    dep_names: Vec<Span>,
}

#[derive(Debug)]
struct Entry {
    name: Span,
    path: Span,
    deps: Span,
}

impl DepDb {
    /// Loads a `modules.dep` database from disk.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` cannot be opened or read.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut db = Self {
            text,
            modules: Vec::new(),
            dep_names: Vec::new(),
        };
        db.index();

        Ok(db)
    }

    /// Returns the relative module path for `module_name`.
    #[must_use]
    pub fn get_path(&self, module_name: &str) -> Option<&str> {
        let entry = self.modules.get(self.find(module_name)?)?;

        Some(text::slice(&self.text, entry.path))
    }

    /// Resolves the dependency-first module load order for `module_name`.
    #[must_use]
    pub fn resolve_load_order(&self, module_name: &str) -> Option<Vec<&str>> {
        let start = self.find(module_name)?;
        let mut order = Vec::new();
        let mut visited = vec![false; self.modules.len()];
        self.visit(start, &mut order, &mut visited);

        Some(order)
    }

    /// Returns the number of indexed modules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Returns whether the dependency database is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    fn index(&mut self) {
        let text = core::mem::take(&mut self.text);
        let mut modules = Vec::new();
        let mut dep_names = Vec::new();
        text::for_each_line(&text, |line, offset| {
            push_module(line, offset, &mut modules, &mut dep_names);
        });
        modules.sort_by(|left, right| {
            key(text::slice(&text, left.name)).cmp(key(text::slice(&text, right.name)))
        });
        self.text = text;
        self.modules = modules;
        self.dep_names = dep_names;
    }

    fn find(&self, module_name: &str) -> Option<usize> {
        self.modules
            .binary_search_by(|entry| {
                key(text::slice(&self.text, entry.name)).cmp(key(module_name))
            })
            .ok()
    }

    fn visit<'a>(&'a self, index: usize, order: &mut Vec<&'a str>, visited: &mut [bool]) {
        let Some(slot) = visited.get_mut(index) else {
            return;
        };
        if *slot {
            return;
        }
        *slot = true;

        let Some(entry) = self.modules.get(index) else {
            return;
        };
        for dep in self
            .dep_names
            .iter()
            .skip(entry.deps.start)
            .take(entry.deps.len)
            .filter_map(|dep| self.find(text::slice(&self.text, *dep)))
        {
            self.visit(dep, order, visited);
        }
        order.push(text::slice(&self.text, entry.path));
    }
}

/// Extracts the canonical module name from a relative module path.
pub(crate) fn get_module_name(path: &str) -> Option<Cow<'_, str>> {
    let filename = path.rsplit('/').next()?;
    let name = stripped_name(filename)?;
    if name.contains('-') {
        Some(Cow::Owned(name.replace('-', "_")))
    } else {
        Some(Cow::Borrowed(name))
    }
}

fn push_module(line: &str, offset: usize, modules: &mut Vec<Entry>, dep_names: &mut Vec<Span>) {
    let lead = line.len().saturating_sub(line.trim_start().len());
    let line = line.trim();
    let Some((raw_path, deps)) = line.split_once(':') else {
        return;
    };

    let path = raw_path.trim();
    let path_base = offset.saturating_add(lead);
    let Some(name) = name_span(path, path_base) else {
        return;
    };

    let deps_start = dep_names.len();
    let mut dep_base = path_base.saturating_add(raw_path.len()).saturating_add(1);
    for dep in deps.split(' ') {
        if let Some(dep_name) = name_span(dep, dep_base) {
            dep_names.push(dep_name);
        }
        dep_base = dep_base.saturating_add(dep.len()).saturating_add(1);
    }

    modules.push(Entry {
        name,
        path: text::span_at(path_base, raw_path),
        deps: Span {
            start: deps_start,
            len: dep_names.len().saturating_sub(deps_start),
        },
    });
}

fn name_span(path: &str, base: usize) -> Option<Span> {
    let filename = path.rsplit('/').next()?;
    let name = stripped_name(filename)?;
    let start = base.saturating_add(path.len().saturating_sub(filename.len()));
    Some(Span {
        start,
        len: name.len(),
    })
}

fn stripped_name(filename: &str) -> Option<&str> {
    filename
        .strip_suffix(KO_ZST)
        .or_else(|| filename.strip_suffix(KO))
}

fn key(name: &str) -> impl Iterator<Item = u8> {
    name.bytes()
        .map(|byte| if byte == b'-' { b'_' } else { byte })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    fn write_db(contents: &str) -> (NamedTempFile, DepDb) {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "{contents}").expect("write failed");
        let db = DepDb::load(file.path()).expect("load failed");
        (file, db)
    }

    #[test]
    fn get_module_name_zst() {
        // ARRANGE
        let path = "kernel/drivers/net/ethernet/intel/igc/igc.ko.zst";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, Some(Cow::Borrowed("igc")));
    }

    #[test]
    fn get_module_name_normalizes_dashes_to_underscores() {
        // ARRANGE
        let path = "kernel/drivers/i2c/busses/i2c-i801.ko.zst";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, Some(Cow::Owned("i2c_i801".to_owned())));
    }

    #[test]
    fn get_module_name_plain_ko_normalizes_dashes() {
        // ARRANGE
        let path = "kernel/drivers/vfio/pci/vfio-pci.ko";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, Some(Cow::Owned("vfio_pci".to_owned())));
    }

    #[test]
    fn get_module_name_ko() {
        // ARRANGE
        let path = "kernel/drivers/virtio/virtio.ko";

        // ACT
        let result = get_module_name(path);

        // ASSERT
        assert_eq!(result, Some(Cow::Borrowed("virtio")));
    }

    #[test]
    fn get_module_name_unsupported_extension() {
        // ACT / ASSERT
        assert_eq!(get_module_name("kernel/fs/ext4/ext4.ko.xz"), None);
        assert_eq!(get_module_name("kernel/fs/ext4/ext4.ko.gz"), None);
    }

    #[test]
    fn get_module_name_no_extension() {
        // ACT / ASSERT
        assert_eq!(get_module_name("kernel/drivers/some_module"), None);
    }

    #[test]
    fn get_module_name_empty() {
        // ACT / ASSERT
        assert_eq!(get_module_name(""), None);
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
        let (_guard, db) = write_db("kernel/drivers/net/igc/igc.ko.zst:");

        // ACT / ASSERT
        assert_eq!(db.len(), 1);
        assert_eq!(
            db.get_path("igc"),
            Some("kernel/drivers/net/igc/igc.ko.zst")
        );
    }

    #[test]
    fn depdb_load_module_with_deps() {
        // ARRANGE
        let (_guard, db) = write_db(
            "kernel/drivers/net/igc/igc.ko.zst: kernel/drivers/ptp/ptp.ko.zst kernel/drivers/net/libphy.ko.zst\n\
             kernel/drivers/ptp/ptp.ko.zst:\n\
             kernel/drivers/net/libphy.ko.zst:",
        );

        // ACT / ASSERT
        assert_eq!(db.len(), 3);
        assert!(db.get_path("igc").is_some());
        assert!(db.get_path("ptp").is_some());
        assert!(db.get_path("libphy").is_some());
    }

    #[test]
    fn depdb_load_nonexistent_file() {
        // ACT / ASSERT
        let result = DepDb::load(Path::new("/nonexistent/modules.dep"));
        result.expect_err("load should fail for nonexistent file");
    }

    #[test]
    fn depdb_load_malformed_lines() {
        // ARRANGE
        let (_guard, db) = write_db("kernel/drivers/broken.ko.zst\nkernel/drivers/valid.ko.zst:");

        // ACT / ASSERT
        assert_eq!(db.len(), 1);
        assert!(db.get_path("valid").is_some());
        assert!(db.get_path("broken").is_none());
    }

    #[test]
    fn depdb_get_path_exists() {
        // ARRANGE
        let (_guard, db) = write_db("kernel/drivers/net/e1000e/e1000e.ko.zst:");

        // ACT / ASSERT
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
        let (_guard, db) = write_db("kernel/a.ko.zst:");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order, vec!["kernel/a.ko.zst"]);
    }

    #[test]
    fn resolve_load_order_single_dep() {
        // ARRANGE
        let (_guard, db) = write_db("kernel/a.ko.zst: kernel/b.ko.zst\nkernel/b.ko.zst:");

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order, vec!["kernel/b.ko.zst", "kernel/a.ko.zst"]);
    }

    #[test]
    fn resolve_load_order_dash_module_by_underscore_name() {
        // ARRANGE
        let (_guard, db) = write_db(
            "kernel/i2c/busses/i2c-i801.ko.zst: kernel/i2c/i2c-smbus.ko.zst\n\
             kernel/i2c/i2c-smbus.ko.zst:",
        );

        // ACT
        let order = db.resolve_load_order("i2c_i801").expect("resolve failed");

        // ASSERT
        assert_eq!(
            order,
            vec![
                "kernel/i2c/i2c-smbus.ko.zst",
                "kernel/i2c/busses/i2c-i801.ko.zst"
            ]
        );
    }

    #[test]
    fn resolve_load_order_chain() {
        // ARRANGE
        let (_guard, db) = write_db(
            "kernel/a.ko.zst: kernel/b.ko.zst\nkernel/b.ko.zst: kernel/c.ko.zst\nkernel/c.ko.zst:",
        );

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
        let (_guard, db) = write_db(
            "kernel/a.ko.zst: kernel/b.ko.zst kernel/c.ko.zst\n\
             kernel/b.ko.zst: kernel/d.ko.zst\n\
             kernel/c.ko.zst: kernel/d.ko.zst\n\
             kernel/d.ko.zst:",
        );

        // ACT
        let order = db.resolve_load_order("a").expect("resolve failed");

        // ASSERT
        assert_eq!(order.len(), 4);
        assert_eq!(order.iter().filter(|x| x.contains("/d.")).count(), 1);
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
        let (_guard, db) =
            write_db("kernel/a.ko.zst: kernel/b.ko.zst\nkernel/b.ko.zst: kernel/a.ko.zst");

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
        let (_guard, db) = write_db("kernel/a.ko.zst: kernel/b.ko.zst");

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

        // ACT / ASSERT
        assert_eq!(db.resolve_load_order("nonexistent"), None);
    }

    #[test]
    fn resolve_load_order_multiple_deps() {
        // ARRANGE
        let (_guard, db) = write_db(
            "kernel/a.ko.zst: kernel/b.ko.zst kernel/c.ko.zst kernel/d.ko.zst\n\
             kernel/b.ko.zst:\nkernel/c.ko.zst:\nkernel/d.ko.zst:",
        );

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
        let (_guard, db) = write_db(
            "kernel/drivers/net/ethernet/intel/igc/igc.ko.zst: kernel/drivers/ptp/ptp.ko.zst\n\
             kernel/drivers/ptp/ptp.ko.zst: kernel/drivers/pps/pps_core.ko.zst\n\
             kernel/drivers/pps/pps_core.ko.zst:\n\
             kernel/drivers/virtio/virtio.ko.zst:\n\
             kernel/drivers/virtio/virtio_net.ko.zst: kernel/drivers/virtio/virtio.ko.zst kernel/drivers/net/net_failover.ko.zst\n\
             kernel/drivers/net/net_failover.ko.zst:",
        );

        // ACT
        let igc_order = db.resolve_load_order("igc").expect("resolve failed");
        let vnet_order = db.resolve_load_order("virtio_net").expect("resolve failed");

        // ASSERT
        assert_eq!(db.len(), 6);
        assert_eq!(igc_order.len(), 3);
        assert!(
            igc_order
                .first()
                .expect("first module")
                .contains("pps_core")
        );
        assert!(igc_order.get(1).expect("second module").contains("ptp"));
        assert!(igc_order.get(2).expect("third module").contains("igc"));

        assert_eq!(vnet_order.len(), 3);
        let vnet_pos = vnet_order
            .iter()
            .position(|x| x.contains("virtio_net"))
            .expect("virtio_net not found");
        assert_eq!(vnet_pos, 2);
    }
}
