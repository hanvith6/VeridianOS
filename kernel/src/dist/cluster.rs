//! Phase 11 — Cluster Membership and Liveness Tracking.
//!
//! Maintains a registry of up to 8 kernel domains (KernelDomainId 0–7).
//! Domain 0 is always the local bootstrap domain. In QEMU single-instance
//! mode all traffic flows through the loopback transport ring.

use spin::Mutex;
use super::types::{
    DkcpMessage, DkcpMessageKind, DkcpPayload, KernelDomainId,
};
use super::transport::dkcp_send;
use crate::println;

// ─── Domain liveness ─────────────────────────────────────────────────────────

/// Number of missed heartbeat ticks before a domain is declared dead.
const LIVENESS_THRESHOLD: u32 = 5;

/// Status of a registered domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainStatus {
    Active,
    Dead,
}

/// Per-domain registration record.
#[derive(Clone, Copy, Debug)]
pub struct DomainInfo {
    pub domain_id:      KernelDomainId,
    pub status:         DomainStatus,
    /// Liveness counter: decremented by heartbeat_tick, reset on Hello/Heartbeat rx.
    pub liveness_epoch: u32,
    /// Human-readable name (up to 15 bytes, null-terminated).
    pub name:           [u8; 16],
}

impl DomainInfo {
    pub const fn new_local() -> Self {
        let mut name = [0u8; 16];
        // "local\0"
        name[0] = b'l'; name[1] = b'o'; name[2] = b'c';
        name[3] = b'a'; name[4] = b'l';
        Self {
            domain_id: KernelDomainId::LOCAL,
            status: DomainStatus::Active,
            liveness_epoch: LIVENESS_THRESHOLD,
            name,
        }
    }
}

// ─── Cluster state ────────────────────────────────────────────────────────────

pub struct ClusterState {
    pub local_id:     KernelDomainId,
    pub domains:      [Option<DomainInfo>; KernelDomainId::MAX_DOMAINS],
    pub domain_count: u8,
    /// Monotonic sequence number for outgoing messages.
    pub seq:          u32,
}

impl ClusterState {
    pub const fn new() -> Self {
        const NONE: Option<DomainInfo> = None;
        Self {
            local_id: KernelDomainId::LOCAL,
            domains: [NONE; KernelDomainId::MAX_DOMAINS],
            domain_count: 0,
            seq: 0,
        }
    }

    fn next_seq(&mut self) -> u32 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }

    /// Find the slot index for a domain ID, or None if not registered.
    pub fn find(&self, id: KernelDomainId) -> Option<usize> {
        for (i, slot) in self.domains.iter().enumerate() {
            if let Some(d) = slot {
                if d.domain_id == id {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Register a new domain (or re-activate a dead one). Returns slot index.
    pub fn register(&mut self, id: KernelDomainId, name: &[u8]) -> Result<usize, &'static str> {
        // Update existing entry
        if let Some(idx) = self.find(id) {
            let d = self.domains[idx].as_mut().unwrap();
            d.status = DomainStatus::Active;
            d.liveness_epoch = LIVENESS_THRESHOLD;
            return Ok(idx);
        }
        // Find empty slot
        for (i, slot) in self.domains.iter_mut().enumerate() {
            if slot.is_none() {
                let mut entry = DomainInfo::new_local();
                entry.domain_id = id;
                entry.status = DomainStatus::Active;
                entry.liveness_epoch = LIVENESS_THRESHOLD;
                let copy_len = name.len().min(15);
                entry.name[..copy_len].copy_from_slice(&name[..copy_len]);
                entry.name[copy_len] = 0;
                *slot = Some(entry);
                self.domain_count += 1;
                return Ok(i);
            }
        }
        Err("cluster: domain table full")
    }

    /// Reset liveness epoch for a domain (called when Heartbeat/Hello rx).
    pub fn touch(&mut self, id: KernelDomainId) {
        if let Some(idx) = self.find(id) {
            if let Some(d) = self.domains[idx].as_mut() {
                d.liveness_epoch = LIVENESS_THRESHOLD;
                d.status = DomainStatus::Active;
            }
        }
    }
}

/// Global cluster state.
pub static CLUSTER: Mutex<ClusterState> = Mutex::new(ClusterState::new());

// ─── Public API ───────────────────────────────────────────────────────────────

/// Initialize the cluster membership state with the local domain ID.
/// Registers Domain 0 (local) and sends a Hello announcement on the loopback.
pub fn cluster_init(domain_id: KernelDomainId) {
    let mut cs = CLUSTER.lock();
    cs.local_id = domain_id;
    cs.register(domain_id, b"local").expect("cluster_init: register failed");

    // Build Hello message
    let seq = cs.next_seq();
    drop(cs);

    let hello = DkcpMessage {
        kind: DkcpMessageKind::Hello,
        src_domain: domain_id,
        dst_domain: KernelDomainId::LOCAL,
        seq,
        mac: [0u8; 16],
        payload: DkcpPayload { raw: [0u8; 32] },
    };

    println!("[CLUSTER] Domain {} initialized. Sending Hello on loopback.", domain_id.0);
    let _ = dkcp_send(&hello);
}

/// Periodic heartbeat tick, called from the timer interrupt.
/// - Decrements liveness epochs for all remote peers.
/// - Marks peers as Dead when epoch reaches 0.
/// - Sends a Heartbeat message from the local domain.
pub fn heartbeat_tick() {
    let mut cs = CLUSTER.lock();
    let local_id = cs.local_id;
    let seq = cs.next_seq();

    for slot in cs.domains.iter_mut() {
        if let Some(d) = slot {
            if d.domain_id == local_id {
                continue; // Don't time out ourselves
            }
            if d.status == DomainStatus::Active {
                if d.liveness_epoch == 0 {
                    d.status = DomainStatus::Dead;
                    println!("[CLUSTER] Domain {} declared dead (liveness timeout).", d.domain_id.0);
                } else {
                    d.liveness_epoch -= 1;
                }
            }
        }
    }
    drop(cs);

    // Send Heartbeat on loopback (in a real cluster, broadcast to all peers)
    let hb = DkcpMessage {
        kind: DkcpMessageKind::Heartbeat,
        src_domain: local_id,
        dst_domain: local_id, // loopback
        seq,
        mac: [0u8; 16],
        payload: DkcpPayload { raw: [0u8; 32] },
    };
    let _ = dkcp_send(&hb);
}

/// Syscall-backing: register a new domain with the given name.
/// Returns the assigned domain ID on success, negative on error.
/// Validate that a user-supplied pointer + length range lies within
/// the user-space virtual address window `[0x4000_0000, 0x8000_0000)`.
/// Returns false for null pointers, kernel-space addresses, or overflow.
fn validate_user_buf(ptr: usize, len: usize) -> bool {
    const USER_START: usize = 0x4000_0000;
    const USER_END:   usize = 0x8000_0000;
    if ptr == 0 || len == 0 { return false; }
    let end = match ptr.checked_add(len) { Some(e) => e, None => return false };
    ptr >= USER_START && end <= USER_END
}

pub fn domain_join(name_ptr: usize, name_len: usize) -> isize {
    let name_bytes = if name_len == 0 {
        b"unknown" as &[u8]
    } else {
        let len = name_len.min(15);
        // Validate pointer is within mapped user-space before dereferencing.
        if !validate_user_buf(name_ptr, len) {
            return -14; // EFAULT
        }
        // SAFETY: validate_user_buf confirmed [name_ptr, name_ptr+len) is in
        // the user address window and len <= 15.
        unsafe { core::slice::from_raw_parts(name_ptr as *const u8, len) }
    };

    let mut cs = CLUSTER.lock();
    // Assign next available domain ID
    let next_id = cs.domain_count;
    if next_id as usize >= KernelDomainId::MAX_DOMAINS {
        return -1; // ENOMEM
    }
    let id = KernelDomainId(next_id as u16);
    match cs.register(id, name_bytes) {
        Ok(_) => {
            println!("[CLUSTER] Domain {} joined as '{}'",
                id.0,
                core::str::from_utf8(&name_bytes[..name_bytes.len().min(15)]).unwrap_or("?"));
            id.0 as isize
        }
        Err(e) => {
            println!("[CLUSTER] domain_join error: {}", e);
            -1
        }
    }
}

/// Syscall-backing: write domain list into user buffer.
/// Format: [u32 count][DomainRecord × count]
/// DomainRecord = [u16 id][u8 status][u8 epoch][16 name bytes] = 20 bytes
pub fn domain_list(buf_ptr: usize, buf_len: usize) -> isize {
    let cs = CLUSTER.lock();
    // Count active domains
    let active: u32 = cs.domains.iter()
        .filter_map(|s| s.as_ref())
        .filter(|d| d.status == DomainStatus::Active)
        .count() as u32;

    let needed = 4 + active as usize * 20;
    if buf_len < needed || !validate_user_buf(buf_ptr, buf_len) {
        return active as isize; // Return count even if buffer too small / invalid
    }

    // SAFETY: validate_user_buf confirmed [buf_ptr, buf_ptr+buf_len) is within
    // the user address window.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };
    buf[0..4].copy_from_slice(&active.to_le_bytes());
    let mut off = 4usize;
    for slot in cs.domains.iter() {
        if let Some(d) = slot {
            if d.status == DomainStatus::Active && off + 20 <= buf_len {
                buf[off..off+2].copy_from_slice(&d.domain_id.0.to_le_bytes());
                buf[off+2] = d.status as u8;
                buf[off+3] = d.liveness_epoch.min(255) as u8;
                buf[off+4..off+20].copy_from_slice(&d.name);
                off += 20;
            }
        }
    }
    active as isize
}

/// Syscall-backing: write cluster status summary into user buffer.
/// Format: [u8 local_id][u8 domain_count][u8 active_count][u8 seq_hi][u32 seq]
pub fn domain_status(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_len < 8 || !validate_user_buf(buf_ptr, buf_len) {
        return -14; // EFAULT
    }
    let cs = CLUSTER.lock();
    let active = cs.domains.iter()
        .filter_map(|s| s.as_ref())
        .filter(|d| d.status == DomainStatus::Active)
        .count() as u8;
    // SAFETY: validate_user_buf confirmed the buffer is within user address space.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len.min(8)) };
    buf[0] = cs.local_id.0 as u8;
    buf[1] = cs.domain_count;
    buf[2] = active;
    buf[3] = 0; // reserved
    buf[4..8].copy_from_slice(&cs.seq.to_le_bytes());
    println!("[CLUSTER] Status: local={}, domains={}, active={}, seq={}",
        cs.local_id.0, cs.domain_count, active, cs.seq);
    0
}

/// Handle an incoming Hello message: register the sender as a live domain.
pub fn handle_hello(msg: &DkcpMessage) {
    let mut cs = CLUSTER.lock();
    let _ = cs.register(msg.src_domain, b"peer");
    println!("[CLUSTER] Hello from domain {}. Registered.", msg.src_domain.0);
}

/// Handle an incoming Heartbeat message: refresh sender liveness.
pub fn handle_heartbeat(msg: &DkcpMessage) {
    let mut cs = CLUSTER.lock();
    cs.touch(msg.src_domain);
}
