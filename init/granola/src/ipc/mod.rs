pub(crate) mod log;
pub(crate) mod process;
pub(crate) mod services;

// The module below contains machine-generated protobuf/tonic code, so the
// strict workspace lints are expected here rather than in the generated output.
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
    pub(crate) mod process {
        tonic::include_proto!("muak.process.v1");
    }
    pub(crate) mod log {
        tonic::include_proto!("muak.log.v1");
    }
    pub(crate) mod service {
        tonic::include_proto!("muak.service.v1");
    }
}
