use std::fmt;
use std::fs::OpenOptions;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

const KVM_DEVICE: &str = "/dev/kvm";

#[derive(Debug)]
pub enum KvmCheckError {
    MissingDevice(PathBuf),
    NotCharDevice(PathBuf),
    Metadata(PathBuf, std::io::Error),
    Open(PathBuf, std::io::Error),
}

impl fmt::Display for KvmCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KvmCheckError::MissingDevice(path) => write!(
                f,
                "KVM device {path:?} not found. Enable virtualization in firmware and ensure the KVM module is loaded."
            ),
            KvmCheckError::NotCharDevice(path) => {
                write!(f, "{path:?} exists but is not a character device. Expected /dev/kvm to be a char device.")
            }
            KvmCheckError::Metadata(path, err) => {
                write!(f, "Failed to read metadata for {path:?}: {err}")
            }
            KvmCheckError::Open(path, err) => {
                write!(
                    f,
                    "Failed to open {path:?}: {err}. Ensure the muak init process has permission to access KVM."
                )
            }
        }
    }
}

impl std::error::Error for KvmCheckError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KvmCheckError::Metadata(_, err) | KvmCheckError::Open(_, err) => Some(err),
            _ => None,
        }
    }
}

pub fn ensure_kvm_available() -> Result<(), KvmCheckError> {
    verify_kvm_device(Path::new(KVM_DEVICE))
}

fn verify_kvm_device(path: &Path) -> Result<(), KvmCheckError> {
    if !path.exists() {
        return Err(KvmCheckError::MissingDevice(path.to_path_buf()));
    }

    let metadata = path
        .metadata()
        .map_err(|err| KvmCheckError::Metadata(path.to_path_buf(), err))?;

    if !metadata.file_type().is_char_device() {
        return Err(KvmCheckError::NotCharDevice(path.to_path_buf()));
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map(|_| ())
        .map_err(|err| KvmCheckError::Open(path.to_path_buf(), err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use uuid::Uuid;

    #[test]
    fn missing_device_returns_error() {
        let tmp_path = std::env::temp_dir().join(format!("muak-test-kvm-{}", Uuid::new_v4()));
        let err = verify_kvm_device(&tmp_path).expect_err("should fail when device is missing");
        assert!(matches!(err, KvmCheckError::MissingDevice(_)));
    }

    #[test]
    fn non_char_device_is_rejected() {
        let tmp_path = std::env::temp_dir().join(format!("muak-test-kvm-file-{}", Uuid::new_v4()));
        File::create(&tmp_path).expect("failed to create temp file");
        let err = verify_kvm_device(&tmp_path).expect_err("regular file should be rejected");
        fs::remove_file(&tmp_path).ok();
        assert!(matches!(err, KvmCheckError::NotCharDevice(_)));
    }
}
