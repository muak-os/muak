# System Requirements

## Hardware

| Requirement | Minimum | Notes |
|---|---|---|
| Architecture | `x86_64` or `aarch64` | — |
| CPU | Any with hardware virtualization | Intel VT-x or AMD-V required for VMs |
| RAM | 512 MB | 2 GB+ recommended for running VMs |
| Disk | 5 GB | Dedicated full disk; Muak does not coexist with other OS data |
| Firmware | UEFI | Legacy BIOS is not supported |
| KVM | `/dev/kvm` present | Required for VM execution |
| TPM2 | `/dev/tpmrm0` recommended | Required for automatic LUKS2 disk unlock; fallback is less secure |

## UEFI requirements

Muak boots as a **Unified Kernel Image (UKI)** — a single PE32+ binary containing the kernel,
initramfs, and command line. Your UEFI firmware must support booting PE images directly from the
EFI System Partition.

If you intend to use **Secure Boot**, the firmware must be in Setup Mode before installation so
that Muak can enroll its own Platform Key (PK), Key Exchange Key (KEK), and db entry.

## TPM2

Muak uses TPM2 PCR#11 to seal the LUKS2 disk encryption key during installation. At boot, `init`
unseals the key from the TPM. Hardware with TPM2 support is strongly recommended for production
deployments.

## Network

Muak requires at least one Ethernet interface for:
- Receiving DHCP (or static IP configuration)
- Allowing `muakctl` to connect to `apid` over gRPC

The gRPC API listens on port `50051` by default (configurable via `config.system.port`).

## muakctl (client machine)

The CLI client `muakctl` runs on any machine and requires:
- Linux, macOS, or Windows
- TCP access to the Muak node on the configured gRPC port
