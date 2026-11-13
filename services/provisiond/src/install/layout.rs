//! Platform layout planning driven by the boot image's disk plan.

use ::disk::convert;
use ::disk::document::Document;
use ::disk::plan::Plan;
use ::disk::role::Role;
use anyhow::{Result, bail};

// Builds the system-disk plan from the boot image's disk plan, rejecting a
// document without the ESP, STATE or EPHEMERAL partition.
pub(super) fn plan_from_doc(doc: &Document) -> Result<Plan> {
    let plan = convert::to_plan(doc)?;

    for role in [Role::Esp, Role::State, Role::Ephemeral] {
        if !plan.partitions.iter().any(|spec| spec.role == Some(role)) {
            bail!("disk plan is missing the '{role:?}' partition");
        }
    }

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use ::disk::document::{Document, Partition};
    use ::disk::layout::Layout;

    use super::*;

    fn standard_doc() -> Document {
        convert::from_plan(&Layout::Uefi.plan()).expect("doc from plan")
    }

    fn replace(doc: &Document, partition: &Partition) -> Document {
        let mut partitions: Vec<_> = doc
            .partitions()
            .iter()
            .filter(|existing| existing.role != partition.role)
            .cloned()
            .collect();
        partitions.push(partition.clone());

        Document::new(doc.wipe(), partitions)
    }

    #[test]
    fn every_partition_stays_on_the_single_plan() {
        // ARRANGE
        let doc = standard_doc();

        // ACT
        let plan = plan_from_doc(&doc).expect("plan");

        // ASSERT
        assert!(plan.wipe);
        assert_eq!(plan.partitions.len(), 3);
        assert_eq!(
            plan.partitions
                .iter()
                .map(|spec| spec.role)
                .collect::<Vec<_>>(),
            vec![Some(Role::Esp), Some(Role::State), Some(Role::Ephemeral)]
        );
    }

    #[test]
    fn plans_require_all_roles() {
        // ARRANGE
        let doc = Document::new(
            true,
            vec![
                convert::from_plan(&Layout::Uefi.plan())
                    .expect("doc from plan")
                    .find(Role::Esp)
                    .cloned()
                    .expect("esp spec"),
            ],
        );

        // ACT
        let result = plan_from_doc(&doc);

        // ASSERT
        let error = result.expect_err("missing roles must be rejected");
        assert!(error.to_string().contains("State"), "{error}");
    }

    #[test]
    fn plans_require_the_ephemeral_role() {
        // ARRANGE
        let standard = standard_doc();
        let doc = Document::new(
            standard.wipe(),
            standard
                .partitions()
                .iter()
                .filter(|partition| partition.role != Role::Ephemeral)
                .cloned()
                .collect(),
        );

        // ACT
        let result = plan_from_doc(&doc);

        // ASSERT
        let error = result.expect_err("a missing ephemeral partition must be rejected");
        assert!(error.to_string().contains("Ephemeral"), "{error}");
    }

    #[test]
    fn recorded_names_flow_into_the_plan() {
        // ARRANGE
        let mut doc = standard_doc();
        let state = doc.find(Role::State).cloned().expect("state");
        let mut renamed = state;
        renamed.name = "SYSTEMVOL".to_owned();
        doc = replace(&doc, &renamed);

        // ACT
        let plan = plan_from_doc(&doc).expect("plan");

        // ASSERT
        let state_spec = plan
            .partitions
            .iter()
            .find(|spec| spec.role == Some(Role::State))
            .expect("state spec");
        assert_eq!(state_spec.name, "SYSTEMVOL", "recorded names must be kept");
    }
}
