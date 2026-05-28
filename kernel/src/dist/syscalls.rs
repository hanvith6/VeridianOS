//! Phase 11 — Distributed System Call Handlers.
//!
//! Each function is called by the syscall dispatcher (syscall/mod.rs).
//! All implementations delegate to the appropriate dist sub-module.

use crate::println;
use super::{cluster, dctp, nes_dist, raft};
use super::types::SemanticGraphMutation;

// ─── Domain management syscalls (90–92) ──────────────────────────────────────

/// Syscall 90 — Join a distributed cluster domain.
///
/// Args:
///   a0 = domain_name_ptr  (u8* — name string in user memory)
///   a1 = domain_name_len  (usize — length of name string)
///   a2 = ip_addr_ptr      (u8* — reserved, ignored in loopback mode)
///   a3 = ip_addr_len      (usize — reserved)
///   a4 = port             (usize — reserved)
///
/// Returns: domain_id (≥0) on success, negative on error.
pub fn sys_domain_join(
    domain_name_ptr: usize,
    domain_name_len: usize,
    _ip_addr_ptr: usize,
    _ip_addr_len: usize,
    _port: usize,
) -> isize {
    println!("[DIST] sys_domain_join: name_ptr=0x{:X} len={}", domain_name_ptr, domain_name_len);
    cluster::domain_join(domain_name_ptr, domain_name_len)
}

/// Syscall 91 — List active nodes in the cluster domain.
///
/// Args:
///   a0 = buf_ptr       (u8* — output buffer in user memory)
///   a1 = buf_len       (usize — length of buffer)
///   a2 = out_count_ptr (usize* — written with number of domains; may be 0)
///
/// Returns: number of active domains on success, negative on error.
pub fn sys_domain_list(
    buf_ptr: usize,
    buf_len: usize,
    out_count_ptr: usize,
) -> isize {
    println!("[DIST] sys_domain_list: buf_ptr=0x{:X} buf_len={}", buf_ptr, buf_len);
    let count = cluster::domain_list(buf_ptr, buf_len);
    // Write count to out_count_ptr if provided
    if out_count_ptr != 0 && count >= 0 {
        unsafe {
            let ptr = out_count_ptr as *mut u32;
            ptr.write_volatile(count as u32);
        }
    }
    count
}

/// Syscall 92 — Get cluster domain status.
///
/// Args:
///   a0 = status_buf_ptr  (u8* — 8-byte output buffer)
///   a1 = status_buf_len  (usize)
///
/// Returns: 0 on success, negative on error.
pub fn sys_domain_status(
    status_buf_ptr: usize,
    status_buf_len: usize,
) -> isize {
    println!("[DIST] sys_domain_status: buf_ptr=0x{:X}", status_buf_ptr);
    cluster::domain_status(status_buf_ptr, status_buf_len)
}

// ─── Remote NES graph dispatch syscalls (93–95) ───────────────────────────────

/// Syscall 93 — Dispatch a NES node to a remote domain.
///
/// Args:
///   a0 = graph_handle     (usize — local NES graph handle ID)
///   a1 = node_id          (usize — node index within the graph)
///   a2 = remote_domain_id (usize — target KernelDomainId)
///
/// Returns: ticket_id (≥0) on success, negative on error.
pub fn sys_graph_dispatch_remote(
    graph_handle: usize,
    node_id: usize,
    remote_domain_id: usize,
) -> isize {
    println!("[DIST] sys_graph_dispatch_remote: graph={} node={} domain={}",
        graph_handle, node_id, remote_domain_id);
    nes_dist::dispatch_node(graph_handle, node_id, remote_domain_id)
}

/// Syscall 94 — Wait for a remote NES node to complete.
///
/// Args:
///   a0 = graph_handle  (usize)
///   a1 = node_id       (usize)
///   a2 = timeout_us    (usize — usize::MAX means infinite)
///
/// Returns: 0 on success, -110 (ETIMEDOUT) on timeout, other negatives on error.
pub fn sys_graph_wait_remote(
    graph_handle: usize,
    node_id: usize,
    timeout_us: usize,
) -> isize {
    println!("[DIST] sys_graph_wait_remote: graph={} node={} timeout={}",
        graph_handle, node_id, timeout_us);
    nes_dist::wait_remote(graph_handle, node_id, timeout_us)
}

/// Syscall 95 — Abort a remote NES node dispatch.
///
/// Args:
///   a0 = graph_handle  (usize)
///   a1 = node_id       (usize)
///
/// Returns: 0 on success, negative on error.
pub fn sys_graph_abort_remote(
    graph_handle: usize,
    node_id: usize,
) -> isize {
    println!("[DIST] sys_graph_abort_remote: graph={} node={}", graph_handle, node_id);
    nes_dist::abort_remote(graph_handle, node_id)
}

// ─── Distributed capability transfer syscalls (96–98) ────────────────────────

/// Syscall 96 — Export a local capability handle to a remote domain.
///
/// Args:
///   a0 = handle_id            (usize — local capability handle)
///   a1 = target_domain_id     (usize — destination KernelDomainId)
///   a2 = out_remote_token_ptr (usize* — written with 8-byte UID token)
///
/// Returns: positive UID token (truncated to isize) on success, negative on error.
pub fn sys_cap_export(
    handle_id: usize,
    target_domain_id: usize,
    out_remote_token_ptr: usize,
) -> isize {
    println!("[DIST] sys_cap_export: handle={} target_domain={}", handle_id, target_domain_id);
    let token = dctp::cap_export(handle_id, target_domain_id);
    if token >= 0 && out_remote_token_ptr != 0 {
        unsafe {
            let ptr = out_remote_token_ptr as *mut u64;
            ptr.write_volatile(token as u64);
        }
    }
    token
}

/// Syscall 97 — Import a capability from a remote domain using its UID.
///
/// Args:
///   a0 = remote_token_ptr  (u8* — pointer to 8-byte UID token)
///   a1 = remote_token_len  (usize — must be ≥8)
///   a2 = src_domain_id     (usize — originating domain)
///
/// Returns: new local handle_id (≥0) on success, negative on error.
pub fn sys_cap_import(
    remote_token_ptr: usize,
    remote_token_len: usize,
    src_domain_id: usize,
) -> isize {
    println!("[DIST] sys_cap_import: token_ptr=0x{:X} len={} src_domain={}",
        remote_token_ptr, remote_token_len, src_domain_id);
    dctp::cap_import(remote_token_ptr, remote_token_len, src_domain_id)
}

/// Syscall 98 — Revoke a previously exported capability on a remote domain.
///
/// Args:
///   a0 = handle_id         (usize — local handle that was exported)
///   a1 = target_domain_id  (usize)
///
/// Returns: 0 on success, negative on error.
pub fn sys_cap_revoke_remote(
    handle_id: usize,
    target_domain_id: usize,
) -> isize {
    println!("[DIST] sys_cap_revoke_remote: handle={} target_domain={}", handle_id, target_domain_id);
    dctp::cap_revoke(handle_id, target_domain_id)
}

// ─── Semantic graph replication syscalls (99–101) ────────────────────────────

/// Syscall 99 — Enable Semantic Graph replication via Raft.
///
/// Args:
///   a0 = enable    (usize — 1 to enable, 0 to disable)
///   a1 = strategy  (usize — reserved; 0 = Raft, 1 = Gossip)
///
/// Returns: 0 on success (always succeeds — Raft is always running).
pub fn sys_sgf_replicate_enable(
    enable: usize,
    strategy: usize,
) -> isize {
    println!("[DIST] sys_sgf_replicate_enable: enable={} strategy={}", enable, strategy);
    if enable == 1 {
        // Append a NodeCreate mutation to the Raft log as a replication marker
        let idx = raft::append_entry(SemanticGraphMutation::NodeCreate {
            node_type: 0xFF, // sentinel: "replication enable" marker
            label_hash: strategy as u64,
        });
        if idx >= 0 {
            println!("[DIST] SGF replication enabled via Raft (log_idx={})", idx);
        }
        if idx < 0 { idx } else { 0 }
    } else {
        println!("[DIST] SGF replication disabled (no-op in loopback mode)");
        0
    }
}

/// Syscall 100 — Query replication status of a semantic graph node.
///
/// Args:
///   a0 = query_ptr      (u8* — input: u32 node_id to query)
///   a1 = query_len      (usize — must be ≥4)
///   a2 = out_status_ptr (u8* — output: u32 status; 0=not replicated, 1=replicated)
///
/// Returns: 0 on success, negative on error.
pub fn sys_sgf_replicate_query(
    query_ptr: usize,
    query_len: usize,
    out_status_ptr: usize,
) -> isize {
    println!("[DIST] sys_sgf_replicate_query: query_ptr=0x{:X} len={}", query_ptr, query_len);
    if query_ptr == 0 || query_len < 4 {
        return -22; // EINVAL
    }
    let node_id = unsafe { *(query_ptr as *const u32) };
    // Check if any Raft log entry covers this node_id
    let r = raft::RAFT.lock();
    let mut replicated = 0u32;
    for entry_opt in r.log.entries.iter().take(r.log.len) {
        if let Some(entry) = entry_opt {
            match entry.mutation {
                SemanticGraphMutation::NodeCreate { node_type: _, label_hash } => {
                    if label_hash == node_id as u64 { replicated = 1; }
                }
                SemanticGraphMutation::BlobUpdate { node_id: n, .. } if n == node_id => {
                    replicated = 1;
                }
                _ => {}
            }
        }
    }
    drop(r);
    if out_status_ptr != 0 {
        unsafe { *(out_status_ptr as *mut u32) = replicated; }
    }
    println!("[DIST] node={} replication_status={}", node_id, replicated);
    replicated as isize
}

/// Syscall 101 — Query local Raft consensus status.
///
/// Args:
///   a0 = status_buf_ptr  (u8* — 32-byte output buffer; see raft::raft_status)
///   a1 = status_buf_len  (usize — must be ≥32)
///
/// Returns: 0 on success, negative on error.
pub fn sys_sgf_raft_status(
    status_buf_ptr: usize,
    status_buf_len: usize,
) -> isize {
    println!("[DIST] sys_sgf_raft_status: buf_ptr=0x{:X} len={}", status_buf_ptr, status_buf_len);
    raft::raft_status(status_buf_ptr, status_buf_len)
}
