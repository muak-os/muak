# Philosophy

Muak is built around a small set of design principles that inform every decision — from the choice of hypervisors to the absence of a shell.

## Immutability

The operating system is read-only. There is no package manager, no `apt install`, no configuration drift. The root filesystem is part of the initramfs, loaded entirely into memory. Every node running the same image is byte-for-byte identical.

This has consequences that are intentional:

- You cannot patch a running Muak node in place. You prepare an update, apply it, and the system reboots into the new image atomically.
- There is no state leaking from one boot to the next except what is explicitly persisted to the STATE partition.
- Debugging "it works on my node but not yours" becomes nearly impossible, which is the point.

## API-Only Surface

Muak has no interactive shell, no SSH, and no local console access. Every operation goes through the gRPC API.

This is not a limitation — it is the design. An API surface is:

- **Auditable**: every action is associated with a certificate fingerprint.
- **Automatable**: `muakctl` is just a gRPC client; any tooling that speaks gRPC can drive Muak.
- **Constrained**: the API does exactly what it documents. There is no escape hatch.

The tradeoff is that operations that would normally be a one-liner in a shell require an explicit API call. This is acceptable because Muak is not a general-purpose server — it does one thing (run VMs) and exposes exactly the operations needed for that.

## Minimal by Definition

Muak does not include:

- A C standard library in userspace (musl is linked statically into each binary)
- A init system with a dependency graph (granola supervises a fixed set of daemons)
- A network configuration framework (networkd handles exactly the network model Muak needs)
- A container runtime
- A web interface
- A multi-node orchestrator

Each of these omissions is deliberate. Every piece of software in the system is a potential attack surface, a source of bugs, and a maintenance burden. Muak's bet is that a smaller, well-understood system is more reliable than a general-purpose one.

## Security as a First-Class Concern

Security is not bolted on — it is part of the boot sequence:

- **Secure Boot** with self-generated keys ensures only signed images boot.
- **TPM2 PCR sealing** ensures disk encryption keys are only accessible to the exact image that was installed.
- **mTLS everywhere** means every API call requires a certificate; there are no bearer tokens, no passwords.
- **RBAC** means capabilities are assigned per certificate, not per user role or group.

The default posture is deny. Capabilities are granted explicitly and logged implicitly (via the certificate fingerprint in every request).

## Pure Rust

Every component is written in Rust, including:

- LUKS2 formatting and dm-crypt activation (no cryptsetup)
- Btrfs subvolume and quota management (no btrfs-progs)
- TPM2 seal/unseal (no tss2 userspace stack)
- UEFI Secure Boot key enrollment (no efitools)
- PE signing (no sbsign)
- GPT partitioning (no fdisk/parted)
- NTP synchronization (no ntpd/chrony)

This is not dogma. Each of these choices reduces the number of external binaries that need to be included in the initramfs, reduces attack surface, and ensures the entire codebase compiles from a single `cargo build` invocation.

## Atomic Updates via kexec

Muak updates are not applied by writing files to disk and rebooting. They are applied by:

1. Staging the new UKI to the EFI partition.
2. Calling `kexec_file_load` to load the new kernel into memory.
3. Calling `reboot(KEXEC)` to execute the new kernel without a hardware reset.

From the operator's perspective, a Muak update is a reboot. From the system's perspective, it is a surgical kernel replacement that takes effect in seconds. If the new image fails validation after the kexec boot, the system reverts to the previous image.
