//! Shared protobuf clients and servers for optional fleet inventory.

pub mod unix;

#[expect(
    clippy::absolute_paths,
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    clippy::arbitrary_source_item_ordering,
    clippy::as_conversions,
    clippy::missing_errors_doc,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::doc_paragraphs_missing_punctuation,
    clippy::empty_structs_with_brackets,
    clippy::excessive_nesting,
    clippy::impl_trait_in_params,
    clippy::inline_modules,
    clippy::module_name_repetitions,
    clippy::must_use_without_reason,
    clippy::std_instead_of_core,
    clippy::too_many_lines,
    unused_qualifications,
    reason = "generated protobuf code"
)]
pub mod fleet {
    tonic::include_proto!("muak.fleet.v1");
}

#[expect(
    clippy::absolute_paths,
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    clippy::arbitrary_source_item_ordering,
    clippy::as_conversions,
    clippy::missing_errors_doc,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::doc_paragraphs_missing_punctuation,
    clippy::empty_structs_with_brackets,
    clippy::excessive_nesting,
    clippy::impl_trait_in_params,
    clippy::inline_modules,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::must_use_without_reason,
    clippy::pattern_type_mismatch,
    clippy::std_instead_of_core,
    clippy::too_many_lines,
    unused_qualifications,
    reason = "generated protobuf code"
)]
pub mod auth {
    tonic::include_proto!("muak.auth.v1");
}

/// Header set exclusively by apid from the verified TLS certificate.
pub const PEER_HEADER: &str = "x-muak-client-fingerprint";
/// Maximum protobuf message size accepted by fleet adapters.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// Read-only publisher socket; no external API route exists for it.
pub const INVENTORY_SOCKET: &str = "/run/services/fleetd.inventory.sock";
