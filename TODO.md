## TODO

- Fix update kexec process to avoid unsigned PE binary
  - Generate a persistent signing key
  - Modify kernel Dockerfile to
    - Accept signing key/cert as build secrets
    - Embed public cert in kernel's trusted keyring (CONFIG_SYSTEM_TRUSTED_KEYS)
    - Sign bzImage with sbsign after building
  - Modify GitHub Actions workflow to inject secrets into Docker build using mount=type=secret,id=...
  - Modify Makefile to support builds with signing (also handle local build)

- Support arm64 architecture
  - Support for devicetree on ARM64 in stub
  - Create kerel config for arm64
  - Add arm kernel parameters support
  - Add build in CI/CD
  - Tweak pkgs Dockerfiles

- Better error management
  - Check if there is /dev/kvm supported when starting the distro

- Enhance networking:
  - Fix order of things: no gateway = fail & no connectivity = fail
  - Add way more testing to cover every edge case
  - Support IPv6 with DHCPv6
  - Automatic failover when primary interface fails
  - Bridge migration to back-up interface
  - Recovery from degraded state (stays degraded)

- Add dynamic kernel module loading (dont forget to transfer /lib from initramfs to the real root)
  - Sign kernel modules if transferring to real root

- Better gRPC communication:
  - Create own independent project in internal/ instead of having it in internal/granola
  - Add authentication using mTLS
    - Store certificates in STATE partition & in ~/.config/muak/ on the client side
    - Handle being in insecure mode (not having certificates) when in maintenance mode with --insecure flag on certain
      commands like muak disks
  - Add permission management for different users using RBAC like system

- Add to maintenance mode:
    - Use config.toml that is a required parameter in muak install to install declaratively the system
      - muak gen-config to generate a config template
      - Static IP configuration
      - DNS configuration
      - Gateway configuration
      - Interface configuration
    - Configure secure boot keys

- Disk Manager Service:
  - LUKS encryption/decryption
  - Quota enforcement (per-VM limits) with btrfs qgroups:
    - Set size limits on subvolumes (each VM disk can be a subvolume)
    - Use btrfs qgroup limit to enforce hard limits
    - Monitor usage with btrfs qgroup show
  - Path isolation (VMs can't access other VM disks)
    - Each VM disk is a separate subvolume
    - Subvolumes can be mounted independently at different paths and act as independent filesystem trees
  - Integrity verification
    - Automatically computes and verifies checksums for all data blocks
    - Uses CRC32C by default
    - Detects silent data corruption automatically
    - Can use btrfs scrub to verify integrity of all data
  - Copy-on-Write disk creation from templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM

- Allow user to change kernel parameters on the fly before rebooting
  - Handle normal/custom kernel parameters inspired by Talos [here](https://github.com/siderolabs/talos/blob/66c01a706f0b1dba88e30dbc1781d7fb7ef57756/website/content/v1.12/reference/kernel.md)
    - muak.port = grpc server port

- Better logging with tracing:
  - tracing::info!(component = "vm", vm_id = %vm_id, "Starting VM");
  - clean up debug logs
  - move logging into shared module

- Add e2e testing:
  - Unit tests
  - Mock system calls etc

- Handle different cli vs server versions for gRPC client/server compatibility

- Remove anyhow and use standard Rust error handling with strong typing
- Simple secure boot support with sbctl or native implementation
- Add TPM measurements in stub
- Add supervision tree for critical services like gRPC server
- Create a TUI interface to display critical system information

- Add a web interface for easier management
- Orchestrator for multipe node cluster to manage VMs when one node fails or updates, like Kubernetes but for VMs
- Add custom hypervisor using the rust-vmm crates for better performance and control
