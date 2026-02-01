//! Fine-grained permissions for RBAC.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Fine-grained permissions for RBAC.
///
/// Permissions follow the `resource:action` naming convention.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Permission {
    // Full administrative access to all operations.
    #[serde(rename = "admin")]
    Admin,

    // VM operations
    #[serde(rename = "vm:read")]
    VmRead,

    #[serde(rename = "vm:create")]
    VmCreate,

    #[serde(rename = "vm:start")]
    VmStart,

    #[serde(rename = "vm:stop")]
    VmStop,

    #[serde(rename = "vm:delete")]
    VmDelete,

    #[serde(rename = "vm:upload")]
    VmUpload,

    // Auth/certificate management
    #[serde(rename = "auth:manage")]
    AuthManage,

    // System/Provision operations
    #[serde(rename = "system:read")]
    SystemRead,

    #[serde(rename = "system:update")]
    SystemUpdate,

    // Process monitoring
    #[serde(rename = "process:read")]
    ProcessRead,
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Admin => "admin",
            Self::VmRead => "vm:read",
            Self::VmCreate => "vm:create",
            Self::VmStart => "vm:start",
            Self::VmStop => "vm:stop",
            Self::VmDelete => "vm:delete",
            Self::VmUpload => "vm:upload",
            Self::AuthManage => "auth:manage",
            Self::SystemRead => "system:read",
            Self::SystemUpdate => "system:update",
            Self::ProcessRead => "process:read",
        };
        write!(f, "{s}")
    }
}

impl FromStr for Permission {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "vm:read" => Ok(Self::VmRead),
            "vm:create" => Ok(Self::VmCreate),
            "vm:start" => Ok(Self::VmStart),
            "vm:stop" => Ok(Self::VmStop),
            "vm:delete" => Ok(Self::VmDelete),
            "vm:upload" => Ok(Self::VmUpload),
            "auth:manage" => Ok(Self::AuthManage),
            "system:read" => Ok(Self::SystemRead),
            "system:update" => Ok(Self::SystemUpdate),
            "process:read" => Ok(Self::ProcessRead),
            _ => Err(format!("Unknown permission: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_display() {
        assert_eq!(Permission::Admin.to_string(), "admin");
        assert_eq!(Permission::VmRead.to_string(), "vm:read");
        assert_eq!(Permission::VmCreate.to_string(), "vm:create");
        assert_eq!(Permission::AuthManage.to_string(), "auth:manage");
        assert_eq!(Permission::SystemUpdate.to_string(), "system:update");
        assert_eq!(Permission::ProcessRead.to_string(), "process:read");
    }

    #[test]
    fn test_permission_from_str() {
        assert_eq!("admin".parse::<Permission>().unwrap(), Permission::Admin);
        assert_eq!("vm:read".parse::<Permission>().unwrap(), Permission::VmRead);
        assert_eq!(
            "vm:create".parse::<Permission>().unwrap(),
            Permission::VmCreate
        );
        assert_eq!(
            "auth:manage".parse::<Permission>().unwrap(),
            Permission::AuthManage
        );
        assert_eq!(
            "system:update".parse::<Permission>().unwrap(),
            Permission::SystemUpdate
        );
        assert_eq!(
            "process:read".parse::<Permission>().unwrap(),
            Permission::ProcessRead
        );

        assert!("invalid".parse::<Permission>().is_err());
    }

    #[test]
    fn test_permission_round_trip() {
        let permissions = [
            Permission::Admin,
            Permission::VmRead,
            Permission::VmCreate,
            Permission::VmStart,
            Permission::VmStop,
            Permission::VmDelete,
            Permission::VmUpload,
            Permission::AuthManage,
            Permission::SystemRead,
            Permission::SystemUpdate,
            Permission::ProcessRead,
        ];

        for perm in permissions {
            let string = perm.to_string();
            let parsed = string.parse::<Permission>().unwrap();
            assert_eq!(perm, parsed);
        }
    }

    #[test]
    fn test_parse_permission() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Wrapper {
            permissions: Vec<Permission>,
        }

        let original = Wrapper {
            permissions: vec![
                Permission::Admin,
                Permission::VmRead,
                Permission::AuthManage,
            ],
        };

        let serialized = toml::to_string(&original).unwrap();
        assert!(serialized.contains("\"vm:read\""));
        assert!(serialized.contains("\"auth:manage\""));

        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }
}
