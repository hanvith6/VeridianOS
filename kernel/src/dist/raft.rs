//! Phase 11 — Raft Consensus Engine for Semantic Graph Replication.
//!
//! Implements the Raft consensus algorithm (Ongaro & Ousterhout, ATC '14)
//! for replicating Semantic Graph Filesystem (SGF) mutations across kernel
//! domains. In single-QEMU loopback mode the node always wins its own
//! election (quorum = 1 vote = majority of 1-node cluster) and becomes
//! Leader within one election timeout tick.

use spin::Mutex;
use super::types::{
    DkcpMessage, DkcpMessageKind, DkcpPayload, RaftEntryPayload,
    KernelDomainId, RaftRole, RaftState, RaftLog, RaftLogEntry,
    SemanticGraphMutation,
};
use super::transport::dkcp_send;
use crate::println;

// ─── Election timeout ─────────────────────────────────────────────────────────

/// Ticks before a follower starts an election. Fixed for determinism in
/// single-node loopback (no split-vote risk). In a real cluster this would
/// be randomised per-node in [150, 300].
const ELECTION_TIMEOUT: u32 = 100;

/// Ticks between leader heartbeat AppendEntries broadcasts.
const HEARTBEAT_INTERVAL: u32 = 30;

// ─── Global Raft state ────────────────────────────────────────────────────────

impl RaftState {
    /// Create the initial Raft state (Follower, term 0, no votes).
    pub const fn new() -> Self {
        const ZERO: u64 = 0;
        RaftState {
            current_term:  0,
            voted_for:     None,
            commit_index:  0,
            last_applied:  0,
            role:          RaftRole::Follower,
            election_tick: ELECTION_TIMEOUT,
            log:           RaftLog::new(),
            next_index:    [1u64; KernelDomainId::MAX_DOMAINS],
            match_index:   [ZERO; KernelDomainId::MAX_DOMAINS],
        }
    }
}

pub static RAFT: Mutex<RaftState> = Mutex::new(RaftState::new());

/// Heartbeat send counter for the leader.
static LEADER_HB_TICK: Mutex<u32> = Mutex::new(0);

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn make_raft_msg(
    kind: DkcpMessageKind,
    src: KernelDomainId,
    dst: KernelDomainId,
    seq: u32,
    payload: RaftEntryPayload,
) -> DkcpMessage {
    DkcpMessage {
        kind,
        src_domain: src,
        dst_domain: dst,
        seq,
        mac: [0u8; 16],
        payload: DkcpPayload { raft_entry: payload },
    }
}

/// Broadcast a message to all known live domains via the loopback ring.
/// In multi-node setups this would iterate over cluster peers.
fn broadcast(msg: &DkcpMessage) {
    let _ = dkcp_send(msg);
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Initialize the Raft engine. Called once from kmain.
pub fn raft_init() {
    let mut r = RAFT.lock();
    r.role = RaftRole::Follower;
    r.current_term = 0;
    r.election_tick = ELECTION_TIMEOUT;
    println!("[RAFT] Initialized: Follower, term=0, election_timeout={}", ELECTION_TIMEOUT);
}

/// Periodic tick — called from heartbeat_tick() in the timer interrupt.
/// - Follower/Candidate: decrement election timer; trigger election on timeout.
/// - Leader: send periodic AppendEntries heartbeats.
pub fn raft_tick() {
    let mut r = RAFT.lock();
    match r.role {
        RaftRole::Follower | RaftRole::Candidate => {
            if r.election_tick == 0 {
                // Start election
                r.current_term += 1;
                r.role = RaftRole::Candidate;
                r.voted_for = Some(KernelDomainId::LOCAL);
                r.election_tick = ELECTION_TIMEOUT;
                let term = r.current_term;
                let last_idx = r.log.len as u64;
                let last_term = if last_idx > 0 {
                    r.log.entries[last_idx as usize - 1]
                        .map(|e| e.term)
                        .unwrap_or(0)
                } else { 0 };
                drop(r);

                println!("[RAFT] Election started: term={}", term);
                let msg = make_raft_msg(
                    DkcpMessageKind::RaftRequestVote,
                    KernelDomainId::LOCAL,
                    KernelDomainId::LOCAL, // loopback
                    term as u32,
                    RaftEntryPayload { term, index: 0, prev_log_index: last_idx, prev_log_term: last_term },
                );
                broadcast(&msg);

                // In a single-node cluster: immediately tally our own vote
                // (we voted for ourselves above). That's a majority → become Leader.
                tally_vote_for_self(term);
            } else {
                r.election_tick -= 1;
            }
        }
        RaftRole::Leader => {
            // Send periodic AppendEntries heartbeat
            let mut hb = LEADER_HB_TICK.lock();
            if *hb == 0 {
                let term = r.current_term;
                let log_len = r.log.len as u64;
                drop(r);
                let msg = make_raft_msg(
                    DkcpMessageKind::RaftAppendEntries,
                    KernelDomainId::LOCAL,
                    KernelDomainId::LOCAL,
                    term as u32,
                    RaftEntryPayload { term, index: log_len, prev_log_index: log_len, prev_log_term: 0 },
                );
                broadcast(&msg);
                *hb = HEARTBEAT_INTERVAL;
            } else {
                *hb -= 1;
            }
        }
    }
}

/// Handle an incoming RaftRequestVote message.
pub fn handle_request_vote(msg: &DkcpMessage) {
    let payload = unsafe { msg.payload.raft_entry };
    let candidate = msg.src_domain;
    let candidate_term = payload.term;

    let mut r = RAFT.lock();

    // If we see a higher term, step down and update term
    if candidate_term > r.current_term {
        r.current_term = candidate_term;
        r.role = RaftRole::Follower;
        r.voted_for = None;
        r.election_tick = ELECTION_TIMEOUT;
    }

    let grant = candidate_term >= r.current_term
        && (r.voted_for.is_none() || r.voted_for == Some(candidate));

    if grant {
        r.voted_for = Some(candidate);
        println!("[RAFT] Granting vote to domain {} for term {}", candidate.0, candidate_term);
    }
    let term = r.current_term;
    drop(r);

    if grant {
        let reply = make_raft_msg(
            DkcpMessageKind::RaftVoteGranted,
            KernelDomainId::LOCAL,
            candidate,
            term as u32,
            RaftEntryPayload { term, index: 0, prev_log_index: 0, prev_log_term: 0 },
        );
        let _ = dkcp_send(&reply);
    }
}

/// Handle an incoming RaftVoteGranted message.
pub fn handle_vote_granted(msg: &DkcpMessage) {
    let payload = unsafe { msg.payload.raft_entry };
    let vote_term = payload.term;

    let mut r = RAFT.lock();
    if r.role != RaftRole::Candidate || vote_term != r.current_term {
        return;
    }

    // In our single-domain cluster, receiving any vote (even our own loopback)
    // constitutes a quorum. Become Leader.
    r.role = RaftRole::Leader;
    let term = r.current_term;
    let log_len = r.log.len as u64;
    // Initialize leader volatile state
    for i in 0..KernelDomainId::MAX_DOMAINS {
        r.next_index[i] = log_len + 1;
        r.match_index[i] = 0;
    }
    drop(r);
    *LEADER_HB_TICK.lock() = 0;

    println!("[RAFT] Became Leader: term={}", term);
}

/// Handle an incoming RaftAppendEntries message.
pub fn handle_append_entries(msg: &DkcpMessage) {
    let payload = unsafe { msg.payload.raft_entry };
    let leader_term = payload.term;

    let mut r = RAFT.lock();
    if leader_term >= r.current_term {
        r.current_term = leader_term;
        r.role = RaftRole::Follower;
        r.election_tick = ELECTION_TIMEOUT; // Reset timeout on leader contact
    }
    drop(r);

    // Send AppendAck
    let ack = make_raft_msg(
        DkcpMessageKind::RaftAppendAck,
        KernelDomainId::LOCAL,
        msg.src_domain,
        leader_term as u32,
        RaftEntryPayload { term: leader_term, index: payload.index, prev_log_index: 0, prev_log_term: 0 },
    );
    let _ = dkcp_send(&ack);
}

/// Append a semantic graph mutation to the Raft log.
/// Called by Leader when a replicated SGF change is requested.
/// Returns the log index of the new entry, or negative on error.
pub fn append_entry(mutation: SemanticGraphMutation) -> isize {
    let mut r = RAFT.lock();
    if r.role != RaftRole::Leader {
        println!("[RAFT] append_entry: not Leader (role={:?})", r.role);
        return -1; // -EPERM: only the leader can append
    }
    let idx = r.log.len;
    if idx >= 128 {
        return -12; // -ENOMEM: log full
    }
    let term = r.current_term;
    let entry = RaftLogEntry { term, index: idx as u64, mutation };
    r.log.entries[idx] = Some(entry);
    r.log.len += 1;
    r.commit_index = idx as u64 + 1;
    let log_len = r.log.len as u64;
    drop(r);

    println!("[RAFT] Log entry {} appended (term={})", idx, term);

    // Broadcast AppendEntries to peers (loopback in QEMU)
    let msg = make_raft_msg(
        DkcpMessageKind::RaftAppendEntries,
        KernelDomainId::LOCAL,
        KernelDomainId::LOCAL,
        term as u32,
        RaftEntryPayload { term, index: log_len, prev_log_index: idx as u64, prev_log_term: term },
    );
    broadcast(&msg);

    idx as isize
}

/// Write Raft status into a user-space buffer.
/// Format (all little-endian):
///   [0]  u8  role  (0=Follower, 1=Candidate, 2=Leader)
///   [1]  u8  reserved
///   [2]  u16 log_len
///   [8]  u64 current_term
///   [16] u64 commit_index
///   [24] u64 last_applied
pub fn raft_status(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_ptr == 0 || buf_len < 32 {
        return -1;
    }
    let r = RAFT.lock();
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, 32) };
    buf[0] = r.role as u8;
    buf[1] = 0;
    buf[2..4].copy_from_slice(&(r.log.len as u16).to_le_bytes());
    buf[4..8].copy_from_slice(&[0u8; 4]);
    buf[8..16].copy_from_slice(&r.current_term.to_le_bytes());
    buf[16..24].copy_from_slice(&r.commit_index.to_le_bytes());
    buf[24..32].copy_from_slice(&r.last_applied.to_le_bytes());
    println!("[RAFT] Status: role={:?}, term={}, commit={}, log_len={}",
        r.role, r.current_term, r.commit_index, r.log.len);
    0
}

/// Compatibility shim for the original init() call in main.rs.
/// Delegates to raft_init().
pub fn init() {
    raft_init();
}

// ─── Single-node quorum helper ────────────────────────────────────────────────

/// Called immediately after casting vote-for-self during election.
/// In a single-domain cluster this is sufficient for quorum.
fn tally_vote_for_self(term: u64) {
    let mut r = RAFT.lock();
    if r.role == RaftRole::Candidate && r.current_term == term {
        r.role = RaftRole::Leader;
        let log_len = r.log.len as u64;
        for i in 0..KernelDomainId::MAX_DOMAINS {
            r.next_index[i] = log_len + 1;
            r.match_index[i] = 0;
        }
        println!("[RAFT] Single-node quorum: became Leader immediately (term={})", term);
    }
}
