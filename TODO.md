## TODO

- Properly support arm64 architecture
  - Support for devicetree on ARM64 in stub

- Better error management
  - Check if there is /dev/kvm supported when starting the distro

- Better networking support:
  - Handle if no interface up when booting granola
  - Handle multiple ethernet interfaces
  - Handle interface hotplugging and disconnection
  - Static IP configuration
  - DNS configuration
  - Network interface management (bring up/down interfaces)

- Add authentication of gRPC API
- Simple secure boot support with sbctl or native implementation
- Add to maintenance mode:
    - Configure networking using the CLI: muak network set --interface <interface> --dhcp|--static
      <ip/cidr> --gateway <gateway> --dns <dns1,dns2,...>
    - Configure secure boot keys

- Disk Manager Service:
  - LUKS encryption/decryption
  - Quota enforcement (per-VM limits) with btrfs qgroups:
    - Set size limits on subvolumes (each VM disk can be a subvolume)
    - Use btrfs qgroup limit to enforce hard limits
    - Monitor usage with btrfs qgroup show
  - Path isolation (VMs can't access other VM disks)
    - Each VM disk is a separate subvolume
    - Subvolumes can be mounted independently at different paths
    - You can use permissions and mount namespaces to prevent cross-VM access
    - Subvolumes act as independent filesystem trees
  - Integrity verification (SHA256 checksums)
    - Automatically computes and verifies checksums for all data blocks
    - Uses CRC32C by default, but you can enable stronger checksums
    - Detects silent data corruption automatically
    - Can use btrfs scrub to verify integrity of all data
  - Copy-on-Write disk creation from templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM
  - Disk lifecycle management

- Automatically update the distro using ostree or similar technology with a simple CLI command: muak update

- Allow user to change kernel parameters on the fly before rebooting
  - Use kexec with /run/uki/kernel and /run/uki/cmdline.txt

- Better logging with tracing:
  - tracing::info!(component = "vm", vm_id = %vm_id, "Starting VM");

- Security features in stub:
  - Add signature verification
  - Add TPM measurements

- Add supervision tree for critical services like gRPC server
- Create a TUI interface to display critical system information
- Add a web interface for easier management
- Add custom hypervisor using the rust-vmm crates for better performance and control
