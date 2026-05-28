//! Phase 11 — Remote NES Graph Node Dispatch.
//!
//! Provides three operations:
//!   dispatch_node  — send a NES node to a remote domain (or self via loopback)
//!   wait_remote    — block-poll until the dispatched node's result arrives
//!   abort_remote   — cancel a pending remote dispatch
//!
//! In single-QEMU loopback mode, dispatch_node immediately enqueues a synthetic
//! GraphNodeResult message after the dispatch, simulating round-trip completion.
//! This exercises all real code paths without requiring a second QEMU instance.
//!
//! Ticket pool: 16 concurrent in-flight remote dispatches.

use spin::Mutex;
use super::types::{
    DkcpMessage, DkcpMessageKind, DkcpPayload, GraphNodeDispatchPayload,
    KernelDomainId,
};
use super::transport::{dkcp_send, dkcp_recv};
use crate::sbi::get_time;
use crate::println;

// ─── Ticket pool ─────────────────────────────────────────────────────────────

/// Status of a remote dispatch ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketStatus {
    Free,
    Pending,
    Complete,
    Aborted,
}

/// An in-flight remote dispatch record.
#[derive(Clone, Copy, Debug)]
pub struct DistTicket {
    pub ticket_id: u8,
    pub graph_id:  u32,
    pub node_id:   u16,
    pub seq:       u32,
    pub status:    TicketStatus,
}

impl DistTicket {
    const fn free(id: u8) -> Self {
        Self { ticket_id: id, graph_id: 0, node_id: 0, seq: 0, status: TicketStatus::Free }
    }
}

const MAX_TICKETS: usize = 16;

pub struct TicketPool {
    pub tickets: [DistTicket; MAX_TICKETS],
    pub seq:     u32,
}

impl TicketPool {
    pub const fn new() -> Self {
        const INIT: DistTicket = DistTicket {
            ticket_id: 0, graph_id: 0, node_id: 0, seq: 0, status: TicketStatus::Free,
        };
        let mut tickets = [INIT; MAX_TICKETS];
        let mut i = 0;
        while i < MAX_TICKETS {
            tickets[i].ticket_id = i as u8;
            i += 1;
        }
        Self { tickets, seq: 0 }
    }

    fn next_seq(&mut self) -> u32 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }

    fn alloc(&mut self, graph_id: u32, node_id: u16) -> Option<u8> {
        for t in self.tickets.iter_mut() {
            if t.status == TicketStatus::Free {
                t.graph_id = graph_id;
                t.node_id = node_id;
                t.seq = self.seq;
                self.seq = self.seq.wrapping_add(1);
                t.status = TicketStatus::Pending;
                return Some(t.ticket_id);
            }
        }
        None
    }

    fn find_by_seq(&mut self, seq: u32) -> Option<&mut DistTicket> {
        self.tickets.iter_mut().find(|t| t.seq == seq && t.status == TicketStatus::Pending)
    }
}

pub static TICKETS: Mutex<TicketPool> = Mutex::new(TicketPool::new());

// ─── Loopback result injection ────────────────────────────────────────────────

/// After dispatching a node, immediately enqueue a synthetic GraphNodeResult
/// on the loopback ring. This simulates remote completion for QEMU testing.
fn inject_loopback_result(seq: u32) {
    let result_msg = DkcpMessage {
        kind: DkcpMessageKind::GraphNodeResult,
        src_domain: KernelDomainId::LOCAL,
        dst_domain: KernelDomainId::LOCAL,
        seq,
        mac: [0u8; 16],
        payload: DkcpPayload { raw: [0u8; 32] },
    };
    // A ring full condition just means the caller will time out — acceptable.
    let _ = dkcp_send(&result_msg);
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Dispatch a NES graph node to a remote domain.
///
/// Returns ticket_id (≥0) on success, negative on error.
pub fn dispatch_node(graph_handle: usize, node_id: usize, remote_domain: usize) -> isize {
    let graph_id = graph_handle as u32;
    let n_id = node_id as u16;

    // Allocate ticket
    let (ticket_id, seq) = {
        let mut pool = TICKETS.lock();
        match pool.alloc(graph_id, n_id) {
            None => return -12, // ENOMEM: ticket pool exhausted
            Some(tid) => {
                let seq = pool.tickets[tid as usize].seq;
                (tid, seq)
            }
        }
    };

    // Build dispatch payload
    let payload = GraphNodeDispatchPayload {
        origin_graph_id: graph_id,
        origin_node_id:  n_id,
        op_type:         0, // generic — real impl would carry from TaskGraph
        device_hint:     0,
        data_size_bytes: 0,
        bulk_offset:     0,
        dep_mask:        0,
        _pad:            [0u8; 10],
    };

    let dispatch_msg = DkcpMessage {
        kind: DkcpMessageKind::GraphNodeDispatch,
        src_domain: KernelDomainId::LOCAL,
        dst_domain: KernelDomainId(remote_domain as u16),
        seq,
        mac: [0u8; 16],
        payload: DkcpPayload { node_dispatch: payload },
    };

    if let Err(_) = dkcp_send(&dispatch_msg) {
        println!("[NES_DIST] dispatch_node: ring full, aborting");
        let mut pool = TICKETS.lock();
        pool.tickets[ticket_id as usize].status = TicketStatus::Aborted;
        return -5; // EIO
    }

    // Loopback: inject synthetic result so wait_remote can complete
    inject_loopback_result(seq);

    println!("[NES_DIST] Dispatched graph={} node={} to domain={} ticket={}",
        graph_id, n_id, remote_domain, ticket_id);
    ticket_id as isize
}

/// Wait for a remote graph node result.
///
/// Polls the DKCP recv ring until a GraphNodeResult with matching seq arrives
/// or timeout_us microseconds elapse (measured via SBI get_time).
/// Returns 0 on success, -110 (ETIMEDOUT) on timeout.
pub fn wait_remote(graph_handle: usize, node_id: usize, timeout_us: usize) -> isize {
    let graph_id = graph_handle as u32;
    let n_id = node_id as u16;

    // Find the ticket for this graph+node
    let seq = {
        let pool = TICKETS.lock();
        pool.tickets.iter()
            .find(|t| t.graph_id == graph_id && t.node_id == n_id && t.status == TicketStatus::Pending)
            .map(|t| t.seq)
    };
    let seq = match seq {
        Some(s) => s,
        None => {
            println!("[NES_DIST] wait_remote: no pending ticket for graph={} node={}", graph_id, n_id);
            return -9; // EBADF
        }
    };

    // Compute deadline (0 = infinite wait, mapped to large timeout)
    let deadline_ticks = if timeout_us == usize::MAX || timeout_us == 0 {
        u64::MAX
    } else {
        // RISC-V mtime ticks at ~10MHz on QEMU virt; 1 us ≈ 10 ticks
        get_time().saturating_add(timeout_us as u64 * 10)
    };

    loop {
        // Check if already marked complete by process_incoming
        {
            let pool = TICKETS.lock();
            let t = &pool.tickets[seq as usize % MAX_TICKETS];
            if t.seq == seq && t.status == TicketStatus::Complete {
                println!("[NES_DIST] wait_remote: ticket seq={} complete", seq);
                return 0;
            }
            if t.seq == seq && t.status == TicketStatus::Aborted {
                return -125; // ECANCELED
            }
        }

        // Try to drain one message from ring
        if let Some(msg) = dkcp_recv() {
            if msg.kind == DkcpMessageKind::GraphNodeResult && msg.seq == seq {
                let mut pool = TICKETS.lock();
                if let Some(t) = pool.find_by_seq(seq) {
                    t.status = TicketStatus::Complete;
                }
                println!("[NES_DIST] wait_remote: result received for seq={}", seq);
                return 0;
            } else {
                // Route other messages through process_incoming logic inline
                drop_or_route(msg);
            }
        }

        // Timeout check
        if get_time() >= deadline_ticks {
            println!("[NES_DIST] wait_remote: timeout for seq={}", seq);
            return -110; // ETIMEDOUT
        }

        // Yield CPU briefly (wfi would stall; just spin for now since we have no async)
        core::hint::spin_loop();
    }
}

/// Abort a pending remote dispatch.
pub fn abort_remote(graph_handle: usize, node_id: usize) -> isize {
    let graph_id = graph_handle as u32;
    let n_id = node_id as u16;

    let seq = {
        let mut pool = TICKETS.lock();
        let mut found_seq = None;
        for t in pool.tickets.iter_mut() {
            if t.graph_id == graph_id && t.node_id == n_id && t.status == TicketStatus::Pending {
                t.status = TicketStatus::Aborted;
                found_seq = Some(t.seq);
                break;
            }
        }
        found_seq
    };

    match seq {
        None => {
            println!("[NES_DIST] abort_remote: no pending ticket for graph={} node={}", graph_id, n_id);
            -9 // EBADF
        }
        Some(s) => {
            // Send GraphNodeAbort to the remote domain
            let abort_msg = DkcpMessage {
                kind: DkcpMessageKind::GraphNodeAbort,
                src_domain: KernelDomainId::LOCAL,
                dst_domain: KernelDomainId::LOCAL,
                seq: s,
                mac: [0u8; 16],
                payload: DkcpPayload { raw: [0u8; 32] },
            };
            let _ = dkcp_send(&abort_msg);
            println!("[NES_DIST] abort_remote: ticket seq={} aborted", s);
            0
        }
    }
}

/// Drain the DKCP recv ring and dispatch each message to the right handler.
/// Called from the timer interrupt to process incoming cluster messages.
pub fn process_incoming() {
    while let Some(msg) = dkcp_recv() {
        dispatch_message(msg);
    }
}

/// Route one message to its handler.
fn dispatch_message(msg: DkcpMessage) {
    match msg.kind {
        DkcpMessageKind::Hello       => super::cluster::handle_hello(&msg),
        DkcpMessageKind::Heartbeat   => super::cluster::handle_heartbeat(&msg),

        DkcpMessageKind::RaftRequestVote  => super::raft::handle_request_vote(&msg),
        DkcpMessageKind::RaftVoteGranted  => super::raft::handle_vote_granted(&msg),
        DkcpMessageKind::RaftAppendEntries => super::raft::handle_append_entries(&msg),
        DkcpMessageKind::RaftAppendAck    => { /* leader bookkeeping — no-op in loopback */ }

        DkcpMessageKind::GraphNodeResult  => {
            let mut pool = TICKETS.lock();
            if let Some(t) = pool.find_by_seq(msg.seq) {
                t.status = TicketStatus::Complete;
            }
        }
        DkcpMessageKind::GraphNodeAbort   => {
            let mut pool = TICKETS.lock();
            if let Some(t) = pool.find_by_seq(msg.seq) {
                t.status = TicketStatus::Aborted;
            }
        }

        DkcpMessageKind::CapExportRequest  => super::dctp::handle_cap_export_request(&msg),
        DkcpMessageKind::CapRevokeNotify   => super::dctp::handle_cap_revoke_notify(&msg),

        _ => { /* Unhandled kinds are silently dropped */ }
    }
}

/// Route a message that arrived during wait_remote's spin loop (non-blocking).
fn drop_or_route(msg: DkcpMessage) {
    dispatch_message(msg);
}
