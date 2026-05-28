//! Type definitions for VeridianOS Phase 11 — Distributed Multi-Kernel Coherence
//! Matches docs/PHASE_11_DESIGN.md specifications.

use spin::Mutex;
use crate::capability::Rights;

/// Unique identifier for a kernel domain in the cluster.
/// Domain 0 is always the bootstrap leader.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(C)]
pub struct KernelDomainId(pub u16);

impl KernelDomainId {
    pub const LOCAL: KernelDomainId = KernelDomainId(0);
    pub const MAX_DOMAINS: usize = 8;
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DkcpMessageKind {
    Null             = 0x00,
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
    pub _pad:            [u8; 10],
}

/// Payload for DCTP capability export.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CapExportPayload {
    pub global_uid: [u8; 16],
    pub rights: u32,
    pub object_type: u32,
    pub remote_handle: u32,
    pub _pad: [u8; 4],
}

/// Payload for Raft entries.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RaftEntryPayload {
    pub term: u64,
    pub index: u64,
    pub prev_log_index: u64,
    pub prev_log_term: u64,
}

/// Payload for Gossip digest.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GossipDigestPayload {
    pub version: u64,
    pub node_id: u32,
    pub _pad: [u8; 20],
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

impl core::fmt::Debug for DkcpPayload {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DkcpPayload")
            .field("raw", unsafe { &self.raw })
            .finish()
    }
}

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

impl DkcpMessage {
    pub const fn zeroed() -> Self {
        Self {
            kind: DkcpMessageKind::Null,
            src_domain: KernelDomainId(0),
            dst_domain: KernelDomainId(0),
            seq: 0,
            mac: [0; 16],
            payload: DkcpPayload { raw: [0; 32] },
        }
    }
}

/// A capability that exists simultaneously on multiple domains.
/// The originating domain holds the authoritative rights bitmap;
/// remote domains hold a "shadow" with a bounded rights subset.
#[derive(Clone, Copy, Debug)]
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

/// Global table of distributed capabilities.
pub struct DistCapTable {
    pub caps: [Option<DistributedCapability>; 64],
}

impl DistCapTable {
    pub const fn new() -> Self {
        const NONE: Option<DistributedCapability> = None;
        Self {
            caps: [NONE; 64],
        }
    }
}

/// Global table of all distributed capabilities known to this domain.
pub static DIST_CAP_TABLE: Mutex<DistCapTable> = Mutex::new(DistCapTable::new());

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RaftRole {
    Follower  = 0,
    Candidate = 1,
    Leader    = 2,
}

/// A semantic graph mutation replicated via Raft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticGraphMutation {
    NodeCreate  { node_type: u8, label_hash: u64 },
    NodeDelete  { node_id: u32 },
    EdgeAdd     { from: u32, to: u32, edge_type: u8 },
    EdgeRemove  { from: u32, to: u32 },
    BlobUpdate  { node_id: u32, blob_bulk_offset: u32, blob_len: u32 },
}

/// A single Raft log entry encapsulates one semantic graph mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct RaftLogEntry {
    pub term:     u64,
    pub index:    u64,
    pub mutation: SemanticGraphMutation,
}

/// Raft log containing a static table of entries.
#[derive(Clone, Debug)]
pub struct RaftLog {
    pub entries: [Option<RaftLogEntry>; 128],
    pub len: usize,
}

impl RaftLog {
    pub const fn new() -> Self {
        const NONE: Option<RaftLogEntry> = None;
        Self {
            entries: [NONE; 128],
            len: 0,
        }
    }
}

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
