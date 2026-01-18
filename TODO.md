## TODO

- Support NTP protocol for time synchronization

- Support ARM64 architecture
  - Support for device tree in stub
  - Create kernel config for ARM64 that follows KSPP recommandations
  - Add arm kernel parameters support
  - Add build in CI/CD
  - Tweak pkgs Dockerfiles

- Better CLI
  - Organize project in different files/modules
  - Add dmesg alias to logs command
  - Handle different cli vs server versions for gRPC client/server compatibility
    - Upgradable compatibility matrix
  - Create install script for users to easily install the CLI regardless of OS
  - Add reset command to factory reset the system
  - Inform the user when an update is available both for the CLI and the server

- Better error management
  - Check if there is /dev/kvm supported when starting the distro
  - Better handling of network error when installing/updating

- Better PID 1
  - Supervision tree for critical services
  - Look into Command::new of tokio instead of raw fork/exec for spawning services
  - Properly reap children in granola (conflict with installer command spawning)
  - Extract services to be file based in /run/services/

- Enhance networking:
  - Handle certificates properly using webpki-roots-certs in reqwest 0.13
  - Fix order of things: no gateway = fail & no connectivity = fail
  - Add way more testing to cover every edge case
  - Automatic failover when primary interface fails
  - Bridge migration to back-up interface
  - Recovery from degraded state (stays degraded)
  - Support custom proxy
  - Support for self-signed certificates
  - Allow creation of multiple bridges
  - Disable bridge when in maintenance mode

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
  - Allow/Disallow auto restart of VMs on system reboot

- Disk Manager Service:
  - LUKS encryption/decryption
  - Copy-on-Write disk creation for templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM to avoid duplication in /run/data/{vm_id}
  - Use Btrfs scrub to verify integrity of all data (/var issue)
  - Allow /run/data to be on a different disk than rootfs
  - Allow vm disks to be stored on a different disk than rootfs

- Allow user to change kernel parameters on the fly before rebooting
  - Handle normal/custom kernel parameters inspired by Talos [here](https://github.com/siderolabs/talos/blob/66c01a706f0b1dba88e30dbc1781d7fb7ef57756/website/content/v1.12/reference/kernel.md)
    - muak.port = gRPC server port
    - muak.dns = main DNS server (might already be in talos inspired params)

- Add e2e testing:
  - Unit tests & Integration tests
  - Mock system calls etc
  - Target 80% coverage

- Better install:
  - Only allow signed installer images to be installed
  - Add polling to see if install was successful after reboot
  - Add better feedback during install process like formatting etc.
  - Improve performance in formatting DATA partition?

- Rework the module loading to not duplicate modules in initramfs and rootfs

- Simple secure boot support using a local project to sign
- Add TPM measurements in stub
- Better stub performance after loadfile success
- Create a TUI interface to display critical system information

- Support Apple M1/M2 using Asahi Linux
- Add a web interface for easier management (in a separate product?)
- Orchestrator for multipe node cluster to manage VMs when one node fails or updates, like Kubernetes but for VMs or
  like Proxmox VE cluster management
- Add custom hypervisor using the rust-vmm crates for better performance and control
