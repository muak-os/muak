//! Authenticated user wrapper with permission checking helpers.

use std::collections::HashSet;

use config::Permission;

/// Represents an authenticated user with their permissions.
///
/// This is a lightweight wrapper that provides permission checking helpers
/// and abstracts away the underlying permission storage.
#[derive(Debug, Clone)]
pub(super) struct AuthenticatedUser {
    permissions: HashSet<Permission>,
}

impl AuthenticatedUser {
    /// Creates a new authenticated user from a permission list.
    #[cfg(test)]
    pub fn new<I>(permissions: I) -> Self
    where
        I: IntoIterator<Item = Permission>,
    {
        Self {
            permissions: permissions.into_iter().collect(),
        }
    }

    /// Checks if the user has a specific permission.
    ///
    /// Admin users automatically bypass permission checks.
    #[must_use]
    pub fn has_permission(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission) || self.permissions.contains(&Permission::Admin)
    }

    /// Checks if the user has the admin permission.
    #[cfg(test)]
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.permissions.contains(&Permission::Admin)
    }

    /// Returns the number of permissions this user has.
    #[cfg(test)]
    #[must_use]
    pub fn permission_count(&self) -> usize {
        self.permissions.len()
    }
}

/// Converts from `config::AuthUser` to our `AuthenticatedUser`.
impl From<&config::AuthUser> for AuthenticatedUser {
    fn from(user: &config::AuthUser) -> Self {
        Self {
            permissions: user.permissions.iter().copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_permission() {
        // ARRANGE
        let user = AuthenticatedUser::new(vec![Permission::VmRead, Permission::VmCreate]);

        // ACT & ASSERT
        assert!(user.has_permission(Permission::VmRead));
        assert!(user.has_permission(Permission::VmCreate));

        assert!(!user.has_permission(Permission::VmDelete));
        assert!(!user.has_permission(Permission::Admin));
    }

    #[test]
    fn admin_has_all_permissions() {
        // ARRANGE
        let admin_user = AuthenticatedUser::new(vec![Permission::Admin]);

        // ACT & ASSERT
        assert!(admin_user.has_permission(Permission::VmRead));
        assert!(admin_user.has_permission(Permission::VmCreate));
        assert!(admin_user.has_permission(Permission::VmDelete));
        assert!(admin_user.has_permission(Permission::AuthManage));
        assert!(admin_user.has_permission(Permission::SystemUpdate));

        assert!(admin_user.has_permission(Permission::Admin));
    }

    #[test]
    fn regular_user_limited_access() {
        // ARRANGE
        let regular_user = AuthenticatedUser::new(vec![Permission::VmRead]);

        // ACT & ASSERT
        assert!(regular_user.has_permission(Permission::VmRead));

        assert!(!regular_user.has_permission(Permission::VmCreate));
        assert!(!regular_user.has_permission(Permission::VmDelete));
        assert!(!regular_user.has_permission(Permission::Admin));
    }

    #[test]
    fn is_admin() {
        // ARRANGE
        let regular = AuthenticatedUser::new(vec![Permission::VmRead]);
        let admin = AuthenticatedUser::new(vec![Permission::Admin]);

        // ACT & ASSERT
        assert!(!regular.is_admin());
        assert!(admin.is_admin());
    }

    #[test]
    fn permission_count() {
        // ARRANGE
        let user = AuthenticatedUser::new(vec![
            Permission::VmRead,
            Permission::VmCreate,
            Permission::VmRead, // Duplicate
        ]);

        // ACT
        let count = user.permission_count();

        // ASSERT
        // HashSet deduplicates
        assert_eq!(count, 2);
    }
}
