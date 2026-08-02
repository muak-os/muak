//! gRPC service implementations for provisiond.

pub mod auth;
pub mod provision;
pub mod security;
pub mod version;

#[expect(
    clippy::absolute_paths,
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    clippy::as_conversions,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::doc_paragraphs_missing_punctuation,
    clippy::empty_structs_with_brackets,
    clippy::excessive_nesting,
    clippy::module_name_repetitions,
    clippy::pattern_type_mismatch,
    clippy::std_instead_of_core,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    reason = "generated protobuf code"
)]
pub mod proto {
    pub mod auth {
        tonic::include_proto!("muak.auth.v1");
    }
    pub mod provision {
        tonic::include_proto!("muak.provision.v1");
    }
    pub mod security {
        tonic::include_proto!("muak.security.v1");
    }
    pub mod version {
        tonic::include_proto!("muak.version.v1");
    }
}
