# TODO

- Enhance `e2e` tests
  - Support ARM architecture
  - Test true networking with IPv6

- Pin extension version to muak's version to avoid breaking changes and allow for better compatibility management

- Allow forwarding kernel logs to external monitoring system.

- Better CLI
  - Create install script for users to easily install the CLI regardless of OS
  - Add extensions to the CLI itself

- Enhance `networkd`:
  - Configurable failover when primary interface fails
    - Bridge migration to back-up interface
  - Selective drift detection for interfaces

- Enchance `koci`:
  - Support custom HTTP proxy
  - Support for self-signed certificates
  - Remove HTTP support
  - Sign extensions and verify them in `koci/ramune` for better supply chain security & allow for "community extensions" that are still usable with a warning about security risks

- Target 80% coverage using unit & integration tests
- Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
- Deterministic simulation tests?

- Enchance `vmd`:
  - Allow for ISO images for VMS
  - Rework commands to be pass through to the hypervisor
  - Make it an extension & rename to workloadd
  - Support for containers like LXC and OCI

- Generate SBOM (https://doc.rust-lang.org/cargo/reference/unstable.html#sbom)

- Orchestrator for multipe node cluster to manage VMs, like Kubernetes but for VMs or like Proxmox VE cluster management
  - WireGuard tunnel?
  - Service accounts based on auth TOFU we have.

- Support for SBCs devices using .img installation
- Support Apple M series processor chips
- Support RISC-V architecture

- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design (could also manage secure boot key when TPM not supported)
- Linux Kernel abstraction layer to support different kernels
- Make the VM themselves declarative?
