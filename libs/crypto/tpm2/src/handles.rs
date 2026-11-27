//! Internal TPM handle newtypes.

macro_rules! define_handle {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub(crate) struct $name(u32);

        impl $name {
            pub(crate) const fn raw(self) -> u32 {
                self.0
            }
        }

        impl From<u32> for $name {
            fn from(value: u32) -> Self {
                Self(value)
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                value.raw()
            }
        }
    };
}

define_handle!(HierarchyHandle);
define_handle!(PersistentHandle);
define_handle!(SessionHandle);
define_handle!(TransientHandle);

impl HierarchyHandle {
    pub(crate) const OWNER: Self = Self(0x4000_0001);
    pub(crate) const NULL: Self = Self(0x4000_0007);
}

impl PersistentHandle {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_conversions_roundtrip_raw_values() {
        // ARRANGE
        let hierarchy = HierarchyHandle::from(0x4000_0001);
        let persistent = PersistentHandle::new(0x8100_0001);
        let session = SessionHandle::from(0x0300_0000);
        let transient = TransientHandle::from(0x8000_0001);

        // ACT
        let hierarchy_raw = hierarchy.raw();
        let persistent_raw = u32::from(persistent);
        let session_raw = u32::from(session);
        let transient_raw = transient.raw();

        // ASSERT
        assert_eq!(
            hierarchy_raw, 0x4000_0001,
            "hierarchy raw value should match"
        );
        assert_eq!(
            persistent_raw, 0x8100_0001,
            "persistent raw value should match"
        );
        assert_eq!(session_raw, 0x0300_0000, "session raw value should match");
        assert_eq!(
            transient_raw, 0x8000_0001,
            "transient raw value should match"
        );
    }

    #[test]
    fn hierarchy_constants_match_expected_values() {
        // ASSERT
        assert_eq!(
            HierarchyHandle::OWNER.raw(),
            0x4000_0001,
            "owner handle should match"
        );
        assert_eq!(
            HierarchyHandle::NULL.raw(),
            0x4000_0007,
            "null handle should match"
        );
    }
}
