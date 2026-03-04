---
title: Factory Reset
description: How to wipe all persistent state from a Muak node while preserving the boot image.
section: maintenance
order: 3
draft: false
---

A factory reset wipes all persistent data from a Muak node by deleting and re-formatting the STATE and DATA partitions. The EFI partition and its UKI are left intact — the node will still boot into live mode after the reset.

## What Gets Erased

| Erased | Not erased |
|--------|------------|
| All configuration (`/run/state/`) | UKI binary on the EFI partition |
| All secrets, CA keys, LUKS tokens | The kernel and initramfs |
| All authorized user certificates | |
| All VM disk images (`/run/data/`) | |
| All VM state | |
| Update history and rollback records | |
| LUKS2 headers (encryption keys) | |

After a factory reset, the node is effectively in the same state as a freshly booted live image. It must be re-installed before it can be used.

## Performing a Factory Reset

```
muakctl system reset
```

`muakctl` will prompt for confirmation before proceeding. To skip the prompt:

```
muakctl system reset --force
```

This action requires the `admin` permission.

## Notes

- Factory reset is **irreversible**. All VM data, configuration, and secrets are permanently destroyed.
- The LUKS2 headers are deleted as part of partition deletion, which destroys the encryption keys. Even if the raw disk sectors are recovered, they cannot be decrypted without the keys.
- Secure Boot keys enrolled in the firmware EFI variable store are **not** removed by factory reset. To remove them, you must enter the firmware setup utility manually.
