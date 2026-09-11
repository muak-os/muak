//! Platform layout planning driven by the boot image's disk plan.

use ::disk::plan::Document;
use ::disk::plan::Plan;
use ::disk::role::Role;
use anyhow::{Result, bail};

/// Builds the system-disk and data-disk plans from the boot image's disk
/// plan, splitting DATA onto its own disk when the assignment is split.
///
/// # Errors
///
/// Returns an error when the document cannot be converted to a plan or is
/// missing a required role.
pub fn plans_from_doc(doc: &Document, shared_data: bool) -> Result<(Plan, Option<Plan>)> {
    let plan = doc.to_plan()?;

    for role in [Role::Esp, Role::State, Role::Data] {
        if !plan.partitions.iter().any(|spec| spec.role == Some(role)) {
            bail!("disk plan is missing the '{role:?}' partition");
        }
    }

    if shared_data {
        return Ok((plan, None));
    }

    let mut system_specs = plan.partitions.clone();
    let data_specs: Vec<::disk::plan::PartitionSpec> = system_specs
        .iter()
        .filter(|spec| spec.role == Some(Role::Data))
        .cloned()
        .collect();
    system_specs.retain(|spec| spec.role != Some(Role::Data));

    Ok((
        Plan {
            wipe: plan.wipe,
            partitions: system_specs,
        },
        Some(Plan {
            wipe: plan.wipe,
            partitions: data_specs,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use ::disk::layout::Layout;
    use ::disk::plan::{Document, Partition};

    use super::*;

    fn standard_doc() -> Document {
        Document::from_plan(&Layout::Uefi.plan()).expect("doc from plan")
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
    fn shared_assignment_keeps_data_on_the_system_plan() {
        // ARRANGE
        let doc = standard_doc();

        // ACT
        let (system, data) = plans_from_doc(&doc, true).expect("plans");

        // ASSERT
        assert!(system.wipe);
        assert_eq!(
            system.partitions.len(),
            3,
            "all partitions stay on the system disk"
        );
        assert!(
            data.is_none(),
            "shared assignment has no separate data plan"
        );
    }

    #[test]
    fn split_assignment_moves_data_to_its_own_plan() {
        // ARRANGE
        let doc = standard_doc();

        // ACT
        let (system, data) = plans_from_doc(&doc, false).expect("plans");

        // ASSERT
        assert_eq!(
            system.partitions.len(),
            2,
            "system disk keeps ESP + STATE only"
        );
        assert!(
            system
                .partitions
                .iter()
                .all(|spec| spec.role != Some(Role::Data))
        );
        let data = data.expect("split assignment must produce a data plan");
        assert_eq!(data.partitions.len(), 1, "the data disk carries DATA only");
        assert_eq!(
            data.partitions.first().expect("data spec").role,
            Some(Role::Data)
        );
    }

    #[test]
    fn plans_require_all_roles() {
        // ARRANGE
        let doc = Document::new(
            true,
            vec![
                Document::from_plan(&Layout::Uefi.plan())
                    .expect("doc from plan")
                    .find(Role::Esp)
                    .cloned()
                    .expect("esp spec"),
            ],
        );

        // ACT
        let result = plans_from_doc(&doc, true);

        // ASSERT
        let error = result.expect_err("missing roles must be rejected");
        assert!(error.to_string().contains("State"), "{error}");
    }

    #[test]
    fn recorded_names_flow_into_the_plans() {
        // ARRANGE
        let mut doc = standard_doc();
        let state = doc.find(Role::State).cloned().expect("state");
        let mut renamed = state.clone();
        renamed.name = "SYSTEMVOL".to_owned();
        doc = replace(&doc, &renamed);

        // ACT
        let (system, _) = plans_from_doc(&doc, true).expect("plans");

        // ASSERT
        let state_spec = system
            .partitions
            .iter()
            .find(|spec| spec.role == Some(Role::State))
            .expect("state spec");
        assert_eq!(state_spec.name, "SYSTEMVOL", "recorded names must be kept");
    }
}
