---
title: What is Muak?
description: An introduction to Muak — a minimal, immutable, API-driven Linux distribution built to run virtual machines on bare metal.
section: overview
order: 1
draft: false
---

Muak is a minimal, immutable, API-driven Linux distribution built to run virtual machines. Boot it
on bare metal and have VMs running in minutes — managed entirely through a single gRPC API secured
with mutual TLS.

## The five pillars

**API-managed.** There is no shell. There is no SSH. Every operation — install, configure, launch a
VM, update the OS, revoke a certificate — goes through a single declarative gRPC API. This
eliminates a whole class of operational complexity: no runbooks, no configuration drift, no
"snowflake" machines.

**Immutable filesystem.** The root filesystem is a read-only squashfs image. Nothing on the running
system can be accidentally (or maliciously) modified. Persistent state lives on a separate
LUKS2-encrypted Btrfs partition.

**Minimal.** Muak ships exactly what is needed to boot, manage the network, and run VMs. No package
manager, no systemd, no shells. The attack surface is as small as possible.

**Atomic.** Every configuration change or OS upgrade is applied as an atomic transaction
via kexec. If the new image fails to validate after boot, the system automatically rolls back to
the previous known-good state. A bad update can never brick the machine.

**Secure by default.** All API traffic uses mutual TLS with ECDSA P-256. Disk encryption is
automatic via LUKS2 with keys sealed to the TPM2 chip. Secure Boot is supported out of the box.

## Muak vs. general-purpose Linux

| | Muak | Traditional Linux |
|---|---|---|
| Shell access | None | bash / sh |
| Configuration | Declarative gRPC API | Files, package managers |
| Filesystem | Immutable squashfs | Mutable |
| Updates | Atomic via kexec | Package manager |
| Disk encryption | Automatic (TPM2-sealed LUKS2) | Optional, manual |
| Purpose | Running VMs | General purpose |

## Architecture at a glance

```
muakctl (CLI)
    |
    | mTLS gRPC (HTTP/2)
    v
  apid  ─── RBAC ─── mTLS termination
    |
    ├── vmd          VM lifecycle (QEMU / Cloud Hypervisor / Firecracker)
    ├── provisiond   Install, update, certificates, history
    ├── networkd     DHCP, TAP/bridge, SLAAC (internal)
    └── granola      PID 1 supervisor, logs, process list
```

For a complete architecture deep-dive, see [learn-more/architecture.md](../learn-more/architecture.md).
