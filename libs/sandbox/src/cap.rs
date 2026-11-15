//! Linux capabilities and the bit each one occupies in a capability set.
//!
//! The numbering and the bit values are the ones the kernel documents, so the
//! table is written out in full instead of derived.

/// A single Linux capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Cap {
    /// `CAP_CHOWN`.
    Chown = 0,
    /// `CAP_DAC_OVERRIDE`.
    DacOverride = 1,
    /// `CAP_DAC_READ_SEARCH`.
    DacReadSearch = 2,
    /// `CAP_FOWNER`.
    Fowner = 3,
    /// `CAP_FSETID`.
    Fsetid = 4,
    /// `CAP_KILL`.
    Kill = 5,
    /// `CAP_SETGID`.
    Setgid = 6,
    /// `CAP_SETUID`.
    Setuid = 7,
    /// `CAP_SETPCAP`.
    Setpcap = 8,
    /// `CAP_LINUX_IMMUTABLE`.
    LinuxImmutable = 9,
    /// `CAP_NET_BIND_SERVICE`.
    NetBindService = 10,
    /// `CAP_NET_BROADCAST`.
    NetBroadcast = 11,
    /// `CAP_NET_ADMIN`.
    NetAdmin = 12,
    /// `CAP_NET_RAW`.
    NetRaw = 13,
    /// `CAP_IPC_LOCK`.
    IpcLock = 14,
    /// `CAP_IPC_OWNER`.
    IpcOwner = 15,
    /// `CAP_SYS_MODULE`.
    SysModule = 16,
    /// `CAP_SYS_RAWIO`.
    SysRawio = 17,
    /// `CAP_SYS_CHROOT`.
    SysChroot = 18,
    /// `CAP_SYS_PTRACE`.
    SysPtrace = 19,
    /// `CAP_SYS_PACCT`.
    SysPacct = 20,
    /// `CAP_SYS_ADMIN`.
    SysAdmin = 21,
    /// `CAP_SYS_BOOT`.
    SysBoot = 22,
    /// `CAP_SYS_NICE`.
    SysNice = 23,
    /// `CAP_SYS_RESOURCE`.
    SysResource = 24,
    /// `CAP_SYS_TIME`.
    SysTime = 25,
    /// `CAP_SYS_TTY_CONFIG`.
    SysTtyConfig = 26,
    /// `CAP_MKNOD`.
    Mknod = 27,
    /// `CAP_LEASE`.
    Lease = 28,
    /// `CAP_AUDIT_WRITE`.
    AuditWrite = 29,
    /// `CAP_AUDIT_CONTROL`.
    AuditControl = 30,
    /// `CAP_SETFCAP`.
    Setfcap = 31,
    /// `CAP_MAC_OVERRIDE`.
    MacOverride = 32,
    /// `CAP_MAC_ADMIN`.
    MacAdmin = 33,
    /// `CAP_SYSLOG`.
    Syslog = 34,
    /// `CAP_WAKE_ALARM`.
    WakeAlarm = 35,
    /// `CAP_BLOCK_SUSPEND`.
    BlockSuspend = 36,
    /// `CAP_AUDIT_READ`.
    AuditRead = 37,
    /// `CAP_PERFMON`.
    Perfmon = 38,
    /// `CAP_BPF`.
    Bpf = 39,
    /// `CAP_CHECKPOINT_RESTORE`.
    CheckpointRestore = 40,
}

impl Cap {
    /// Returns the kernel number of this capability.
    #[must_use = "the number is what the kernel syscall takes"]
    pub const fn number(self) -> u32 {
        bit(self).trailing_zeros()
    }

    /// Every Linux capability, in kernel number order.
    pub const ALL: [Self; 41] = [
        Self::Chown,
        Self::DacOverride,
        Self::DacReadSearch,
        Self::Fowner,
        Self::Fsetid,
        Self::Kill,
        Self::Setgid,
        Self::Setuid,
        Self::Setpcap,
        Self::LinuxImmutable,
        Self::NetBindService,
        Self::NetBroadcast,
        Self::NetAdmin,
        Self::NetRaw,
        Self::IpcLock,
        Self::IpcOwner,
        Self::SysModule,
        Self::SysRawio,
        Self::SysChroot,
        Self::SysPtrace,
        Self::SysPacct,
        Self::SysAdmin,
        Self::SysBoot,
        Self::SysNice,
        Self::SysResource,
        Self::SysTime,
        Self::SysTtyConfig,
        Self::Mknod,
        Self::Lease,
        Self::AuditWrite,
        Self::AuditControl,
        Self::Setfcap,
        Self::MacOverride,
        Self::MacAdmin,
        Self::Syslog,
        Self::WakeAlarm,
        Self::BlockSuspend,
        Self::AuditRead,
        Self::Perfmon,
        Self::Bpf,
        Self::CheckpointRestore,
    ];
}

pub(crate) const fn bit(cap: Cap) -> u64 {
    match cap {
        Cap::Chown => 0x0000_0000_0000_0001,
        Cap::DacOverride => 0x0000_0000_0000_0002,
        Cap::DacReadSearch => 0x0000_0000_0000_0004,
        Cap::Fowner => 0x0000_0000_0000_0008,
        Cap::Fsetid => 0x0000_0000_0000_0010,
        Cap::Kill => 0x0000_0000_0000_0020,
        Cap::Setgid => 0x0000_0000_0000_0040,
        Cap::Setuid => 0x0000_0000_0000_0080,
        Cap::Setpcap => 0x0000_0000_0000_0100,
        Cap::LinuxImmutable => 0x0000_0000_0000_0200,
        Cap::NetBindService => 0x0000_0000_0000_0400,
        Cap::NetBroadcast => 0x0000_0000_0000_0800,
        Cap::NetAdmin => 0x0000_0000_0000_1000,
        Cap::NetRaw => 0x0000_0000_0000_2000,
        Cap::IpcLock => 0x0000_0000_0000_4000,
        Cap::IpcOwner => 0x0000_0000_0000_8000,
        Cap::SysModule => 0x0000_0000_0001_0000,
        Cap::SysRawio => 0x0000_0000_0002_0000,
        Cap::SysChroot => 0x0000_0000_0004_0000,
        Cap::SysPtrace => 0x0000_0000_0008_0000,
        Cap::SysPacct => 0x0000_0000_0010_0000,
        Cap::SysAdmin => 0x0000_0000_0020_0000,
        Cap::SysBoot => 0x0000_0000_0040_0000,
        Cap::SysNice => 0x0000_0000_0080_0000,
        Cap::SysResource => 0x0000_0000_0100_0000,
        Cap::SysTime => 0x0000_0000_0200_0000,
        Cap::SysTtyConfig => 0x0000_0000_0400_0000,
        Cap::Mknod => 0x0000_0000_0800_0000,
        Cap::Lease => 0x0000_0000_1000_0000,
        Cap::AuditWrite => 0x0000_0000_2000_0000,
        Cap::AuditControl => 0x0000_0000_4000_0000,
        Cap::Setfcap => 0x0000_0000_8000_0000,
        Cap::MacOverride => 0x0000_0001_0000_0000,
        Cap::MacAdmin => 0x0000_0002_0000_0000,
        Cap::Syslog => 0x0000_0004_0000_0000,
        Cap::WakeAlarm => 0x0000_0008_0000_0000,
        Cap::BlockSuspend => 0x0000_0010_0000_0000,
        Cap::AuditRead => 0x0000_0020_0000_0000,
        Cap::Perfmon => 0x0000_0040_0000_0000,
        Cap::Bpf => 0x0000_0080_0000_0000,
        Cap::CheckpointRestore => 0x0000_0100_0000_0000,
    }
}
