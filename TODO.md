# TODO

- gh release in CI

- Support RISC-V architecture
  - https://github.com/rust-lang/rust/issues/148920
  - https://github.com/llvm/llvm-project/pull/148045
  - https://github.com/MicrosoftDocs/win32/pull/2075

- Move toolchain to another repo?
- Remove latest tag support

- Fix secureboot not enrolling properly in edk2 on real hardware.
- Allow secureboot to go from true to false

- Fix provisiond failing when preparing a second update in one daemon lifetime: `wizard::config::configure()` is a process-global `OnceLock` that errors with "config already configured" on retry — make it reconfigurable (`RwLock`) so failed prepares don't need a reboot before retrying

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
  - Parallel preflights
  - Allow for easier support of cloud providers like AWS, GCP etc with correct format & codec output.
  - Add SBOM precursor generation for each artifact
  - Allow customizing kernel parameters?
  - Bring your own secure boot keys?
  - Parametize secure boot (for easier platform support)

- Enchance `koci`:
  - Support custom HTTP proxy
  - Support for self-signed certificates

- Sign extensions and verify them for better supply chain security
- Allow for "community extensions" that are still usable with a warning about security risks

- Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
- Deterministic simulation tests?

- Enchance `workloadd`:
  - Allow ISO images for VMS
  - Rework commands to be pass through to the hypervisor
  - Make it an extension
  - Support for containers like LXC, OCI and Kubernetes node

- Support Apple M series processor chips using Asahi Linux kernel patches and m1n1 bootloader

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

- Update firmware with a simple `muakctl update --firmware` command (like `fwupd`)
  - Support for multiple vendors and platforms
  - Support for rollback in case of failure

- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design (could also manage secure boot key when TPM not supported)
