# Network architecture

Muak networking is built from sans-io finite state machines, a platform runtime, and thin
Linux adapters. Everything that decides policy is testable without a kernel; everything that
touches a socket lives in one backend behind a narrow port.

The layering is enforced by `scripts/check-network-layers.sh`, also available as
`just check-network-layers` and run first by `just lint`. The script reads crate manifests and
source imports only, so it runs offline.

## Crate layers

| Layer | Crates | Role |
| --- | --- | --- |
| Domain | `netmodel`, `ip`, `ethernet`, `dhcp-proto`, `ndp`, `mac` | Vocabulary, codecs, and checksums with no I/O |
| Methods | `ip-method`, `ip-static`, `dhcp`, `slaac` | Per-family address provisioning FSMs |
| Control | `iface`, `net-supervisor` | Per-interface and global network FSMs |
| Driver | `net-loop` | Drives the FSMs and executes their effects against the ports |
| Ports | `net-runtime` | Platform-agnostic capability traits and the `Runtime` bundle |
| Backend | `net-runtime-linux` | The Linux implementation of the ports |
| Verbs | `netlink-linux` | rtnetlink message builders and typed parsers |
| Adapters | `packet-v4-linux`, `icmpv6-linux`, `tap-linux` | Synchronous byte pipes over Linux sockets |

Dependencies point down the table, except that every crate may depend on the domain layer. The
domain, methods, control, driver, and ports crates never depend on tokio, netlink, rustix, or
libc, so they compile and test on any target. They also never depend on the verbs crate or the
Linux adapters, and they never reach the pre-Wave 0.2 crates (`netlink`, `netlink-monitor`,
`netdev`, `linkctl`, `dhcp-client`, `packet`, `tap`), which pull tokio or netlink internally.

### Wave 0.2 status

`net-loop` and `net-runtime-linux` are skeletons in Wave 0.2. The Linux backend implementation
and the real driver loop land in later waves; until then the layer rules apply to the skeleton
manifests and sources.

## `netlink-linux` is verbs, not a socket adapter

`netlink-linux` builds `netlink-packet-core` messages for link, address, bridge, and route
operations, and parses `netlink-packet-route` messages back into `netmodel` values. It owns no
socket, no runtime, and no event loop. Only `net-runtime-linux` turns those messages into
bytes on the netlink socket and feeds replies back into the parsers.

## `net-runtime-linux` is the Linux backend

`net-runtime-linux` is the single crate that implements the runtime ports. It owns the netlink
socket, the packet and ICMPv6 sockets, the event source, the clock, and entropy. It is the only
crate allowed to depend on tokio, `netlink-proto`, `netlink-sys`, or to hold a tokio `AsyncFd`.

## Two effect languages

The FSMs speak two effect languages, one per altitude.

- `net-supervisor` emits high-level effects: discover links, probe carriers, configure an
  interface, ensure a bridge, enslave or detach a port, write the resolver, publish status.
  These effects change global network shape.
- `iface` emits low-level effects: set a link up or down, add or remove an address, ensure or
  remove a default route, open, send on, and close a socket. These effects change one
  interface.

`net-supervisor` reaches `iface` through its command input and reads its state through a
snapshot. `iface` never reaches upward, and neither crate reaches a socket directly.

## Async only in runtime ports

Runtime port traits in `net-runtime` may use async, because reading a socket or an event
source must be able to wait. FSM traits never use async: `step` takes the inputs it already
has and returns at most one effect, and `next_wake` reports the deadline at which the FSM must
be polled again. The driver awaits the ports and pushes the results into the FSMs.

## File descriptor ownership

Adapters open and return `OwnedFd`; the backend stores the descriptor and owns its lifetime.
Adapters that operate on an already open descriptor take `BorrowedFd`, and that borrow lasts
for exactly one `send` or `recv` call. No descriptor is stored as `BorrowedFd`, no borrow is
held across an await, and no descriptor is closed twice: closing happens only through the
`OwnedFd` that the backend holds.

## Methods own framing, adapters are byte pipes

An address method decides which frame to build, which fields to set, and which socket to send
it on. Adapters only move bytes: they open a socket bound to a link, send the frame they are
handed, and return the bytes they receive. Adapters never parse protocol headers, never check
checksums, and never reply on their own. This keeps every protocol decision inside an FSM that
can be tested with fixed buffers.

## No `Spawn` in `Runtime`

The `Runtime` bundle exposes link, address, route, events, packet, ICMPv6, clock, and entropy
capabilities. It deliberately has no task spawner. FSMs are driven by `net-loop` on the
backend's own single task, so no FSM state can be moved to another thread behind the driver's
back.

## `tap-linux` is direct

`tap-linux` creates and deletes TUN/TAP devices. It is used directly by `workloadd` for
hypervisor networking, not through the `Runtime` bundle, because a TAP device is a workload
concern rather than a host network configuration concern.

## One async runtime

tokio is the only async runtime in the network stack. There is no smol, and no second executor
to bridge to. Crates that need to sleep or read asynchronously depend on the ports, and the
backend provides the tokio implementation.
