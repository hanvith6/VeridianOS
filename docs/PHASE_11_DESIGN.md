# VeridianOS Phase 11 — Distributed Multi-Kernel Coherence

> **Status:** Design Draft  
> **Depends On:** Phase 10 (Self-Improving Kernel Policies)  
> **Estimated Duration:** 6–8 weeks  
> **Complexity Summary:** High overall; see per-component breakdown below.

---

## 1. Problem Statement & Motivation

VeridianOS Phases 1–10 built a fully capable, AI-native, capability-based microkernel operating entirely within a **single address domain**. Even Phase 10's adaptive scheduler, Phase 7's task graphs, and Phase 9's agent runtime are all local to one RISC-V hart running one kernel image.

Modern AI inference and training workloads — from LLM serving to multi-agent orchestration — intrinsically span multiple physical nodes. When the kernel itself knows about tasks (Phase 7) and agents (Phase 9), the natural next step is to allow the kernel to **coordinate work and identity across machine boundaries** without surrendering the security invariants of the capability model.

Phase 11 addresses three interlocking problems:

| Problem | Phase 11 Solution |
|---|---|
| **Work distribution**: An AI task graph too large for one machine. | Distributed NES task graph partitioning and cross-kernel node dispatch. |
| **Identity portability**: A capability handle is meaningless on another kernel. | Cryptographically authenticated Distributed Capability Transfer Protocol (DCTP). |
| **Knowledge consistency**: The Semantic Graph Filesystem diverges across nodes. | Raft-based semantic graph replication with eventual-consistency gossip for large blobs. |

### Why in the Kernel?

Prior art (MPI, gRPC, Spark) handles distribution entirely in user space, delegating scheduling to libraries. This creates three fundamental inefficiencies:

1. **Double-scheduling**: The OS scheduler knows nothing about inter-node dependencies, causing suboptimal preemption of graph-critical nodes.
2. **Capability laundering**: User-space identity tokens (JWT, certificates) must be re-validated on every cross-node call. A kernel-managed capability can carry verifiable, hardware-attested provenance.
3. **Graph incoherence**: The Semantic Graph Filesystem (Phase 8) cannot enforce graph invariants (type-safe edges, semantic constraints) if peers apply updates without coordination.

By making distribution a **first-class kernel service**, VeridianOS can schedule, migrate, and synchronize with the same safety guarantees and performance-critical paths that govern local execution.

---

## 2. Architecture Overview

### 2.1 Physical Topology

Phase 11 treats each QEMU instance (or each RISC-V hart with its own S-mode domain, as in M-mode domain separation via the Physical Memory Protection unit) as a **Kernel Domain**:

```
  ╔══════════════════════════════════════════════════════════════╗
  ║                  VeridianOS Multi-Kernel Cluster             ║
  ║                                                              ║
  ║  ┌─────────────────────────────┐                            ║
  ║  │   Kernel Domain 0 (K0)      │ ◄──── Primary / Leader     ║
  ║  │  ┌──────┐ ┌──────┐ ┌─────┐ │                            ║
  ║  │  │ NES  │ │ SGF  │ │ AGT │ │                             ║
  ║  │  └──┬───┘ └──┬───┘ └──┬──┘ │                            ║
  ║  │     │        │        │    │                             ║
  ║  │  ┌──▼────────▼────────▼──┐ │                            ║
  ║  │  │     DKCP Endpoint     │ │                             ║
  ║  │  │   (virtio-net / IPI)  │ │                             ║
  ║  │  └──────────┬────────────┘ │                             ║
  ║  └─────────────┼──────────────┘                            ║
  ║                │  DKCP Transport (virtio-net or shared mem) ║
  ║  ┌─────────────┼──────────────┐                            ║
  ║  │   Kernel Domain 1 (K1)     │                            ║
  ║  │  ┌──────┐ ┌──────┐ ┌─────┐│                            ║
  ║  │  │ NES  │ │ SGF  │ │ AGT ││                            ║
  ║  │  └──┬───┘ └──┬───┘ └──┬──┘│                            ║
  ║  │  ┌──▼────────▼────────▼──┐│                            ║
  ║  │  │     DKCP Endpoint     ││                            ║
  ║  │  └───────────────────────┘│                            ║
  ║  └─────────────────────────────┘                           ║
  ║                                                              ║
  ║  (Additional domains K2..Kn connect the same way)           ║
  ╚══════════════════════════════════════════════════════════════╝
```

**NES** = Neural Execution Subsystem  
**SGF** = Semantic Graph Filesystem  
**AGT** = Agent Runtime  
**DKCP** = Distributed Kernel Coherence Protocol (new, Phase 11)

### 2.2 Intra-Domain vs. Cross-Domain Communication

```
  ┌──────────────────────────────────────────────────────────┐
  │                    Communication Layers                   │
  │                                                          │
  │  L0: RISC-V IPI (inter-processor interrupt)              │
  │      hart → hart, same QEMU instance, <100 ns latency    │
  │      Used for: local domain wakeup, spin-lock release     │
  │                                                          │
  │  L1: Shared MMIO Ring Buffer (virtio-net emulated)       │
  │      cross-QEMU via loopback, ~5–20 µs RTT               │
  │      Used for: DKCP messages (≤ 4 KB)                    │
  │                                                          │
  │  L2: DKCP Bulk Transfer (chunked pages via virtio)       │
  │      For: capability handle blobs, SGF node payloads     │
  │      ~50–200 µs per 64 KB transfer                       │
  │                                                          │
  │  L3: Consensus Log (Raft replicated across domains)      │
  │      Used for: SGF structural mutations, Raft entries     │
  └──────────────────────────────────────────────────────────┘
```

### 2.3 RISC-V IPI Usage

RISC-V defines inter-processor interrupts via the SBI `sbi_send_ipi()` call. Within a multi-hart QEMU session, IPIs allow one hart to asynchronously signal another. In Phase 11:

- **Local domain wakeup**: When the DKCP endpoint enqueues a received packet, it fires an IPI to the scheduler hart to wake the DKCP dispatch thread immediately rather than relying on the next timer tick.
- **Completion notification**: When a remote NES node finishes and its result is returned via DKCP, the receiving kernel fires an IPI to unblock any local thread waiting in `SYS_GRAPH_WAIT`.
- **Raft heartbeat interrupt**: The Raft leader sends heartbeat IPIs to follower domains at configurable intervals (default: 50 ms), avoiding polling.

```
  Hart 0 (DKCP recv loop):
    ├── receives packet from virtio-net ring
    ├── writes to DKCP_INBOX
    └── SBI: sbi_send_ipi(hart_mask = scheduler_hart_bit)
           │
           ▼
  Hart 1 (Scheduler hart, woken from wfi):
    ├── checks DKCP_INBOX
    ├── dispatches DKCP message handler
    └── resumes waiting graph node / Raft follower logic
```

### 2.4 Shared Memory Between Kernel Domains

For QEMU multi-process simulation, shared memory is established via the `ivshmem` (Inter-VM Shared Memory) device. In Phase 11, this is the **preferred zero-copy transport**:

```
  Physical Memory Layout (each domain maps its own view):

  0x8000_0000 — 0x8FFF_FFFF  Kernel Domain 0 private RAM
  0x9000_0000 — 0x9FFF_FFFF  Kernel Domain 1 private RAM
  0xA000_0000 — 0xA00F_FFFF  DKCP Shared Window (1 MB, mapped RW by both)
    ├── 0xA000_0000  DKCP_CTRL: 64-byte control block (domain count, epoch)
    ├── 0xA000_0040  DKCP_TX_RING[D0→D1]: 256-entry × 256-byte message ring
    ├── 0xA004_0000  DKCP_TX_RING[D1→D0]: same structure
    └── 0xA008_0000  DKCP_BULK_AREA: 512 KB for large blob transfers
```

For the QEMU-only simulation in Phase 11, the shared window is emulated via a **virtio-net** loopback between two QEMU instances with identical memory-mapped layouts, since true ivshmem requires QEMU inter-process coordination. The abstraction layer is identical; only the physical transport differs.

---

## 3. Key Data Structures

All new structures are added to the kernel in the new `kernel/src/dist/` module.

### 3.1 `KernelDomainId`

```rust
/// Unique identifier for a kernel domain in the cluster.
/// Domain 0 is always the bootstrap leader.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(C)]
pub struct KernelDomainId(pub u16);

impl KernelDomainId {
    pub const LOCAL: KernelDomainId = KernelDomainId(0);
    pub const MAX_DOMAINS: usize = 8;
}
```

**Complexity: Low**

### 3.2 `DkcpMessage` (Distributed Kernel Coherence Protocol Message)

```rust
/// Fixed-size DKCP protocol message. Fits in one cache line (64 bytes).
/// Larger payloads reference an offset into DKCP_BULK_AREA.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
pub struct DkcpMessage {
    /// Message type discriminant.
    pub kind: DkcpMessageKind,
    /// Source kernel domain.
    pub src_domain: KernelDomainId,
    /// Destination kernel domain.
    pub dst_domain: KernelDomainId,
    /// Monotonic sequence number for duplicate detection.
    pub seq: u32,
    /// HMAC-SHA256 truncated to 16 bytes for message authentication.
    pub mac: [u8; 16],
    /// Inline payload (32 bytes). For larger data, points into bulk area.
    pub payload: DkcpPayload,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DkcpMessageKind {
    // Cluster management
    Hello            = 0x01,  // Domain join announcement
    Heartbeat        = 0x02,  // Liveness check
    Goodbye          = 0x03,  // Graceful shutdown
    // NES Task Graph distribution
    GraphNodeDispatch = 0x10, // Dispatch a NES node to remote domain
    GraphNodeResult   = 0x11, // Result of a dispatched NES node
    GraphNodeAbort    = 0x12, // Cancel a remote graph node
    // Capability transfer (DCTP)
    CapExportRequest  = 0x20, // Request to send a capability to peer
    CapExportAck      = 0x21, // Acknowledge receipt, return remote handle ID
    CapRevokeNotify   = 0x22, // Notify peer that a cap has been revoked
    // Semantic Graph replication (Raft)
    RaftRequestVote   = 0x30,
    RaftVoteGranted   = 0x31,
    RaftAppendEntries = 0x32,
    RaftAppendAck     = 0x33,
    // Semantic Graph gossip
    GossipDigest      = 0x40, // Anti-entropy digest of node version vectors
    GossipRequest     = 0x41, // Request missing entries
    GossipData        = 0x42, // Push missing entries
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union DkcpPayload {
    pub raw:             [u8; 32],
    pub node_dispatch:   GraphNodeDispatchPayload,
    pub cap_export:      CapExportPayload,
    pub raft_entry:      RaftEntryPayload,
    pub gossip_digest:   GossipDigestPayload,
}
```

**Complexity: Medium**

### 3.3 `GraphNodeDispatchPayload`

```rust
/// Payload for DkcpMessageKind::GraphNodeDispatch.
/// Tells the remote domain what NES node to execute.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GraphNodeDispatchPayload {
    /// Which local graph (on the originating domain) this node belongs to.
    pub origin_graph_id: u32,
    /// Local node index within that graph.
    pub origin_node_id:  u16,
    /// Operation type (mirrors NES OpType).
    pub op_type:         u8,
    /// Preferred device hint on remote domain (0=Any).
    pub device_hint:     u8,
    /// Size of input data in DKCP_BULK_AREA.
    pub data_size_bytes: u32,
    /// Offset into DKCP_BULK_AREA for input data (0 if inline).
    pub bulk_offset:     u32,
    /// Packed dependency mask (remote node IDs, up to 16).
    pub dep_mask:        u16,
    _pad:                [u8; 10],
}
```

**Complexity: Medium**

### 3.4 `DistributedCapability` (DCTP)

```rust
/// A capability that exists simultaneously on multiple domains.
/// The originating domain holds the authoritative rights bitmap;
/// remote domains hold a "shadow" with a bounded rights subset.
#[derive(Clone, Debug)]
pub struct DistributedCapability {
    /// The domain that originally created this capability.
    pub origin_domain: KernelDomainId,
    /// Global capability UID (128-bit, cryptographically random).
    pub global_uid:    [u8; 16],
    /// Rights held on the *local* domain (never exceeds origin rights).
    pub local_rights:  Rights,
    /// The local handle ID this maps to (Some) or None if not yet imported.
    pub local_handle:  Option<u32>,
    /// Revocation epoch: if origin increments this, all remote shadows expire.
    pub epoch:         u32,
}

/// Global table of all distributed capabilities known to this domain.
pub static DIST_CAP_TABLE: Mutex<DistCapTable> = Mutex::new(DistCapTable::new());
```

**Complexity: High** — requires cryptographic identity, revocation protocol, and handle table integration.

### 3.5 `RaftState` (Semantic Graph Consensus)

```rust
/// Raft consensus state for Semantic Graph replication.
/// Each kernel domain participates as a Raft peer.
pub struct RaftState {
    /// Current term number.
    pub current_term:  u64,
    /// Domain ID we voted for in the current term (None if not voted).
    pub voted_for:     Option<KernelDomainId>,
    /// Index of last log entry applied to the local semantic graph.
    pub commit_index:  u64,
    /// Index of last log entry appended to our log.
    pub last_applied:  u64,
    /// Role of this domain in the current term.
    pub role:          RaftRole,
    /// Monotonic tick counter for election timeout tracking.
    pub election_tick: u32,
    /// Raft log: ordered list of semantic graph mutations.
    pub log:           RaftLog,
    // Leader-only state:
    /// next_index[i] = next log entry to send to domain i.
    pub next_index:    [u64; KernelDomainId::MAX_DOMAINS],
    /// match_index[i] = highest log entry known replicated on domain i.
    pub match_index:   [u64; KernelDomainId::MAX_DOMAINS],
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RaftRole {
    Follower  = 0,
    Candidate = 1,
    Leader    = 2,
}

/// A single Raft log entry encapsulates one semantic graph mutation.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct RaftLogEntry {
    pub term:     u64,
    pub index:    u64,
    pub mutation: SemanticGraphMutation,
}

/// A semantic graph mutation replicated via Raft.
#[derive(Clone, Debug)]
pub enum SemanticGraphMutation {
    NodeCreate  { node_type: u8, label_hash: u64 },
    NodeDelete  { node_id: u32 },
    EdgeAdd     { from: u32, to: u32, edge_type: u8 },
    EdgeRemove  { from: u32, to: u32 },
    BlobUpdate  { node_id: u32, blob_bulk_offset: u32, blob_len: u32 },
}
```

**Complexity: High**

### 3.6 `DkcpRing` (Lock-Free Transport Ring)

```rust
/// Single-producer, single-consumer ring buffer for DKCP messages.
/// Mapped into the shared memory window at a fixed physical address.
/// Uses an atomic head/tail for coordination between domains without locks.
#[repr(C, align(4096))]
pub struct DkcpRing {
    pub head: AtomicU32,           // written by consumer
    pub tail: AtomicU32,           // written by producer
    _pad:     [u8; 56],            // pad to cache line
    pub slots: [DkcpMessage; 256], // 256 × 64 bytes = 16 KB ring body
}
```

**Complexity: Medium**

---

## 4. New Syscalls (Numbering from 90+)

All new syscalls are in range `[90, 119]`. They are registered in `kernel/src/syscall/numbers.rs` and dispatched in `kernel/src/syscall/mod.rs`.

### 4.1 Cluster Management

```
SYS_DOMAIN_JOIN   = 90
  a0: ptr to DomainJoinArgs { self_id: u16, transport_mmio_base: u64, transport_mmio_len: u32 }
  Returns: 0 on success, negative error code

SYS_DOMAIN_LIST   = 91
  a0: ptr to output buffer [KernelDomainId; 8]
  a1: ptr to output count (usize)
  Returns: number of live domains

SYS_DOMAIN_STATUS = 92
  a0: domain_id (u16)
  Returns: DomainStatus bitmask (ALIVE=1, LEADER=2, SYNCED=4)
```

**Complexity: Low** — thin wrappers around `dist::cluster::*`.

### 4.2 Distributed NES Graph Dispatch

```
SYS_GRAPH_DISPATCH_REMOTE = 93
  a0: local_graph_id (u32)   — handle to local NES graph
  a1: node_id (u16)          — which node to dispatch remotely
  a2: target_domain (u16)    — destination domain ID (0 = kernel chooses)
  a3: device_hint (u8)       — preferred device on remote (0 = Any)
  Returns: remote_ticket (u64) — opaque ID to use with SYS_GRAPH_WAIT_REMOTE

SYS_GRAPH_WAIT_REMOTE = 94
  a0: remote_ticket (u64)
  a1: timeout_us (u64)
  a2: ptr to output ResultBuffer { status: u32, data_bulk_offset: u32, data_len: u32 }
  Returns: 0 on success, -ETIMEDOUT, -EREMOTE

SYS_GRAPH_ABORT_REMOTE = 95
  a0: remote_ticket (u64)
  Returns: 0 on success, -EALREADY (node already completed)
```

**Complexity: High** — requires ticket tracking, result buffering, and DKCP round-trip.

### 4.3 Distributed Capability Transfer (DCTP)

```
SYS_CAP_EXPORT = 96
  a0: local_handle_id (u32)      — handle to export
  a1: target_domain (u16)        — which domain to send to
  a2: rights_to_grant (u32)      — subset of local rights (must be ≤ local)
  a3: ptr to output [u8; 16]     — filled with global_uid of exported cap
  Returns: 0 on success, -EPERM if rights exceed local, -ENOENT if handle missing

SYS_CAP_IMPORT = 97
  a0: ptr to [u8; 16] global_uid — UID of capability to import
  a1: ptr to output local_handle (u32) — filled with new local handle ID
  Returns: 0 on success, -EPERM if origin revoked, -ENOENT if UID unknown

SYS_CAP_REVOKE_REMOTE = 98
  a0: ptr to [u8; 16] global_uid — capability to revoke everywhere
  Returns: 0 on success (revocation is async; shadow handles on peers expire
          within one Raft election timeout period)
```

**Complexity: High** — security-critical; requires MAC verification of transfer packets.

### 4.4 Semantic Graph Replication

```
SYS_SGF_REPLICATE_ENABLE = 99
  a0: node_id (u32)       — which SGF node to make globally replicated
  Returns: 0 on success

SYS_SGF_REPLICATE_QUERY = 100
  a0: node_id (u32)
  a1: ptr to output ReplicationStatus { commit_index: u64, applied_on_domains: u8 }
  Returns: 0 on success, -ENOENT if node not replicated

SYS_SGF_RAFT_STATUS = 101
  a1: ptr to output RaftPublicStatus { role: u8, term: u64, commit_index: u64, leader_domain: u16 }
  Returns: 0 always
```

**Complexity: Medium** — thin wrappers; core complexity is in `dist::raft`.

---

## 5. Implementation Plan

### Ordered File Creation / Modification

Each step builds on the previous. Estimated complexity is marked: 🟢 Low | 🟡 Medium | 🔴 High.

#### Step 1 — Scaffolding: `kernel/src/dist/` module 🟢

Create the module skeleton. No logic yet.

```
kernel/src/dist/
├── mod.rs           (re-exports, declares submodules)
├── types.rs         (KernelDomainId, DkcpMessage, all structs above)
├── ring.rs          (DkcpRing lock-free SPSC ring buffer)
├── transport.rs     (virtio-net / mmio send/recv primitives)
├── cluster.rs       (domain membership table, heartbeat logic)
├── nes_dist.rs      (remote graph dispatch, ticket table)
├── dctp.rs          (distributed capability transfer protocol)
└── raft.rs          (Raft consensus engine for SGF)
```

**Files to create:** `kernel/src/dist/mod.rs`, `kernel/src/dist/types.rs`  
**Files to modify:** `kernel/src/main.rs` (add `pub mod dist;`)

#### Step 2 — Lock-Free Ring Buffer: `ring.rs` 🟡

Implement `DkcpRing` using `core::sync::atomic::AtomicU32` with `Acquire`/`Release` ordering. Test with local loopback (sender and receiver in the same domain during early boot).

**Files to create:** `kernel/src/dist/ring.rs`

#### Step 3 — Transport Abstraction: `transport.rs` 🟡

Wrap the existing `virtio::blk` subsystem or a new virtio-net device (or a static shared-memory buffer for testing) as the DKCP transport. Implement:
- `fn dkcp_send(msg: &DkcpMessage) -> Result<(), DkcpError>`
- `fn dkcp_recv() -> Option<DkcpMessage>`
- IPI-triggered wakeup via `sbi::sbi_send_ipi()`

For the Phase 11 MVP, use a **static shared memory array** (`static LOOPBACK_RING: DkcpRing`) mapped identically in both QEMU instances — simplest to verify.

**Files to create:** `kernel/src/dist/transport.rs`  
**Files to modify:** `kernel/src/sbi.rs` (add `sbi_send_ipi()` wrapper)

#### Step 4 — Cluster Membership: `cluster.rs` 🟡

Implement:
- `static CLUSTER: Mutex<ClusterState>` with domain table
- `fn domain_join(id, transport_base)` — called from `SYS_DOMAIN_JOIN`
- `fn heartbeat_tick()` — called from the timer interrupt; marks domains dead after 3 missed heartbeats
- `fn cluster_init()` — initializes self as domain 0

**Files to create:** `kernel/src/dist/cluster.rs`  
**Files to modify:** `kernel/src/main.rs` (call `dist::cluster::cluster_init()` during boot)  
**Files to modify:** `kernel/src/trap.rs` (call `dist::cluster::heartbeat_tick()` in timer handler)

#### Step 5 — Syscall Wiring: `numbers.rs`, `mod.rs` 🟢

Register syscalls 90–101. Wire them to stub handlers that print `[DIST] SYS_xxx called` and return 0. This lets the user-space test program compile and link before the full implementation.

**Files to modify:** `kernel/src/syscall/numbers.rs`  
**Files to modify:** `kernel/src/syscall/mod.rs`

#### Step 6 — Remote NES Dispatch: `nes_dist.rs` 🔴

This is the most complex component. Implement:

1. **`RemoteTicketTable`**: A fixed-size array (`MAX_REMOTE_TICKETS = 64`) of:
   ```rust
   struct RemoteTicket {
       ticket_id:       u64,
       origin_graph_id: u32,
       origin_node_id:  u16,
       remote_domain:   KernelDomainId,
       state:           TicketState,  // Pending / Complete / Aborted
       result_offset:   u32,          // into DKCP_BULK_AREA
       result_len:      u32,
   }
   ```
2. **`sys_graph_dispatch_remote()`**: Creates a `DkcpMessage::GraphNodeDispatch`, serializes the node's input data into `DKCP_BULK_AREA`, allocates a ticket, and sends via `transport::dkcp_send()`.
3. **`sys_graph_wait_remote()`**: Spins (with WFI) on `ticket.state == TicketState::Complete`. On completion, copies result from bulk area to caller's buffer.
4. **DKCP receive handler** (`handle_incoming_message()`): Called from the DKCP dispatch thread. For `GraphNodeResult` messages, marks the matching ticket complete and fires an IPI to wake waiting threads.
5. **Remote execution side**: When a `GraphNodeDispatch` message arrives on the remote domain, it schedules a local NES node (using existing `nes::syscalls` internals), runs it, then sends back a `GraphNodeResult`.

**Files to create:** `kernel/src/dist/nes_dist.rs`  
**Files to modify:** `kernel/src/nes/syscalls.rs` (expose `execute_node_raw()` for internal use)

#### Step 7 — Distributed Capability Transfer: `dctp.rs` 🔴

Implement the DCTP security protocol:

1. **Shared secret bootstrap**: In Phase 11, each domain pair shares a pre-shared 32-byte key stored in a `static DOMAIN_KEYS: [[u8; 32]; 8]` array (populated at domain join time — future phases can replace with DH). This key is used for HMAC-SHA256 (truncated to 16 bytes) on every `CapExport*` message.

2. **`sys_cap_export()`**:
   - Validate `local_handle_id` exists and caller has `Rights::TRANSFER`
   - Validate `rights_to_grant ⊆ local_rights`
   - Generate `global_uid = blake2s(handle_id ‖ origin_domain ‖ nonce)` (using a compact PRNG seeded from `rdtime`)
   - Insert into `DIST_CAP_TABLE`
   - Send `DkcpMessage::CapExportRequest` with the global_uid, rights, and object type
   - Wait for `CapExportAck` (or timeout)

3. **`sys_cap_import()`**:
   - Look up `global_uid` in `DIST_CAP_TABLE`
   - Validate MAC of the original export request
   - Create a local shadow handle in the current process's `HandleTable`
   - Return new local handle ID

4. **`sys_cap_revoke_remote()`**:
   - Increment `epoch` in `DIST_CAP_TABLE` entry
   - Broadcast `DkcpMessage::CapRevokeNotify` to all live domains
   - Remote domains invalidate shadow handles with matching `global_uid`

**Files to create:** `kernel/src/dist/dctp.rs`  
**Files to modify:** `kernel/src/capability/mod.rs` (add `Rights::TRANSFER` bit, shadow handle support)

#### Step 8 — Raft Consensus Engine: `raft.rs` 🔴

Implement a minimal Raft (Ongaro & Ousterhout, 2014) for semantic graph mutation replication:

1. **Leader Election**: Standard Raft randomized election timeout (150–300 ms in wall time, approximated by timer tick counts). Domains broadcast `RaftRequestVote`; majority vote wins.

2. **Log Replication**: Leader receives `SemanticGraphMutation` (from `SYS_NODE_CREATE`, `SYS_EDGE_ADD`, etc.), appends to local log, broadcasts `RaftAppendEntries` to all followers. Commit once a quorum acknowledges.

3. **Application to SGF**: On commit, apply the `SemanticGraphMutation` to the local `semantic_graph` module. This guarantees linearizability: all non-leader kernels apply mutations in the same order.

4. **Optimization — Read Lease**: For `SYS_GRAPH_QUERY` (read-only), the leader grants a "read lease" valid for one election timeout: followers can serve reads locally without contacting the leader (avoids round-trip for the common read case).

5. **Log Compaction (Snapshot)**: At `commit_index` multiples of 128, serialize the full semantic graph state into `DKCP_BULK_AREA` and broadcast as a Raft snapshot. This bounds log memory usage.

**Files to create:** `kernel/src/dist/raft.rs`  
**Files to modify:** `kernel/src/semantic_graph/mod.rs` (add `apply_mutation(m: &SemanticGraphMutation)` function, gate write operations behind Raft when replication is enabled)

#### Step 9 — Anti-Entropy Gossip: `raft.rs` (gossip section) 🟡

For large semantic graph blobs (Phase 8 node payloads), Raft log entries only carry a `BlobUpdate { node_id, bulk_offset, blob_len }` pointer. The actual blob bytes are exchanged via a **gossip protocol** (inspired by Amazon Dynamo):

- Each domain maintains a **version vector** per node: `(node_id → last_mutation_term)`.
- Periodically (every 500 ms), send a `GossipDigest` to a random peer.
- The peer compares digests, responds with `GossipRequest` for nodes where it is behind.
- The requester responds with `GossipData` chunks.

This decouples blob synchronization from the latency-sensitive Raft commit path.

**Files to modify:** `kernel/src/dist/raft.rs` (add gossip functions)

#### Step 10 — SGF Replication Syscalls: `mod.rs` 🟢

Wire `SYS_SGF_REPLICATE_ENABLE`, `SYS_SGF_REPLICATE_QUERY`, `SYS_SGF_RAFT_STATUS` to the Raft module functions.

**Files to modify:** `kernel/src/syscall/mod.rs`

#### Step 11 — Boot Integration: `main.rs` 🟢

Add Phase 11 initialization sequence after agent runtime init:

```rust
// Phase 11: Distributed Multi-Kernel Coherence
println!("[BOOT] Initializing Distributed Kernel Coherence...");
dist::cluster::cluster_init(KernelDomainId(0));
dist::raft::init();
println!("[BOOT] Domain 0 initialized as Raft leader candidate.");
```

Spawn DKCP dispatch thread:
```rust
thread::spawn_thread(dist::transport::dkcp_dispatch_loop)
    .expect("Failed to spawn DKCP dispatch thread");
```

**Files to modify:** `kernel/src/main.rs`

#### Step 12 — User-Space Verification Program 🟡

Create `user_programs/dist_test/`:

```
user_programs/dist_test/
├── Cargo.toml
└── src/
    └── main.rs
```

The `dist_test` program (detailed in §6) exercises all new syscalls and prints structured results.

**Files to create:** `user_programs/dist_test/Cargo.toml`, `user_programs/dist_test/src/main.rs`  
**Files to modify:** `Cargo.toml` (workspace members), `Makefile` (build target)

---

## 6. Verification Strategy

### 6.1 Unit Tests (within the kernel, `#[test]` under `cfg(test)`)

Since VeridianOS runs `no_std`, unit tests use the `cargo test --target x86_64-unknown-linux-gnu` trick for pure-logic modules:

1. **`ring.rs`**: Test SPSC ring with a local sender+receiver in the same address space. Assert no data loss at 10,000 messages, correct ordering, and correct wraparound.
2. **`dctp.rs`**: Test `global_uid` generation uniqueness, rights subset validation, and epoch-based revocation logic — all without actual network communication.
3. **`raft.rs`**: Test election logic with a mock message bus (a `[VecDeque<DkcpMessage>; 8]` array simulating the network) for a 3-domain cluster. Assert leader emerges, log replication commits.

### 6.2 Integration Test: `dist_test` User Program

The `dist_test` binary is booted as the init process by domain 0. It simulates a second domain by using the loopback transport.

**Phase 11 Verification Sequence:**

```
[DIST_TEST] Step 1: Query cluster status — expect 1 domain alive (self)
  syscall: SYS_DOMAIN_LIST
  assert: count == 1, domains[0] == KernelDomainId(0)
  PASS ✓

[DIST_TEST] Step 2: Create NES graph and dispatch node to remote (loopback)
  syscall: SYS_GRAPH_CREATE → graph_id
  syscall: SYS_GRAPH_ADD_NODE (GEMM, size=1024, deps=[])
  syscall: SYS_GRAPH_DISPATCH_REMOTE (node=0, domain=0 loopback)
  syscall: SYS_GRAPH_WAIT_REMOTE (ticket, timeout=10_000 µs)
  assert: result.status == 0, result.data_len == 1024
  PASS ✓

[DIST_TEST] Step 3: Export capability and re-import it
  syscall: SYS_CAP_EXPORT (handle=0, domain=0 loopback, rights=READ)
  syscall: SYS_CAP_IMPORT (global_uid from above)
  assert: new_handle != original_handle, rights == READ
  PASS ✓

[DIST_TEST] Step 4: Revoke exported capability
  syscall: SYS_CAP_REVOKE_REMOTE (global_uid)
  // try to use the shadow handle — should fail
  syscall: SYS_HANDLE_CLOSE (new_handle)  → expect -ENOENT (already revoked)
  PASS ✓

[DIST_TEST] Step 5: Create SGF node, enable replication, check Raft status
  syscall: SYS_NODE_CREATE
  syscall: SYS_SGF_REPLICATE_ENABLE (node_id)
  syscall: SYS_SGF_RAFT_STATUS
  assert: role == Leader, term >= 1, commit_index >= 1
  PASS ✓

[DIST_TEST] ALL TESTS PASSED — VeridianOS Phase 11 verified!
```

### 6.3 Two-QEMU Integration Test

For full multi-node verification (optional, for CI with `make run-cluster`):

1. Launch QEMU instance 0 with `disk.img` containing `dist_test`.
2. Launch QEMU instance 1 with virtio-net connected to instance 0 via a TAP bridge.
3. Domain 0 calls `SYS_DOMAIN_JOIN` with domain 1's transport address.
4. Domain 1 calls `SYS_DOMAIN_JOIN` with domain 0's transport address.
5. Verify Raft leader election produces exactly one leader.
6. Verify capability export from domain 0 imports correctly on domain 1.
7. Verify a semantic graph node created on domain 0 appears on domain 1 after commit.

**Makefile target:** `make run-cluster` (launches both QEMU instances and pipes output to logs).

---

## 7. Academic References

### Distributed Operating Systems & Coherence

**[1] Lamport, L. (1978). "Time, Clocks, and the Ordering of Events in a Distributed System."** *Communications of the ACM, 21*(7), 558–565.  
Foundational paper establishing the happened-before relation and logical clocks. Directly informs the `seq` monotonic counter in `DkcpMessage` for duplicate detection and causal ordering of capability operations.

**[2] Ongaro, D., & Ousterhout, J. (2014). "In Search of an Understandable Consensus Algorithm (Extended Version)."** *USENIX ATC '14.*  
The Raft paper. Phase 11's consensus engine is a direct implementation of §3–§5 of this paper, adapted for a `no_std` Rust environment. We use Raft's randomized election timeout, log matching property, and commit rule verbatim.

**[3] DeCandia, G., et al. (2007). "Dynamo: Amazon's Highly Available Key-Value Store."** *SOSP '07.*  
The gossip-based anti-entropy and vector clock mechanisms in Phase 11's SGF blob synchronization are directly inspired by Dynamo §4.7–4.8. The version vector per semantic graph node mirrors Dynamo's per-key vector clock.

**[4] Tanenbaum, A. S., & Van Renesse, R. (1985). "Distributed Operating Systems."** *ACM Computing Surveys, 17*(4), 419–470.  
Seminal survey covering capability migration across domain boundaries. The observation that "capabilities must be unforgeable across domains" (§3.2) directly motivates our HMAC-authenticated `CapExportRequest` in DCTP.

### Capability Migration

**[5] Levy, H. M. (1984). "Capability-Based Computer Systems."** *Digital Press.*  
Chapter 8 ("Capability Migration") is the canonical reference for the rights-amplification problem in distributed capability systems. Phase 11's rule that `rights_to_grant ⊆ local_rights` and the epoch-based revocation protocol implement the "rights attenuation" principle from this chapter.

**[6] Murray, D., et al. (2013). "Naiad: A Timely Dataflow System."** *SOSP '13.*  
Naiad's progress tracking across a distributed dataflow graph directly parallels how Phase 11 tracks `RemoteTicket` completion across the DKCP. The "pointstamp" concept maps to our `(origin_graph_id, origin_node_id, seq)` tuple.

### Microkernel Distribution

**[7] Barham, P., et al. (2003). "Xen and the Art of Virtualization."** *SOSP '03.*  
Xen's shared-memory event channel mechanism (§2.3) is the architectural model for the `DkcpRing` shared MMIO ring buffer. The producer-consumer protocol with `AtomicU32` head/tail mirrors Xen's ring descriptor design.

**[8] Hohmuth, M., & Härtig, H. (2001). "Pragmatic Nonblocking Synchronization for Real-Time Systems."** *USENIX ATC '01.*  
Informs our choice of `Acquire`/`Release` atomic ordering (rather than `SeqCst`) in `DkcpRing`, and the avoidance of MCS locks in interrupt-driven DKCP paths.

**[9] Shapiro, M., et al. (2011). "Conflict-Free Replicated Data Types."** *SSS '11.*  
The Phase 11 gossip protocol's version vector is a CRDT G-Counter, allowing the semantic graph's blob metadata to be merged without coordination. This paper justifies why blob metadata (node versions) can use eventual consistency while structural mutations (edge additions) require Raft.

### RISC-V Inter-Processor Interrupts

**[10] RISC-V International. (2023). "RISC-V Supervisor Binary Interface (SBI) Specification v2.0."**  
§6 (IPI Extension, EID=0x735049) defines the `sbi_send_ipi()` function used in Phase 11 for cross-hart notification. Our `sbi.rs` wrapper calls `ecall` with `EID=0x735049, FID=0x0`.

**[11] Waterman, A., et al. (2019). "The RISC-V Instruction Set Manual, Volume II: Privileged Architecture."** *RISC-V International.*  
§3.1.7 (Machine-Mode IPI via CLINT `msip` registers) and §4.6 (Supervisor-mode IPI delegation) describe the hardware mechanism underlying SBI IPI calls, directly informing the `dkcp_dispatch_loop` interrupt handling strategy.

---

## 8. Complexity Summary

| Component | Complexity | Key Risk |
|---|---|---|
| Module scaffold (`dist/mod.rs`, `types.rs`) | 🟢 Low | None |
| `DkcpRing` lock-free ring buffer | 🟡 Medium | Memory ordering bugs on relaxed hardware |
| Transport abstraction (`transport.rs`) | 🟡 Medium | virtio-net driver integration |
| Cluster membership (`cluster.rs`) | 🟡 Medium | Heartbeat timing with timer interrupt |
| Syscall wiring (90–101) | 🟢 Low | Correct dispatch table indexing |
| Remote NES dispatch (`nes_dist.rs`) | 🔴 High | Data serialization, bulk area management |
| DCTP capability transfer (`dctp.rs`) | 🔴 High | Security: rights attenuation, revocation |
| Raft consensus engine (`raft.rs`) | 🔴 High | Correctness: log matching, split-brain |
| Gossip anti-entropy | 🟡 Medium | Version vector merging, bandwidth |
| SGF replication syscalls | 🟢 Low | Thin wrappers over Raft |
| Boot integration | 🟢 Low | Initialization ordering |
| User-space `dist_test` | 🟡 Medium | Syscall interface correctness |
| Two-QEMU integration test | 🟡 Medium | QEMU networking setup |

**Overall Phase Complexity: 🔴 High**

The three High-complexity components (Remote NES, DCTP, Raft) are independent and can be developed in parallel by separate contributors. The recommended implementation order is: Raft first (self-contained), then DCTP (depends only on cluster), then Remote NES (most integration points).

---

## 9. Security Invariants

Phase 11 must not weaken any capability security guarantee established in Phases 3–4:

1. **No rights amplification**: `SYS_CAP_EXPORT` enforces `rights_to_grant ⊆ caller_local_rights` in the kernel — not user space.
2. **Unforgeable global UIDs**: `global_uid` is computed using the kernel's internal PRNG seeded from `rdtime` XOR domain ID. User space never controls `global_uid` generation.
3. **HMAC authentication**: All `CapExport*` DKCP messages carry a 16-byte MAC. A spoofed packet without the correct pre-shared domain key cannot create a valid shadow handle.
4. **Revocation completeness**: `SYS_CAP_REVOKE_REMOTE` broadcasts epoch increment to all live domains. Shadow handles on domains that are temporarily unreachable are garbage-collected when the domain reconnects and processes the revocation log (stored as a Raft log entry).
5. **Principle of least privilege in dispatch**: `SYS_GRAPH_DISPATCH_REMOTE` only transfers *data* (input bytes) to the remote domain, never capability handles. The remote NES executes the computation in an isolated context with no access to the originating process's handle table.

---

## 10. Future Extensions (Phase 12+)

- **Byzantine fault tolerance**: Replace Raft (crash fault tolerant, `f < n/2`) with PBFT or HotStuff for adversarial environments.
- **Hardware attestation**: Replace pre-shared domain keys with RISC-V Keystone enclave attestation for cryptographically verified domain identity.
- **WAN coherence**: Add TCP transport to `transport.rs` for cross-datacenter multi-kernel clusters.
- **Formal verification**: Prove the DCTP rights-attenuation invariant in Coq/Isabelle, following the seL4 methodology.
- **Distributed NES auto-partitioning**: Extend Phase 10's epsilon-greedy scheduler to automatically partition NES task graphs across the cluster based on learned per-domain execution profiles.
