## TODO

- Allow for /run/data to be on a different disk

- Support arm64 architecture
  - Support for devicetree in stub
  - Create kernel config for arm64
  - Add arm kernel parameters support
  - Add build in CI/CD
  - Tweak pkgs Dockerfiles

- Properly structure the CLI

- Better error management
  - Check if there is /dev/kvm supported when starting the distro
  - Better handling of network error when installing/updating

- Better support for services
  - Supervision tree for critical services

- Enhance networking:
  - Fix order of things: no gateway = fail & no connectivity = fail
  - Add way more testing to cover every edge case
  - Support IPv6 with DHCPv6
  - Automatic failover when primary interface fails
  - Bridge migration to back-up interface
  - Recovery from degraded state (stays degraded)
  - Support custom proxy
  - Allow creation of multiple bridges

- Better gRPC communication:
  - Add authentication using mTLS
    - Store certificates in STATE partition & in ~/.config/muak/ on the client side
    - Allow some commands like listing disks in maintenance mode without authentication
  - Add permission management for different users using RBAC like system

- Declarative system configuration:
  - Better configuration for the network
    - Static IP configuration
    - DNS configuration
    - Gateway configuration
    - Interface configuration
    - Proxy configuration

- Disk Manager Service:
  - LUKS encryption/decryption
  - Copy-on-Write disk creation from templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM to avoid duplication in /run/data/{vm_id}
  - Use Btrfs scrub to verify integrity of all data (/var issue)

- Allow user to change kernel parameters on the fly before rebooting
  - Handle normal/custom kernel parameters inspired by Talos [here](https://github.com/siderolabs/talos/blob/66c01a706f0b1dba88e30dbc1781d7fb7ef57756/website/content/v1.12/reference/kernel.md)
    - muak.port = gRPC server port
    - muak.dns = main DNS server (might already be in talos inspired params)

- Add e2e testing:
  - Unit tests
  - Mock system calls etc
  - Target 80% coverage

- Handle different cli vs server versions for gRPC client/server compatibility

- Rework the module loading to not duplicate modules in initramfs and rootfs

- Remove anyhow and use standard Rust error handling with strong typing
- Simple secure boot support with sbctl or native implementation
- Add TPM measurements in stub
- Create a TUI interface to display critical system information

- Support Apple M1/M2 using Asahi Linux
- Add a web interface for easier management
- Orchestrator for multipe node cluster to manage VMs when one node fails or updates, like Kubernetes but for VMs
- Add custom hypervisor using the rust-vmm crates for better performance and control
