# TODO

- Fix CI caching

- Support for Raspbnerry Pi like devices using .img installation

- Support for containers like LXC and OCI

- Add ARM e2e tests

- Better CLI
  - Create install script for users to easily install the CLI regardless of OS
  - Create a dashboard command with a TUI to display critical system information
  - Add extensions to the CLI itself

- Enhance `networkd`:
  - Configurable failover when primary interface fails
    - Bridge migration to back-up interface
  - Recovery from degraded state (stays degraded)
  - Support custom proxy
  - Support for self-signed certificates
  - Support air gap connectivity check
  - Remove connectivity check

- Target 80% coverage using unit & integration tests
- Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
- Deterministic simulation tests?

- Enchance `vmd`:
  - Allow for ISO images for VMS
  - Rework commands to be pass through to the hypervisor
  - Make it an extension

- Better logging across the codebase with kmsg, --debug flag in journal to print a lot more

- Improve security:
  - Add SElinux & enforce
  - Clean as much crate dependencies as possible to reduce attack surface and maintenance cost
  - Generate SBOM (https://github.com/rust-lang/rfcs/pull/3553)
  - Sign extensions and verify them in `imager` for better supply chain security & allow for "community extensions" that are still usable with a warning about security risks

- Orchestrator for multipe node cluster to manage VMs, like Kubernetes but for VMs or like Proxmox VE cluster management
  - WireGuard tunnel?
  - Service accounts based on auth TOFU we have.

- Support Apple M series processor chips
- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design (could also manage secure boot key when TPM not supported)
- Make the VM themselves declarative
- Add custom hypervisor using the rust-vmm crates for better performance and control
