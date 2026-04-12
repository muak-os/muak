# TODO

- Fix CI caching

- Support for containers like LXC and OCI

- Add ARM e2e tests

- Pin extension version to muak's version to avoid breaking changes and allow for better compatibility management

- Better CLI
  - Create install script for users to easily install the CLI regardless of OS
  - Add extensions to the CLI itself

- Enhance `networkd`:
  - Configurable failover when primary interface fails
    - Bridge migration to back-up interface
  - Recovery from degraded state (stays degraded)
  - Test IPv6 with e2e tests

- Enchance `imager`:
  - Support custom http proxy
  - Support for self-signed certificates
  - Remove http support
  - Sign extensions and verify them in `imager/ramune` for better supply chain security & allow for "community extensions" that are still usable with a warning about security risks

- Target 80% coverage using unit & integration tests
- Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
- Deterministic simulation tests?

- Enchance `vmd`:
  - Allow for ISO images for VMS
  - Rework commands to be pass through to the hypervisor
  - Make it an extension & rename to workloadd

- Generate SBOM (https://github.com/rust-lang/rfcs/pull/3553)

- Orchestrator for multipe node cluster to manage VMs, like Kubernetes but for VMs or like Proxmox VE cluster management
  - WireGuard tunnel?
  - Service accounts based on auth TOFU we have.

- Support for Raspberry Pi like devices using .img installation
- Support RISC-V architecture
- Support Apple M series processor chips

- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design (could also manage secure boot key when TPM not supported)
- Make the VM themselves declarative?
