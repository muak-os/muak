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
  - Create a dashboard command with a TUI interface to display critical system information

- Better PID 1
  - Better explicit restart strategy for services in supervisor (exponential backoff etc)
  - Properly reap children in granola (conflict with installer command spawning)
  - Extract services to be file based in /run/services/
  - Check if there is /dev/kvm supported when starting the distro, setting degraded system state if not

- Enhance networking:
  - Handle certificates properly using webpki-roots-certs in reqwest 0.13 or switch to ureq
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
  - Configure NTP server in system config
  - Better configuration for the network
    - Static IP configuration
    - DNS configuration
    - Gateway configuration
    - Interface configuration
    - Proxy configuration

- Enhance sysconfig shared lib:
  - Config versioning: Track config changes over time
  - Handle config error properly by going back to default in binaries using sysconfig

- Disk Manager Service:
  - LUKS encryption/decryption with libcryptsetup-rs
    - Automatic unlocking using TPM2
    - Add support in internal/init to allow for e2e remote unlocking using gRPC or some other way (Tang like?)
  - Copy-on-Write disk creation for templates
    - Btrfs snapshots create instant, space-efficient copies
    - Btrfs snapshots use COW, so only changed blocks consume space
    - Create one golden image, snapshot for each VM to avoid duplication
  - Use Btrfs scrub to verify integrity of all data (/var issue)
  - Allow /run/data to be on a different disk than rootfs
  - Allow vm disks to be stored on a different disk than rootfs
  - Replace direct call to `mkfs.btrfs` with FFI bindings or some other way to avoid spawning processes

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

- Support Apple M1/M2 using Asahi Linux
- Add a web interface for easier management (in a separate product easily installable with a golden image?)
- Orchestrator for multipe node cluster to manage VMs when one node fails or updates, like Kubernetes but for VMs or
  like Proxmox VE cluster management
- Add custom hypervisor using the rust-vmm crates for better performance and control
