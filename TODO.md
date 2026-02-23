## TODO

- Support for Raspbnerry Pi like devices using .img installation

- Better CLI
  - Handle different cli vs server versions for gRPC client/server compatibility
    - Upgradable compatibility matrix
    - Inform the user when an update is available both for the CLI and the server
  - Create install script for users to easily install the CLI regardless of OS
  - Create a dashboard command with a TUI interface to display critical system information

- Add journalctl like support in PID 1 to monitor logs of all services

- Enhance networking:
  - Fix order of things: no gateway = fail & no connectivity = fail
  - Automatic failover when primary interface fails
  - Bridge migration to back-up interface
  - Recovery from degraded state (stays degraded)
  - Support custom proxy
  - Support for self-signed certificates
  - Allow creation of multiple bridges
  - Disable bridge when in maintenance mode
  - Full declarative config for network interfaces, bridges etc

- Enhance sysconfig shared lib:
  - Config versioning with tracking of changes over time
  - **Allow config hot reload (SIGHUP or something)**

- Disk Manager Service:
  - Copy-on-Write disk creation for templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM to avoid duplication
  - Use Btrfs scrub to verify integrity of all data
  - **Allow /run/data to be on a different disk than rootfs**

- Add e2e testing:
  - Unit tests & Integration tests
  - Mock system calls etc
  - Target 80% coverage
  - Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
  - Deterministic simulation tests?

- Only allow signed installer images to be installed

- Allow update with config file to update system config

- Clean as much crate dependencies as possible to reduce attack surface and maintenance cost
  - Generate SBOM
  - Consolidate crypto crates and primitives to `internal/shared/crypto` if possible

- Support for TPM2:
  - Add TPM PCR#11 measurements in stub (.pcrsig & .pcrkey sections in UKI file)
  - Prefer TPM2 backend when available to store disk encryption keys

- Support for containers like LXC and Docker

- Support Apple M1/M2 using Asahi Linux
- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design (could also manage secure boot key when TPM not supported)
- Orchestrator for multipe node cluster to manage VMs when one node fails or updates, like Kubernetes but for VMs or
  like Proxmox VE cluster management
- Make the VM themselves declarative
- Add custom hypervisor using the rust-vmm crates for better performance and control
