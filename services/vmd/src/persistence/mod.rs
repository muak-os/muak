mod state;

pub use state::{DiskConfigPersisted, VmPersisted, delete_vm, load_vms, save_vm};

pub const VMS_DIR: &str = "/run/state/vmd/vms";
