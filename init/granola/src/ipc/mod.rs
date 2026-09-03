pub mod log;
pub mod process;

// The module below contains machine-generated protobuf/tonic code, so the
// strict workspace lints are expected here rather than in the generated output.
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
    pub mod process {
        tonic::include_proto!("muak.process.v1");
    }
    pub mod log {
        tonic::include_proto!("muak.log.v1");
    }
}
