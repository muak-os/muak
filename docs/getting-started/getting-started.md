# Getting Started

This guide walks you through a full bare-metal installation of Muak from scratch: building the
image, booting from live media, installing to disk, and running your first VM.

## Prerequisites

- A machine meeting the [system requirements](system-requirements.md)
- A USB drive or virtual disk for booting the Muak ISO
- `muakctl` installed on your workstation

## Step 1 — Obtain the Muak ISO

### Build from source

Go to `https://muak.dev/download` to download pre-built ISOs.

### Write to USB

```sh
dd if=_out/muak-x86_64.iso of=/dev/sdX bs=4M status=progress && sync
```

## Step 2 — Boot in live mode

Insert the USB drive and boot the target machine. Muak starts in live mode — the OS runs entirely
in RAM with no disk writes. On first boot, Muak starts in **maintenance mode** automatically,
listening on port 50051 without requiring client certificates.

Verify the node is reachable:

```sh
muakctl --insecure --endpoint <node-ip>:50051 disks
```

This lists the available disks and confirms connectivity.

## Step 3 — Prepare a configuration file

Generate a `config.toml` for the target machine using the following command:

```sh
muakctl config generate > config.toml
```

Edit it like you wish. See [configure/machine-configuration.md](../configure/machine-configuration.md) for all options.

**Warning:** the target disk will be completely erased. Verify the disk name with `muakctl disks`
before proceeding.

## Step 4 — Install

```sh
muakctl install \
  --config config.toml \
  --insecure \
  --endpoint <node-ip>:50051
```

The installer streams progress messages. The full process takes from a few seconds to a few minutes
depending on network speed and disk performance.

On completion, `muakctl` saves the CA certificate, your client certificate, and the server name
to `~/.config/muak/config.toml` as a new context. The node reboots automatically.

## Step 5 — Connect to the installed node

After reboot, the node runs with full mTLS and RBAC active. Your saved credentials are used
automatically:

```sh
muakctl disks
muakctl security state
```

## Step 6 — Upload a VM kernel and create a VM

```sh
# Upload a Linux kernel image
muakctl vm upload --file /path/to/bzImage

# Create a VM (this allocates disk and registers the VM)
muakctl vm create \
  --name alpine-vm \
  --cpus 2 \
  --memory 1024 \
  --kernel /run/data/<vm_id>/bzImage \
  --root-disk-size 8192 \
  --hypervisor qemu

# Start the VM
muakctl vm start <vm_id>

# Verify it is running
muakctl vm list
```

## Step 7 — Watch the VM serial console

```sh
muakctl vm logs <vm_id> --tail 100
```

## Next steps

- [configure/machine-configuration.md](../configure/machine-configuration.md) — tune the node config
- [virtual-machines/lifecycle.md](../virtual-machines/lifecycle.md) — full VM lifecycle reference
- [security/certificate-authorities.md](../security/certificate-authorities.md) — add more users
- [maintenance/atomic-updates.md](../maintenance/atomic-updates.md) — update the OS atomically
