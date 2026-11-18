//! cgroup v2 hierarchy setup.

use core::ffi::CStr;
use std::fs;
use std::io::Write as _;

use anyhow::{Context as _, Result};
use rustix::mount::{MountFlags, mount};

const FSTYPE: &str = "cgroup2";
const FLAGS: MountFlags = MountFlags::NOSUID
    .union(MountFlags::NOEXEC)
    .union(MountFlags::NODEV);
// `nsdelegate` is the only option this kernel accepts. `xattr` is rejected as
// an unknown parameter, so per cgroup labels are not requested and the
// `cgroup_seclabel` policy capability stays unused rather than being a reason
// to pass an option the kernel refuses.
const OPTIONS: &CStr = c"nsdelegate";

/// Mount point of the unified hierarchy, and the root of every service cgroup.
pub(crate) const ROOT: &str = "/sys/fs/cgroup";
const CONTROLLERS: &str = "/sys/fs/cgroup/cgroup.controllers";
const MOUNTINFO: &str = "/proc/self/mountinfo";
const SUBTREE_CONTROL: &str = "/sys/fs/cgroup/cgroup.subtree_control";
const REQUIRED: [&str; 2] = ["memory", "pids"];

/// Prepares the unified cgroup v2 hierarchy for service management.
pub(crate) fn setup() -> Result<()> {
    require_root()?;

    if root_mounted()? {
        kmsg::info!("cgroup v2 hierarchy already mounted");
    } else {
        mount_root().context("mounting the cgroup2 hierarchy")?;
    }

    enable_controllers()
}

fn require_root() -> Result<()> {
    let metadata = fs::metadata(ROOT)
        .with_context(|| format!("{ROOT} is missing, is CONFIG_CGROUPS enabled?"))?;

    anyhow::ensure!(metadata.is_dir(), "{ROOT} is not a directory");

    Ok(())
}

fn root_mounted() -> Result<bool> {
    let mountinfo =
        fs::read_to_string(MOUNTINFO).with_context(|| format!("reading {MOUNTINFO}"))?;

    Ok(is_mounted(&mountinfo, ROOT))
}

fn mount_root() -> Result<()> {
    mount(FSTYPE, ROOT, FSTYPE, FLAGS, Some(OPTIONS))?;

    Ok(())
}

fn enable_controllers() -> Result<()> {
    let contents =
        fs::read_to_string(CONTROLLERS).with_context(|| format!("reading {CONTROLLERS}"))?;

    if let Some(name) = missing_required(&contents) {
        anyhow::bail!("{CONTROLLERS} does not list the required {name} controller");
    }

    let mut enabled = 0_usize;
    for name in controller_names(&contents) {
        match enable_controller(name) {
            Ok(()) => enabled = enabled.saturating_add(1),
            Err(e) if REQUIRED.contains(&name) => {
                return Err(e.context(format!("enabling the {name} controller")));
            }
            Err(e) => kmsg::warn!("cgroup: enabling {name} failed: {e}"),
        }
    }

    kmsg::info!("cgroup: enabled {enabled} controllers");

    Ok(())
}

fn enable_controller(name: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new().write(true).open(SUBTREE_CONTROL)?;
    file.write_all(request(name).as_bytes())?;

    Ok(())
}

fn is_mounted(mountinfo: &str, target: &str) -> bool {
    mountinfo
        .lines()
        .any(|line| line.split(' ').nth(4) == Some(target) && fstype(line) == Some(FSTYPE))
}

fn fstype(line: &str) -> Option<&str> {
    line.split(" - ").nth(1)?.split(' ').next()
}

fn missing_required(contents: &str) -> Option<&'static str> {
    REQUIRED
        .into_iter()
        .find(|required| !controller_names(contents).any(|name| name == *required))
}

fn controller_names(contents: &str) -> impl Iterator<Item = &str> {
    contents.split_whitespace()
}

fn request(name: &str) -> String {
    let mut request = String::with_capacity(name.len().saturating_add(1));
    request.push('+');
    request.push_str(name);

    request
}

#[cfg(test)]
mod tests {
    use super::{controller_names, is_mounted, missing_required, request};

    const MOUNTINFO: &str = "22 1 0:21 / /sys rw,nosuid shared:5 - sysfs sysfs rw\n\
                             88 22 0:31 / /sys/fs/cgroup rw,nosuid shared:4 - cgroup2 cgroup2 rw\n\
                             89 22 0:32 / /sys/fs/cgroup/foo rw,nosuid shared:6 - cgroup2 cgroup2 rw\n";

    const CHILD_MOUNTINFO: &str =
        "89 22 0:32 / /sys/fs/cgroup/foo rw,nosuid shared:6 - cgroup2 cgroup2 rw\n";

    #[test]
    fn detects_the_mount_point() {
        // ARRANGE
        let mountinfo = MOUNTINFO;

        // ACT
        let mounted = is_mounted(mountinfo, "/sys/fs/cgroup");

        // ASSERT
        assert!(mounted);
    }

    #[test]
    fn rejects_a_non_cgroup2_mount_point() {
        // ARRANGE
        let mountinfo = "36 22 0:31 / /sys/fs/cgroup rw,nosuid shared:4 - tmpfs tmpfs rw\n";

        // ACT
        let mounted = is_mounted(mountinfo, "/sys/fs/cgroup");

        // ASSERT
        assert!(!mounted);
    }

    #[test]
    fn accepts_a_cgroup2_mount_point() {
        // ARRANGE
        let mountinfo = "36 22 0:31 / /sys/fs/cgroup rw,nosuid shared:4 - cgroup2 cgroup2 rw\n";

        // ACT
        let mounted = is_mounted(mountinfo, "/sys/fs/cgroup");

        // ASSERT
        assert!(mounted);
    }

    #[test]
    fn requires_an_exact_mount_point_match() {
        // ARRANGE
        let mountinfo = CHILD_MOUNTINFO;

        // ACT
        let parent_mounted = is_mounted(mountinfo, "/sys/fs/cgroup");
        let partial_mounted = is_mounted(mountinfo, "/sys/fs/cgroup/fo");

        // ASSERT
        assert!(!parent_mounted);
        assert!(!partial_mounted);
    }

    #[test]
    fn splits_controller_names_on_whitespace() {
        // ARRANGE
        let contents = "cpu io memory\n";

        // ACT
        let names: Vec<&str> = controller_names(contents).collect();

        // ASSERT
        assert_eq!(names, ["cpu", "io", "memory"]);
    }

    #[test]
    fn accepts_contents_listing_every_required_controller() {
        // ARRANGE
        let contents = "cpuset cpu io memory pids\n";

        // ACT
        let missing = missing_required(contents);

        // ASSERT
        assert_eq!(missing, None);
    }

    #[test]
    fn flags_a_missing_memory_controller() {
        // ARRANGE
        let contents = "cpuset cpu io pids\n";

        // ACT
        let missing = missing_required(contents);

        // ASSERT
        assert_eq!(missing, Some("memory"));
    }

    #[test]
    fn flags_a_missing_pids_controller() {
        // ARRANGE
        let contents = "cpuset cpu io memory\n";

        // ACT
        let missing = missing_required(contents);

        // ASSERT
        assert_eq!(missing, Some("pids"));
    }

    #[test]
    fn treats_an_empty_controller_list_as_a_failure() {
        // ARRANGE
        let contents = "";

        // ACT
        let missing = missing_required(contents);

        // ASSERT
        assert_eq!(missing, Some("memory"));
    }

    #[test]
    fn builds_a_single_enable_request() {
        // ARRANGE
        let name = "cpu";

        // ACT
        let payload = request(name);

        // ASSERT
        assert_eq!(payload, "+cpu");
    }

    #[test]
    fn yields_no_controller_for_empty_contents() {
        // ARRANGE
        let contents = "";

        // ACT
        let mut names = controller_names(contents);

        // ASSERT
        assert!(names.next().is_none());
    }
}
