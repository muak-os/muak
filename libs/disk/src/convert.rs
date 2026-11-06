//! Conversions between the executor plan and the serialized document.

use crate::document::{self, Document, Partition};
use crate::error::DiskError;
use crate::plan::{PartitionSpec, Plan};

/// Builds the document from a partition plan.
///
/// # Errors
///
/// Returns [`DiskError`] when a plan partition carries no role.
pub fn from_plan(plan: &Plan) -> Result<Document, DiskError> {
    let partitions = plan
        .partitions
        .iter()
        .map(partition_from_spec)
        .collect::<Result<Vec<_>, DiskError>>()?;

    Ok(Document::new(plan.wipe, partitions))
}

/// Converts the document back into a partition plan for the executor.
///
/// # Errors
///
/// Returns [`DiskError`] when a type GUID is not a valid UUID.
pub fn to_plan(document: &Document) -> Result<Plan, DiskError> {
    let partitions = document
        .partitions()
        .iter()
        .map(|partition| {
            let type_guid = parse_type_guid(&partition.type_guid)?;

            Ok(PartitionSpec {
                role: Some(partition.role),
                name: partition.name.clone(),
                type_guid,
                size: partition.size,
            })
        })
        .collect::<Result<Vec<_>, DiskError>>()?;

    Ok(Plan {
        wipe: document.wipe(),
        partitions,
    })
}

fn partition_from_spec(spec: &PartitionSpec) -> Result<Partition, DiskError> {
    let role = spec
        .role
        .ok_or_else(|| DiskError::Plan(format!("partition '{}' has no role", spec.name)))?;

    Ok(Partition {
        role,
        name: spec.name.clone(),
        type_guid: document::guid(&spec.type_guid),
        size: spec.size,
    })
}

fn parse_type_guid(text: &str) -> Result<[u8; 16], DiskError> {
    let parsed = uuid::Uuid::parse_str(text)
        .map_err(|e| DiskError::Plan(format!("bad type GUID '{text}': {e}")))?;

    Ok(parsed.into_bytes())
}
