//! Fine-grained permissions for RBAC.

use std::collections::HashSet;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Fine-grained permissions for RBAC.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Full administrative access to all operations.
    #[serde(rename = "admin")]
    Admin,

    // VM operations
    /// Read VM state and configuration.
    #[serde(rename = "vm:read")]
    VmRead,

    /// Create new VMs.
    #[serde(rename = "vm:create")]
    VmCreate,

    /// Start existing VMs.
    #[serde(rename = "vm:start")]
    VmStart,

    /// Stop running VMs.
    #[serde(rename = "vm:stop")]
    VmStop,

    /// Delete existing VMs.
    #[serde(rename = "vm:delete")]
    VmDelete,

    /// Upload VM images.
    #[serde(rename = "vm:upload")]
    VmUpload,

    // Auth/certificate management
    /// Manage authentication and certificate operations.
    #[serde(rename = "auth:manage")]
    AuthManage,

    // System/Provision operations
    /// Read system configuration and state.
    #[serde(rename = "system:read")]
    SystemRead,

    /// Update system configuration.
    #[serde(rename = "system:update")]
    SystemUpdate,

    // Process monitoring
    /// Read process information and metrics.
    #[serde(rename = "process:read")]
    ProcessRead,

    // Security monitoring
    /// Read security events and audit logs.
    #[serde(rename = "security:read")]
    SecurityRead,
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
            Self::SecurityRead => "security:read",
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
            "security:read" => Ok(Self::SecurityRead),
            _ => Err(format!("Unknown permission: {}", s)),
        }
    }
}

/// Known permission categories that support wildcards.
const CATEGORIES: &[&str] = &["vm", "system", "auth", "process", "security"];

impl Permission {
    /// Returns all permissions in a category.
    #[must_use]
    pub fn all_in_category(category: &str) -> &'static [Permission] {
        match category {
            "vm" => &[
                Permission::VmRead,
                Permission::VmCreate,
                Permission::VmStart,
                Permission::VmStop,
                Permission::VmDelete,
                Permission::VmUpload,
            ],
            "system" => &[Permission::SystemRead, Permission::SystemUpdate],
            "auth" => &[Permission::AuthManage],
            "process" => &[Permission::ProcessRead],
            "security" => &[Permission::SecurityRead],
            _ => &[],
        }
    }

    /// Expands a permission pattern (wildcard) to concrete permissions.
    pub fn expand_pattern(pattern: &str) -> Result<Vec<Permission>, String> {
        let Some(category) = pattern.strip_suffix(":*") else {
            return pattern.parse().map(|p| vec![p]);
        };
        let perms = Self::all_in_category(category);
        if perms.is_empty() {
            Err(format!(
                "Unknown permission category: '{}'. Valid categories: {}",
                category,
                CATEGORIES.join(", ")
            ))
        } else {
            Ok(perms.to_vec())
        }
    }

    /// Returns the category prefix for this permission (e.g., "vm" for VmRead).
    #[must_use]
    pub fn category(&self) -> Option<&'static str> {
        match self {
            Self::Admin => None,
            Self::VmRead
            | Self::VmCreate
            | Self::VmStart
            | Self::VmStop
            | Self::VmDelete
            | Self::VmUpload => Some("vm"),
            Self::AuthManage => Some("auth"),
            Self::SystemRead | Self::SystemUpdate => Some("system"),
            Self::ProcessRead => Some("process"),
            Self::SecurityRead => Some("security"),
        }
    }
}

/// Collapses a list of permissions, replacing complete categories with wildcards.
pub fn collapse(permissions: &[Permission]) -> Vec<String> {
    let perm_set: HashSet<Permission> = permissions.iter().copied().collect();
    let mut result = Vec::new();
    let mut handled = HashSet::new();

    if perm_set.contains(&Permission::Admin) {
        return vec!["admin".to_string()];
    }

    for category in CATEGORIES {
        let category_perms = Permission::all_in_category(category);
        if category_perms.iter().all(|p| perm_set.contains(p)) {
            result.push(format!("{}:*", category));
            handled.extend(category_perms.iter().copied());
        }
    }

    let mut remaining: Vec<_> = permissions
        .iter()
        .filter(|p| !handled.contains(p))
        .map(|p| p.to_string())
        .collect();
    remaining.sort();
    remaining.dedup();
    result.extend(remaining);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_display() {
        // ASSERT
        assert_eq!(Permission::Admin.to_string(), "admin");
        assert_eq!(Permission::VmRead.to_string(), "vm:read");
        assert_eq!(Permission::VmCreate.to_string(), "vm:create");
        assert_eq!(Permission::AuthManage.to_string(), "auth:manage");
        assert_eq!(Permission::SystemUpdate.to_string(), "system:update");
        assert_eq!(Permission::ProcessRead.to_string(), "process:read");
        assert_eq!(Permission::SecurityRead.to_string(), "security:read");
    }

    #[test]
    fn permission_from_str() {
        // ASSERT
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
        assert_eq!(
            "security:read".parse::<Permission>().unwrap(),
            Permission::SecurityRead
        );

        assert!("invalid".parse::<Permission>().is_err());
    }

    #[test]
    fn all_in_category() {
        // ARRANGE & ACT
        let vm_perms = Permission::all_in_category("vm");
        let system_perms = Permission::all_in_category("system");
        let auth_perms = Permission::all_in_category("auth");
        let process_perms = Permission::all_in_category("process");
        let security_perms = Permission::all_in_category("security");

        // ASSERT
        assert_eq!(vm_perms.len(), 6);
        assert!(vm_perms.contains(&Permission::VmRead));
        assert!(vm_perms.contains(&Permission::VmCreate));
        assert!(vm_perms.contains(&Permission::VmStart));
        assert!(vm_perms.contains(&Permission::VmStop));
        assert!(vm_perms.contains(&Permission::VmDelete));
        assert!(vm_perms.contains(&Permission::VmUpload));

        assert_eq!(system_perms.len(), 2);
        assert!(system_perms.contains(&Permission::SystemRead));
        assert!(system_perms.contains(&Permission::SystemUpdate));

        assert_eq!(auth_perms.len(), 1);
        assert!(auth_perms.contains(&Permission::AuthManage));

        assert_eq!(process_perms.len(), 1);
        assert!(process_perms.contains(&Permission::ProcessRead));

        assert_eq!(security_perms.len(), 1);
        assert!(security_perms.contains(&Permission::SecurityRead));

        assert!(Permission::all_in_category("unknown").is_empty());
    }

    #[test]
    fn expand_pattern_wildcards() {
        // ARRANGE & ACT
        let vm_perms = Permission::expand_pattern("vm:*").unwrap();
        let system_perms = Permission::expand_pattern("system:*").unwrap();
        let auth_perms = Permission::expand_pattern("auth:*").unwrap();
        let process_perms = Permission::expand_pattern("process:*").unwrap();
        let security_perms = Permission::expand_pattern("security:*").unwrap();

        // ASSERT
        assert_eq!(vm_perms.len(), 6);
        assert_eq!(system_perms.len(), 2);
        assert_eq!(auth_perms.len(), 1);
        assert_eq!(auth_perms[0], Permission::AuthManage);
        assert_eq!(process_perms.len(), 1);
        assert_eq!(process_perms[0], Permission::ProcessRead);
        assert_eq!(security_perms.len(), 1);
        assert_eq!(security_perms[0], Permission::SecurityRead);
    }

    #[test]
    fn expand_pattern_single() {
        // ARRANGE & ACT
        let perms = Permission::expand_pattern("vm:read").unwrap();
        let admin_perms = Permission::expand_pattern("admin").unwrap();

        // ASSERT
        assert_eq!(perms, vec![Permission::VmRead]);
        assert_eq!(admin_perms, vec![Permission::Admin]);
    }

    #[test]
    fn expand_pattern_invalid() {
        // ARRANGE & ACT
        let result = Permission::expand_pattern("invalid:*");
        let result2 = Permission::expand_pattern("vm:invalid");

        // ASSERT
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown permission category"));
        assert!(result2.is_err());
        assert!(result2.unwrap_err().contains("Unknown permission"));
    }

    #[test]
    fn category() {
        // ASSERT
        assert_eq!(Permission::Admin.category(), None);
        assert_eq!(Permission::VmRead.category(), Some("vm"));
        assert_eq!(Permission::VmCreate.category(), Some("vm"));
        assert_eq!(Permission::SystemRead.category(), Some("system"));
        assert_eq!(Permission::AuthManage.category(), Some("auth"));
        assert_eq!(Permission::ProcessRead.category(), Some("process"));
        assert_eq!(Permission::SecurityRead.category(), Some("security"));
    }

    #[test]
    fn collapse_to_wildcards_complete_category() {
        // ARRANGE
        let perms = vec![
            Permission::VmRead,
            Permission::VmCreate,
            Permission::VmStart,
            Permission::VmStop,
            Permission::VmDelete,
            Permission::VmUpload,
        ];

        // ACT
        let collapsed = collapse(&perms);

        // ASSERT
        assert_eq!(collapsed, vec!["vm:*"]);

        let perms = vec![Permission::SystemRead, Permission::SystemUpdate];
        let collapsed = collapse(&perms);
        assert_eq!(collapsed, vec!["system:*"]);
    }

    #[test]
    fn collapse_to_wildcards_partial_category() {
        // ARRANGE
        let perms = vec![Permission::VmRead, Permission::VmCreate];

        // ACT
        let collapsed = collapse(&perms);

        // ASSERT
        assert!(collapsed.contains(&"vm:read".to_string()));
        assert!(collapsed.contains(&"vm:create".to_string()));
        assert!(!collapsed.contains(&"vm:*".to_string()));
    }

    #[test]
    fn collapse_to_wildcards_mixed() {
        // ARRANGE
        let perms = vec![
            Permission::VmRead,
            Permission::VmCreate,
            Permission::VmStart,
            Permission::VmStop,
            Permission::VmDelete,
            Permission::VmUpload,
            Permission::SystemRead,
        ];

        // ACT
        let collapsed = collapse(&perms);

        // ASSERT
        assert!(collapsed.contains(&"vm:*".to_string()));
        assert!(collapsed.contains(&"system:read".to_string()));
        assert!(!collapsed.contains(&"system:*".to_string()));
    }

    #[test]
    fn collapse_to_wildcards_admin() {
        // ARRANGE
        let perms = vec![Permission::Admin, Permission::VmRead];

        // ACT
        let collapsed = collapse(&perms);

        // ASSERT
        assert_eq!(collapsed, vec!["admin"]);
    }

    #[test]
    fn collapse_to_wildcards_empty() {
        // ACT
        let collapsed = collapse(&[]);

        // ASSERT
        assert!(collapsed.is_empty());
    }

    #[test]
    fn permission_round_trip() {
        // ARRANGE
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
            Permission::SecurityRead,
        ];

        // ACT & ASSERT
        for perm in permissions {
            let string = perm.to_string();
            let parsed = string.parse::<Permission>().unwrap();
            assert_eq!(perm, parsed);
        }
    }

    #[test]
    fn parse_permission() {
        // ARRANGE
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

        // ACT
        let serialized = toml::to_string(&original).expect("serialization failed");
        let deserialized: Wrapper = toml::from_str(&serialized).expect("deserialization failed");

        // ASSERT
        assert!(serialized.contains("\"vm:read\""));
        assert!(serialized.contains("\"auth:manage\""));
        assert_eq!(original, deserialized);
    }
}
