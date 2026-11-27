//! Integration tests for the plan <-> document conversions.

pub mod common;

#[cfg(test)]
mod tests {
    use disk::convert;
    use disk::document::{Document, Partition};
    use disk::plan::Size;
    use disk::role::Role;

    use crate::common;

    #[test]
    fn plan_conversion_round_trips() {
        // ARRANGE
        let plan = common::uefi_plan();

        // ACT
        let doc = convert::from_plan(&plan).expect("from plan");
        let converted = convert::to_plan(&doc).expect("to plan");

        // ASSERT
        assert_eq!(converted, plan, "plan conversion must round trip");
    }

    #[test]
    fn plan_conversion_rejects_bad_type_guids() {
        // ARRANGE
        let doc = Document::new(
            true,
            vec![Partition {
                role: Role::Esp,
                name: "EFI".to_owned(),
                type_guid: "not-a-uuid".to_owned(),
                size: Size::Fill,
            }],
        );

        // ACT
        let result = convert::to_plan(&doc);

        // ASSERT
        assert!(result.is_err(), "invalid type GUIDs must be rejected");
    }
}
