#![cfg(test)]

//! A fake sysfs, procfs and dev tree shared by the blockdev integration tests.

use std::fs;
use std::path::{Path, PathBuf};

use blockdev::linux::Roots;
use tempfile::TempDir;

/// A temporary fake of the kernel trees blockdev reads.
pub struct Fixture {
    dir: TempDir,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    /// Creates an empty fixture with the sysfs, procfs and dev roots in place.
    ///
    /// # Panics
    ///
    /// Panics when the temporary trees cannot be created.
    #[must_use = "dropping it deletes the fixture directory"]
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("fixture directory");
        create_roots(dir.path());

        Self { dir }
    }

    /// Returns the roots pointing at this fixture.
    #[must_use = "callers need the roots to point the code at the fixture"]
    pub fn roots(&self) -> Roots {
        Roots {
            block: self.dir.path().join("sys/block"),
            class_block: self.dir.path().join("sys/class/block"),
            dev: self.dir.path().join("dev"),
            mounts: self.dir.path().join("proc/mounts"),
            swaps: self.dir.path().join("proc/swaps"),
        }
    }

    /// Creates a disk directory with the attributes the kernel reports for it.
    ///
    /// # Panics
    ///
    /// Panics when the disk attributes cannot be written.
    pub fn disk(&self, name: &str, size_sectors: u64) {
        let sysfs = self.roots().block.join(name);
        fs::create_dir_all(sysfs.join("device")).expect("device directory");
        fs::create_dir_all(sysfs.join("queue")).expect("queue directory");
        fs::write(sysfs.join("size"), format!("{size_sectors}\n")).expect("size");
        fs::write(sysfs.join("removable"), "0\n").expect("removable");
        fs::write(sysfs.join("ro"), "0\n").expect("read only");
        fs::write(sysfs.join("queue/rotational"), "0\n").expect("rotational");
        fs::write(sysfs.join("device/model"), format!("Model {name}\n")).expect("model");
        fs::write(sysfs.join("device/serial"), "").expect("serial");
        fs::write(sysfs.join("wwid"), "").expect("wwid");
        std::os::unix::fs::symlink("../../class/scsi", sysfs.join("device/subsystem"))
            .expect("subsystem link");

        let _node = self.dev_node(name);
        self.class_link(name, &format!("../../block/{name}"));
    }

    /// Creates a partition directory carrying `partname` as its GPT name.
    ///
    /// # Panics
    ///
    /// Panics when the partition attributes cannot be written.
    pub fn partition(
        &self,
        disk: &str,
        name: &str,
        number: u32,
        partname: &str,
        start: u64,
        size: u64,
    ) {
        let sysfs = self.roots().block.join(disk).join(name);
        fs::create_dir_all(&sysfs).expect("partition directory");
        fs::write(sysfs.join("partition"), format!("{number}\n")).expect("partition number");
        fs::write(sysfs.join("start"), format!("{start}\n")).expect("start");
        fs::write(sysfs.join("size"), format!("{size}\n")).expect("size");
        fs::write(
            sysfs.join("uevent"),
            format!("DEVNAME={name}\nDEVTYPE=partition\nPARTNAME={partname}\n"),
        )
        .expect("uevent");

        let _node = self.dev_node(name);
        self.class_link(name, &format!("../../block/{disk}/{name}"));
    }

    /// Overwrites one attribute of an existing disk or partition.
    ///
    /// # Panics
    ///
    /// Panics when the attribute cannot be written.
    pub fn attribute(&self, disk: &str, relative: &str, value: &str) {
        fs::write(self.roots().block.join(disk).join(relative), value).expect("attribute");
    }

    /// Creates an empty device node and returns its path.
    ///
    /// # Panics
    ///
    /// Panics when the device node cannot be written.
    #[must_use = "callers need the path to open the node just created"]
    pub fn dev_node(&self, name: &str) -> PathBuf {
        let node = self.roots().dev.join(name);
        fs::write(&node, b"").expect("device node");

        node
    }

    /// Replaces the kernel mount table.
    ///
    /// # Panics
    ///
    /// Panics when the mount table cannot be written.
    pub fn mounts(&self, content: &str) {
        fs::write(self.roots().mounts, content).expect("mount table");
    }

    /// Replaces the kernel swap table.
    ///
    /// # Panics
    ///
    /// Panics when the swap table cannot be written.
    pub fn swaps(&self, content: &str) {
        fs::write(self.roots().swaps, content).expect("swap table");
    }

    /// Adds a holder entry below `device`, which may be a partition.
    ///
    /// # Panics
    ///
    /// Panics when the holder entry cannot be written.
    pub fn holder(&self, device: &str, holder: &str) {
        let holders = self.roots().block.join(device).join("holders");
        fs::create_dir_all(&holders).expect("holders directory");
        fs::write(holders.join(holder), b"").expect("holder");
    }

    /// Sets the device-mapper name a holder reports.
    ///
    /// # Panics
    ///
    /// Panics when the mapping name cannot be written.
    pub fn mapping(&self, holder: &str, dm_name: &str) {
        let dm = self.roots().block.join(holder).join("dm");
        fs::create_dir_all(&dm).expect("dm directory");
        fs::write(dm.join("name"), format!("{dm_name}\n")).expect("dm name");
    }

    fn class_link(&self, name: &str, target: &str) {
        let link = self.roots().class_block.join(name);
        std::os::unix::fs::symlink(target, link).expect("class link");
    }
}

fn create_roots(dir: &Path) {
    for relative in ["sys/block", "sys/class/block", "dev", "proc"] {
        fs::create_dir_all(dir.join(relative)).expect("fixture root");
    }

    fs::write(dir.join("proc/mounts"), "").expect("mount table");
    fs::write(
        dir.join("proc/swaps"),
        "Filename\tType\tSize\tUsed\tPriority\n",
    )
    .expect("swap table");
}
