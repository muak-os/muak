## TODO

- Support NTP protocol for time synchronization

- Support for Raspbnerry Pi like devices using .img installation

- Better CLI
  - Add dmesg alias to logs command
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

- Declarative system configuration:
  - Configure NTP server in system config
  - Better configuration for the network
    - Static IP configuration
    - DNS configuration
    - Gateway configuration
    - Interface configuration
    - Proxy configuration

- Enhance sysconfig shared lib:
  - Config versioning with tracking of changes over time
  - **Allow config hot reload (SIGHUP or something)**

- Enhance `apid`
  - Rate limit auth routes

- Disk Manager Service:
  - Add support in internal/init to allow for e2e remote unlocking using gRPC or some other way (Tang like?)
  - Copy-on-Write disk creation for templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM to avoid duplication
  - Use Btrfs scrub to verify integrity of all data (/var issue)
  - **Allow /run/data to be on a different disk than rootfs**

- Add e2e testing:
  - Unit tests & Integration tests
  - Mock system calls etc
  - Target 80% coverage
  - Chaos engineering tests for networking failures, disk failures, service failures etc. (cargo-mutants)
  - Deterministic simulation tests?

- Better install:
  - Only allow signed installer images to be installed
  - Improve performance in formatting DATA partition?

- Clean as much crate dependencies as possible to reduce attack surface and maintenance cost
  - Generate SBOM
  - Consolidate crypto crates and primitives to `internal/shared/crypto` if possible

- Support for TPM2:
  - Add TPM PCR#11 measurements in stub
  - Prefer TPM2 backend when available to store disk encryption keys

- Support for containers like LXC and Docker

- Support Apple M1/M2 using Asahi Linux
- Add a web interface for easier management (in a separate product easily installable with a golden image?) style with
  Swiss Web Design
- Orchestrator for multipe node cluster to manage VMs when one node fails or updates, like Kubernetes but for VMs or
  like Proxmox VE cluster management
- Make the VM themselves declarative
- Add custom hypervisor using the rust-vmm crates for better performance and control
