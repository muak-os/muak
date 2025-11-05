## TODO

- Better networking support:
  - Handle if no interface up when booting granola
  - Handle multiple ethernet interfaces
  - Handle interface hotplugging and disconnection
  - Static IP configuration
  - DNS configuration
  - Network interface management (bring up/down interfaces)
- Add authentication of gRPC API
- Simple secure boot support with sbctl or native implementation
- Add maintenance mode when the user first boot the system
    - In maintenance mode, you can install the distro using the CLI: muak install --target <disk>
    - You can check installation status using: muak status
    - You can also configure networking using the CLI: muak network set --interface <interface> --dhcp|--static
      <ip/cidr> --gateway <gateway> --dns <dns1,dns2,...>
    - Configure secure boot keys
- Copy UKI from ISO to installed EFI partition during installation
- Mount and use installed STATE partition for persistent configuration
- Disk Manager Service:
  - LUKS encryption/decryption
  - Quota enforcement (per-VM limits)
  - Path isolation (VMs can't access other VM disks)
  - Integrity verification (SHA256 checksums)
  - Copy-on-Write disk creation from templates
  - Disk lifecycle management
- Better logging with tracing:
    - tracing::info!(component = "vm", vm_id = %vm_id, "Starting VM");
- Add supervision tree for critical services like gRPC server
- Automatically update the distro using ostree or similar technology with a simple CLI command: muak update
- Create a TUI interface to display critical system information
- Add a web interface for easier management
- Add custom hypervisor using the rust-vmm crates for better performance and control

