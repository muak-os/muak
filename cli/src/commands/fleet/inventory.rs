//! Filter and display the manager's latest retained inventory.

use std::io::Write;

use anyhow::{Result, ensure};
use fleet_api::fleet::{NodeRecord, VmSummary};
use serde_json::json;

use super::{Format, Options};

pub(super) const VM_STATES: [&str; 7] = [
    "unspecified",
    "created",
    "starting",
    "running",
    "stopping",
    "stopped",
    "failed",
];

#[derive(Clone, Copy)]
pub(super) enum View<'a> {
    Nodes(Option<&'a str>),
    Vms(Option<&'a str>),
}

pub(super) fn write(
    out: &mut dyn Write,
    nodes: &[NodeRecord],
    options: &Options,
    view: View<'_>,
) -> Result<()> {
    if let Some(id) = options.node.as_deref() {
        ensure!(
            nodes.iter().any(|node| node.node_id == id),
            "No fleet node with UUID '{id}'"
        );
    }
    let selected = nodes.iter().filter(|node| {
        matches_filter(options.node.as_deref(), &node.node_id)
            && matches_filter(options.freshness.as_deref(), freshness(node))
    });
    match view {
        View::Nodes(status) => {
            let rows = selected.filter(|node| matches_filter(status, &node.status));
            write_nodes(out, rows, options.output)
        }
        View::Vms(state) => {
            let rows = selected
                .flat_map(vm_rows)
                .filter(|row| matches_filter(state, vm_state(row.1.state)));
            write_vms(out, rows, options.output)
        }
    }
}

fn matches_filter(filter: Option<&str>, value: &str) -> bool {
    filter.is_none_or(|expected| expected == value)
}

fn freshness(node: &NodeRecord) -> &'static str {
    if node.stale { "stale" } else { "fresh" }
}

fn vm_state(value: i32) -> &'static str {
    usize::try_from(value)
        .ok()
        .and_then(|index| VM_STATES.get(index).copied())
        .unwrap_or("unspecified")
}

fn vm_rows(node: &NodeRecord) -> impl Iterator<Item = (&NodeRecord, &VmSummary)> {
    node.snapshot
        .iter()
        .flat_map(|snapshot| &snapshot.vms)
        .map(move |vm| (node, vm))
}

fn write_nodes<'a>(
    out: &mut dyn Write,
    rows: impl Iterator<Item = &'a NodeRecord>,
    format: Format,
) -> Result<()> {
    match format {
        Format::Table => {
            writeln!(out, "NODE\tHOST\tSTATUS\tFRESHNESS\tAGE(s)\tCERTIFICATE")?;
            for node in rows {
                let age = node
                    .observation_age_secs
                    .map_or_else(|| "never".to_owned(), |age| age.to_string());
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    node.node_id,
                    node.hostname.escape_default(),
                    node.status,
                    freshness(node),
                    age,
                    node.fingerprint
                )?;
            }
        }
        Format::Json => {
            let data: Vec<_> = rows
                .map(|node| {
                    json!({
                        "node_id": node.node_id,
                        "hostname": node.hostname,
                        "status": node.status,
                        "freshness": freshness(node),
                        "observation_age_secs": node.observation_age_secs,
                        "certificate_fingerprint": node.fingerprint,
                    })
                })
                .collect();
            writeln!(out, "{}", serde_json::to_string(&data)?)?;
        }
    }
    Ok(())
}

fn write_vms<'a>(
    out: &mut dyn Write,
    rows: impl Iterator<Item = (&'a NodeRecord, &'a VmSummary)>,
    format: Format,
) -> Result<()> {
    match format {
        Format::Table => {
            writeln!(out, "NODE\tVM\tNAME\tSTATE\tFRESHNESS")?;
            for (node, vm) in rows {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}",
                    node.node_id,
                    vm.vm_id,
                    vm.name.escape_default(),
                    vm_state(vm.state),
                    freshness(node)
                )?;
            }
        }
        Format::Json => {
            let data: Vec<_> = rows
                .map(|row| {
                    json!({
                        "node_id": row.0.node_id,
                        "hostname": row.0.hostname,
                        "vm_id": row.1.vm_id,
                        "name": row.1.name,
                        "state": vm_state(row.1.state),
                        "freshness": freshness(row.0),
                        "observation_age_secs": row.0.observation_age_secs,
                    })
                })
                .collect();
            writeln!(out, "{}", serde_json::to_string(&data)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use fleet_api::fleet::NodeSnapshot;
    use serde_json::Value;

    use super::*;
    use crate::commands::fleet::Action;
    use crate::{Cli, Commands};

    fn node(id: &str, stale: bool, observation_age_secs: Option<u64>) -> NodeRecord {
        NodeRecord {
            node_id: id.to_owned(),
            hostname: format!("host-{id}"),
            status: if observation_age_secs.is_some() {
                "approved"
            } else {
                "pending"
            }
            .to_owned(),
            stale,
            observation_age_secs,
            snapshot: observation_age_secs.map(|_| NodeSnapshot {
                hostname: format!("host-{id}"),
                vms: vec![
                    VmSummary {
                        vm_id: format!("vm-{id}"),
                        name: "build\nworker".to_owned(),
                        state: 3,
                    },
                    VmSummary {
                        vm_id: format!("stopped-{id}"),
                        name: "stopped".to_owned(),
                        state: 5,
                    },
                ],
            }),
            ..NodeRecord::default()
        }
    }

    fn cli_output(args: &[&str], nodes: &[NodeRecord]) -> Result<String> {
        let cli =
            Cli::try_parse_from(["muakctl", "fleet"].into_iter().chain(args.iter().copied()))?;
        let mut out = Vec::new();
        if let Commands::Fleet {
            action: Action::Nodes { options, status },
        } = cli.command
        {
            write(&mut out, nodes, &options, View::Nodes(status.as_deref()))?;
        } else if let Commands::Fleet {
            action: Action::Vms { options, state },
        } = cli.command
        {
            write(&mut out, nodes, &options, View::Vms(state.as_deref()))?;
        } else {
            anyhow::bail!("expected a fleet inventory command");
        }
        Ok(String::from_utf8(out)?)
    }

    #[test]
    fn vm_filters_compose_and_json_carries_observation_age() {
        // ARRANGE
        let nodes = [node("a", false, Some(3)), node("b", true, Some(62))];

        // ACT
        let output = cli_output(
            &[
                "vms",
                "--node",
                "a",
                "--freshness",
                "fresh",
                "--state",
                "running",
                "-o",
                "json",
            ],
            &nodes,
        )
        .unwrap();
        let empty = cli_output(
            &["vms", "--node", "b", "--freshness", "fresh", "-o", "json"],
            &nodes,
        )
        .unwrap();

        // ASSERT
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap(),
            json!([{
                "node_id": "a",
                "hostname": "host-a",
                "vm_id": "vm-a",
                "name": "build\nworker",
                "state": "running",
                "freshness": "fresh",
                "observation_age_secs": 3_u64,
            }])
        );
        assert_eq!(serde_json::from_str::<Value>(&empty).unwrap(), json!([]));
    }

    #[test]
    fn lists_pending_nodes_and_reports_an_unknown_target() {
        // ARRANGE
        let nodes = [node("a", true, Some(62)), node("b", true, None)];

        // ACT
        let output = cli_output(
            &[
                "nodes",
                "--status",
                "pending",
                "--freshness",
                "stale",
                "-o",
                "json",
            ],
            &nodes,
        )
        .unwrap();
        let error = cli_output(&["vms", "--node", "host-b"], &nodes).unwrap_err();

        // ASSERT
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap(),
            json!([{
                "node_id": "b",
                "hostname": "host-b",
                "status": "pending",
                "freshness": "stale",
                "observation_age_secs": null,
                "certificate_fingerprint": "",
            }])
        );
        assert!(
            error
                .to_string()
                .contains("No fleet node with UUID 'host-b'")
        );
    }

    #[test]
    fn default_table_keeps_the_existing_rows_and_headers() {
        // ARRANGE
        let nodes = [node("a", false, Some(3))];

        // ACT
        let output = cli_output(&["vms"], &nodes).unwrap();

        // ASSERT
        assert_eq!(
            output,
            concat!(
                "NODE\tVM\tNAME\tSTATE\tFRESHNESS\n",
                "a\tvm-a\tbuild\\nworker\trunning\tfresh\n",
                "a\tstopped-a\tstopped\tstopped\tfresh\n",
            )
        );
    }
}
