# TODO

- Enhance `e2e` tests
  - Support ARM & RISC-V architecture
  - Test true networking with IPv6

- Increase audit of the system:
  - Allow forwarding kernel logs to external monitoring systems
  - Audit every API call and allow user to review them

- Better `cli`:
  - Create install script for users to easily install the CLI regardless of OS
  - Add extensions to the CLI itself?
  - Allow for `muakctl update --overlays` to update overlays files (full reboot instead of kexec?)
  - E2E encryption between CLI and the daemon?

- Enhance `wizard`:
  - Remove preflights OCI pull and use annotations
  - Add manifest release OCI instead of hardcoded
  - Add extension catalog in resolver (using OCI image?)
  - Parallel preflights
  - Add SBOM precursor generation for each artifact

- Enchance `koci`:
  - Support for proper auth token for pull following the OCI standard
  - Support custom HTTP proxy
  - Support for self-signed certificates
  - Remove HTTP only support
  - Sign extensions and verify them for better supply chain security
    - Allow for "community extensions" that are still usable with a warning about security risks

- In `provisiond` save koci cache in /run/state and clean cache on updates if it's stale (more than 3 weeks old for example)

- Target 80% coverage using unit & integration tests
- Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
- Deterministic simulation tests?

- Better code quality:
  - Fix all clippy warnings
  - Remove `ring` crate and use rust crypto project crates instead (especially in `sbolt`).
  - Consolidate crypto code instead of duplicating (maybe in a single crate?)

- Enchance `workloadd`:
  - Allow ISO images for VMS
  - Rework commands to be pass through to the hypervisor
  - Make it an extension
  - Support for containers like LXC and OCI

- Support Apple M series processor chips using Asahi Linux kernel patches and m1n1 bootloader
  - Correct partition formatting and new install workflow for Apple Silicon Macs

- Linux Kernel abstraction layer to support different kernels
  - Feature gate `libc` usage
  - Allow for no_std environments
  - Update build target to support either existing Linux target or none
    - x86_64: `x86_64-unknown-none`
    - AArch64: `aarch64-unknown-none-softfloat`
    - RISC-V 64: `riscv64gc-unknown-none-elf`

- Orchestrator for multipe node cluster to manage VMs, like Kubernetes but for VMs or like Proxmox VE cluster management
  - WireGuard tunnel?
  - Service accounts based on auth TOFU we have.

- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design (could also manage secure boot key when TPM not supported)
