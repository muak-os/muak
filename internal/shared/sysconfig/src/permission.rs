//! Fine-grained permissions for RBAC.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

/// Known permission categories that support wildcards.
const CATEGORIES: &[&str] = &["vm", "system", "auth", "process"];

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
            _ => &[],
        }
    }

    /// Expands a permission pattern (wildcard) to concrete permissions.
    pub fn expand_pattern(pattern: &str) -> Result<Vec<Permission>, String> {
        if let Some(category) = pattern.strip_suffix(":*") {
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
        } else {
            pattern.parse().map(|p| vec![p])
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
    fn test_all_in_category() {
        let vm_perms = Permission::all_in_category("vm");
        assert_eq!(vm_perms.len(), 6);
        assert!(vm_perms.contains(&Permission::VmRead));
        assert!(vm_perms.contains(&Permission::VmCreate));
        assert!(vm_perms.contains(&Permission::VmStart));
        assert!(vm_perms.contains(&Permission::VmStop));
        assert!(vm_perms.contains(&Permission::VmDelete));
        assert!(vm_perms.contains(&Permission::VmUpload));

        let system_perms = Permission::all_in_category("system");
        assert_eq!(system_perms.len(), 2);
        assert!(system_perms.contains(&Permission::SystemRead));
        assert!(system_perms.contains(&Permission::SystemUpdate));

        let auth_perms = Permission::all_in_category("auth");
        assert_eq!(auth_perms.len(), 1);
        assert!(auth_perms.contains(&Permission::AuthManage));

        let process_perms = Permission::all_in_category("process");
        assert_eq!(process_perms.len(), 1);
        assert!(process_perms.contains(&Permission::ProcessRead));

        assert!(Permission::all_in_category("unknown").is_empty());
    }

    #[test]
    fn test_expand_pattern_wildcards() {
        let vm_perms = Permission::expand_pattern("vm:*").unwrap();
        assert_eq!(vm_perms.len(), 6);

        let system_perms = Permission::expand_pattern("system:*").unwrap();
        assert_eq!(system_perms.len(), 2);

        let auth_perms = Permission::expand_pattern("auth:*").unwrap();
        assert_eq!(auth_perms.len(), 1);
        assert_eq!(auth_perms[0], Permission::AuthManage);

        let process_perms = Permission::expand_pattern("process:*").unwrap();
        assert_eq!(process_perms.len(), 1);
        assert_eq!(process_perms[0], Permission::ProcessRead);
    }

    #[test]
    fn test_expand_pattern_single() {
        let perms = Permission::expand_pattern("vm:read").unwrap();
        assert_eq!(perms, vec![Permission::VmRead]);

        let perms = Permission::expand_pattern("admin").unwrap();
        assert_eq!(perms, vec![Permission::Admin]);
    }

    #[test]
    fn test_expand_pattern_invalid() {
        let result = Permission::expand_pattern("invalid:*");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown permission category"));

        let result = Permission::expand_pattern("vm:invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown permission"));
    }

    #[test]
    fn test_category() {
        assert_eq!(Permission::Admin.category(), None);
        assert_eq!(Permission::VmRead.category(), Some("vm"));
        assert_eq!(Permission::VmCreate.category(), Some("vm"));
        assert_eq!(Permission::SystemRead.category(), Some("system"));
        assert_eq!(Permission::AuthManage.category(), Some("auth"));
        assert_eq!(Permission::ProcessRead.category(), Some("process"));
    }

    #[test]
    fn test_collapse_to_wildcards_complete_category() {
        let perms = vec![
            Permission::VmRead,
            Permission::VmCreate,
            Permission::VmStart,
            Permission::VmStop,
            Permission::VmDelete,
            Permission::VmUpload,
        ];
        let collapsed = collapse(&perms);
        assert_eq!(collapsed, vec!["vm:*"]);

        let perms = vec![Permission::SystemRead, Permission::SystemUpdate];
        let collapsed = collapse(&perms);
        assert_eq!(collapsed, vec!["system:*"]);
    }

    #[test]
    fn test_collapse_to_wildcards_partial_category() {
        let perms = vec![Permission::VmRead, Permission::VmCreate];
        let collapsed = collapse(&perms);
        assert!(collapsed.contains(&"vm:read".to_string()));
        assert!(collapsed.contains(&"vm:create".to_string()));
        assert!(!collapsed.contains(&"vm:*".to_string()));
    }

    #[test]
    fn test_collapse_to_wildcards_mixed() {
        let perms = vec![
            Permission::VmRead,
            Permission::VmCreate,
            Permission::VmStart,
            Permission::VmStop,
            Permission::VmDelete,
            Permission::VmUpload,
            Permission::SystemRead,
        ];
        let collapsed = collapse(&perms);
        assert!(collapsed.contains(&"vm:*".to_string()));
        assert!(collapsed.contains(&"system:read".to_string()));
        assert!(!collapsed.contains(&"system:*".to_string()));
    }

    #[test]
    fn test_collapse_to_wildcards_admin() {
        let perms = vec![Permission::Admin, Permission::VmRead];
        let collapsed = collapse(&perms);
        assert_eq!(collapsed, vec!["admin"]);
    }

    #[test]
    fn test_collapse_to_wildcards_empty() {
        let collapsed = collapse(&[]);
        assert!(collapsed.is_empty());
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
