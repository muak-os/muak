//! gRPC service implementations for provisiond.

pub(crate) mod auth;
pub(crate) mod provision;
pub(crate) mod security;
pub(crate) mod version;

#[expect(
    clippy::absolute_paths,
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    clippy::arbitrary_source_item_ordering,
    clippy::as_conversions,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::doc_paragraphs_missing_punctuation,
    clippy::empty_structs_with_brackets,
    clippy::excessive_nesting,
    clippy::impl_trait_in_params,
    clippy::inline_modules,
    clippy::pattern_type_mismatch,
    clippy::std_instead_of_core,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    unnameable_types,
    unreachable_pub,
    unused_qualifications,
    reason = "generated protobuf code"
)]
pub(crate) mod proto {
    pub(crate) mod auth {
        tonic::include_proto!("muak.auth.v1");
    }
    pub(crate) mod provision {
        tonic::include_proto!("muak.provision.v1");
    }
    pub(crate) mod security {
        tonic::include_proto!("muak.security.v1");
    }
    pub(crate) mod service {
        tonic::include_proto!("muak.service.v1");
    }
    pub(crate) mod version {
        tonic::include_proto!("muak.version.v1");
    }
}

pub(crate) mod checks;
pub(crate) mod inventory;
pub(crate) mod plan;
