# Configuration History

Every time the system configuration changes — on install, update, or rollback — Muak records a history entry. This lets you see what changed, when, and by whom, and retrieve exact configuration snapshots from any point in time.

## What Is Recorded

Each history entry stores:

| Field | Description |
|-------|-------------|
| `timestamp` | Unix timestamp (seconds) of the change |
| `update_id` | Update ID of the form `update-<unix_timestamp>` |
| `author` | SHA-256 fingerprint of the mTLS client certificate that triggered the change |
| `change_kind` | One of: `install`, `update`, `rollback` |

In addition to the metadata, a full TOML snapshot of the configuration at that point in time is stored alongside each entry and can be retrieved via the API.

## Listing History

To see the most recent configuration changes:

```sh
muakctl config history
```

This shows the last 10 entries by default. To retrieve more:

```sh
muakctl config history --limit 50
muakctl config history -l 50
```

## Diffing Changes

To see exactly which fields changed between two configurations, use `muakctl config diff`. The most common usage is to inspect a specific update against its automatic predecessor — the entry that was active immediately before it:

```sh
muakctl config diff --to <update-id>
```

Muak looks up the predecessor automatically from the history, so you only need to supply the update you are curious about.

To compare any two specific updates explicitly:

```sh
muakctl config diff --from <update-id-a> --to <update-id-b>
```

Note: `--from` without `--to` is an error. `--to` is always required.

## Exporting the Current Configuration

To download the live configuration currently running on the node to a local file:

```sh
muakctl config export
```

This writes a timestamped file named `config-<timestamp>.toml` in the current directory. This is useful as a starting point when preparing a new update or as a point-in-time backup.
