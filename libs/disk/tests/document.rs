//! Integration tests for the serialized disk document.

pub mod common;

#[cfg(test)]
mod tests {
    use disk::document::{Document, Partition, guid};
    use disk::error::DiskError;
    use disk::plan::Size;
    use disk::role::Role;
    use disk::sizes::ESP_TYPE_GUID;

    use crate::common;

    #[test]
    fn toml_round_trip_preserves_partitions() {
        // ARRANGE
        let doc = common::uefi_document();

        // ACT
        let serialized = doc.to_toml().expect("serialize");
        let parsed = Document::from_toml(&serialized).expect("deserialize");

        // ASSERT
        assert_eq!(parsed, doc, "round trip must preserve the document");
    }

    #[test]
    fn serialized_document_carries_api_version() {
        // ARRANGE
        let doc = common::uefi_document();

        // ACT
        let serialized = doc.to_toml().expect("serialize");

        // ASSERT
        assert!(
            serialized.contains("api_version = \"muak.dev/disk/v1-beta\""),
            "serialized document must carry the api version: {serialized}"
        );
    }

    #[test]
    fn from_toml_rejects_unknown_api_version() {
        // ARRANGE
        let bytes = "api_version = \"muak.dev/disk/v999\"\nwipe = true\n[[partitions]]\nrole = \"esp\"\nname = \"EFI\"\ntype_guid = \"00000000-0000-0000-0000-000000000000\"\nsize = \"fill\"\n";

        // ACT
        let result = Document::from_toml(bytes);

        // ASSERT
        assert!(
            matches!(result, Err(DiskError::UnsupportedApiVersion(_))),
            "unknown api_version must be rejected"
        );
    }

    #[test]
    fn from_toml_rejects_missing_api_version() {
        // ARRANGE
        let bytes = "wipe = true\n[[partitions]]\nrole = \"esp\"\nname = \"EFI\"\ntype_guid = \"00000000-0000-0000-0000-000000000000\"\nsize = \"fill\"\n";

        // ACT
        let result = Document::from_toml(bytes);

        // ASSERT
        let error = result.expect_err("missing api_version must fail");
        assert!(error.to_string().contains("api_version"), "{error}");
    }

    #[test]
    fn find_returns_partition_by_role() {
        // ARRANGE
        let doc = common::uefi_document();

        // ACT
        let found = doc.find(Role::State).expect("state partition");

        // ASSERT
        assert_eq!(found.name, "STATE", "role lookup must return STATE");
    }

    #[test]
    fn find_returns_none_for_missing_role() {
        // ARRANGE
        let doc = Document::new(
            true,
            vec![Partition {
                role: Role::Esp,
                name: "EFI".to_owned(),
                type_guid: guid(&ESP_TYPE_GUID),
                size: Size::Fill,
            }],
        );

        // ACT
        let found = doc.find(Role::Data);

        // ASSERT
        assert!(found.is_none(), "absent roles must return none");
    }

    #[test]
    fn read_parses_document_from_the_filesystem() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("disk.toml");
        let doc = common::uefi_document();
        std::fs::write(&path, doc.to_toml().expect("serialize")).expect("write fixture");

        // ACT
        let loaded = Document::read(&path).expect("read");

        // ASSERT
        assert_eq!(loaded, doc, "read must parse the written document");
    }
}
