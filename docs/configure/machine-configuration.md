---
title: Machine Configuration
description: Reference for the TOML configuration file used to configure a Muak node.
section: configure
order: 1
draft: false
---

Muak is configured via a single TOML file that is applied at installation time and
updated via `muakctl update --config`.

## Full schema

```toml
[system]
# Human-readable name for this node. Included in the server TLS certificate SAN.
name = "muak"

# Block device to install onto. Must be a full disk path (e.g. /dev/sda, /dev/nvme0n1).
disk = "/dev/sda"

# OCI image reference for the installed system.
image = "ghcr.io/muak-os/installer:latest"

# Additional squashfs extension OCI images to layer on top of the base rootfs.
extensions = []

# Enable Secure Boot key enrollment during installation.
# Requires UEFI to be in Setup Mode at install time.
secureboot = false

# gRPC API port that apid listens on.
port = 50051

# NTP server hostname for time synchronization.
ntp = "pool.ntp.org"

[network]
# Enable IPv6 SLAAC (Stateless Address Autoconfiguration).
ipv6 = true

[vm]
# Automatically restart VMs that exit unexpectedly.
auto_restart = false
```

## Field reference

### `[system]`

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Node name; used as TLS SAN |
| `disk` | string | yes | Target block device path |
| `image` | string | yes | OCI installer image reference |
| `extensions` | `[]string` | no | Extension OCI image references |
| `secureboot` | bool | no | Enroll Secure Boot keys at install time |
| `port` | u16 | no | gRPC listening port (default `50051`) |
| `ntp` | string | no | NTP server hostname |

### `[network]`

| Field | Type | Required | Description |
|---|---|---|---|
| `ipv6` | bool | no | Enable IPv6 SLAAC (default `true`) |

### `[vm]`

| Field | Type | Required | Description |
|---|---|---|---|
| `auto_restart` | bool | no | Restart VMs on unexpected exit (default `false`) |

## Applying a new config

To update the config on a running node without changing the OS image:

```sh
muakctl update --config new-config.toml
```

This stages and applies the config atomically via the same kexec update path as an OS update.
See [maintenance/atomic-updates.md](../maintenance/atomic-updates.md) for details.
