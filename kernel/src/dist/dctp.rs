//! Phase 11 — Distributed Capability Transfer Protocol (DCTP).
//!
//! Provides three operations:
//!   cap_export  — publishes a local capability to the global UID table and
//!                 sends a CapExportRequest over the loopback transport.
//!   cap_import  — looks up a 128-bit UID in the local DIST_CAP_TABLE and
//!                 installs a shadow Handle into the calling process's table.
//!   cap_revoke  — bumps the epoch on an exported capability so all remote
//!                 shadows become invalid, and broadcasts CapRevokeNotify.
//!
//! UID derivation (no CSPRNG in bare-metal no_std):
//!   UID bytes 0-7  = rdtime (hardware cycle counter)
//!   UID bytes 8-9  = local domain id
//!   UID bytes 10-11 = handle_id as u16
//!   UID bytes 12-15 = monotonic counter (EXPORT_SEQ)
//! This gives collision-resistant UIDs for a research kernel. Production code
//! would use a VirtIO entropy device.

use spin::Mutex;
use super::types::{
    DkcpMessage, DkcpMessageKind, DkcpPayload, CapExportPayload,
    KernelDomainId, DistributedCapability, DIST_CAP_TABLE,
};
use super::transport::dkcp_send;
use crate::capability::{Handle, ObjectType, Rights};
use crate::process::with_current_process;
use crate::sbi::get_time;
use crate::println;

// ─── Monotonic UID sequence ───────────────────────────────────────────────────

static EXPORT_SEQ: Mutex<u32> = Mutex::new(0);

fn next_export_seq() -> u32 {
    let mut s = EXPORT_SEQ.lock();
    let v = *s;
    *s = s.wrapping_add(1);
    v
}

/// Derive a 128-bit UID without a CSPRNG.
fn derive_uid(handle_id: usize) -> [u8; 16] {
    let ticks: u64 = get_time();
    let seq:   u32 = next_export_seq();
    let mut uid = [0u8; 16];
    uid[0..8].copy_from_slice(&ticks.to_le_bytes());
    uid[8..10].copy_from_slice(&(KernelDomainId::LOCAL.0).to_le_bytes());
    uid[10..12].copy_from_slice(&(handle_id as u16).to_le_bytes());
    uid[12..16].copy_from_slice(&seq.to_le_bytes());
    uid
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Export a local capability handle to a remote domain.
///
/// Steps:
/// 1. Read the handle from CURRENT_PROCESS to get type and rights.
/// 2. Derive a 128-bit UID.
/// 3. Insert a DistributedCapability record into DIST_CAP_TABLE.
/// 4. Send a CapExportRequest DkcpMessage via loopback.
/// 5. Return the lower 8 bytes of the UID interpreted as u64, truncated to isize.
pub fn cap_export(handle_id: usize, _target_domain: usize) -> isize {
    // Step 1: read handle
    let res = with_current_process(|p| {
        match p.handle_table.get(handle_id) {
            Ok(h) => Ok((h.object_type, h.rights)),
            Err(_) => Err(-9), // EBADF
        }
    });

    let (obj_type, local_rights) = match res {
        Some(Ok(val)) => val,
        Some(Err(err)) => return err,
        None => return -3, // EPERM
    };

    // Step 2: UID
    let uid = derive_uid(handle_id);

    // Step 3: register in DIST_CAP_TABLE
    {
        let mut table = DIST_CAP_TABLE.lock();
        let mut inserted = false;
        for slot in table.caps.iter_mut() {
            if slot.is_none() {
                *slot = Some(DistributedCapability {
                    origin_domain: KernelDomainId::LOCAL,
                    global_uid: uid,
                    local_rights,
                    local_handle: Some(handle_id as u32),
                    epoch: 1,
                });
                inserted = true;
                break;
            }
        }
        if !inserted {
            println!("[DCTP] cap_export: DIST_CAP_TABLE full");
            return -12; // ENOMEM
        }
    }

    // Step 4: send CapExportRequest
    let payload = CapExportPayload {
        global_uid: uid,
        rights: local_rights.bits(),
        object_type: obj_type as u32,
        remote_handle: handle_id as u32,
        _pad: [0u8; 4],
    };
    let msg = DkcpMessage {
        kind: DkcpMessageKind::CapExportRequest,
        src_domain: KernelDomainId::LOCAL,
        dst_domain: KernelDomainId::LOCAL,
        seq: next_export_seq(),
        mac: [0u8; 16],
        payload: DkcpPayload { cap_export: payload },
    };
    let _ = dkcp_send(&msg);

    // Step 5: return lower 8 bytes of UID as token
    let token = u64::from_le_bytes(uid[0..8].try_into().unwrap_or([0u8; 8]));
    println!("[DCTP] cap_export: handle={} exported with UID token=0x{:X}", handle_id, token);
    (token & 0x7FFF_FFFF_FFFF_FFFF) as isize // ensure positive
}

/// Import a capability from a global UID token.
///
/// Steps:
/// 1. Read the 8-byte UID prefix from uid_ptr.
/// 2. Search DIST_CAP_TABLE for a matching entry (by first 8 UID bytes).
/// 3. Create a shadow Handle with the stored rights.
/// 4. Insert into CURRENT_PROCESS handle table.
/// 5. Return the new local handle_id.
pub fn cap_import(uid_ptr: usize, uid_len: usize, _src_domain: usize) -> isize {
    if uid_ptr == 0 || uid_len < 8 {
        return -22; // EINVAL
    }

    // Validate that uid_ptr is a valid user pointer
    let valid = with_current_process(|proc| {
        proc.validate_user_buffer(uid_ptr, 8, false)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

    let uid_prefix = unsafe {
        let bytes = core::slice::from_raw_parts(uid_ptr as *const u8, 8);
        let mut arr = [0u8; 8];
        arr.copy_from_slice(bytes);
        arr
    };

    // Search table by prefix match (first 8 bytes = time-derived, unique enough)
    let (found_type, found_rights) = {
        let table = DIST_CAP_TABLE.lock();
        let mut result = None;
        for slot in table.caps.iter() {
            if let Some(cap) = slot {
                if cap.global_uid[0..8] == uid_prefix {
                    result = Some((cap.origin_domain, cap.local_rights));
                    break;
                }
            }
        }
        match result {
            Some(r) => r,
            None => {
                println!("[DCTP] cap_import: UID not found in DIST_CAP_TABLE");
                return -2; // ENOENT
            }
        }
    };

    // Install shadow Handle into calling process
    let shadow = Handle::new(
        ObjectType::VirtualMemoryObject, // generic shadow type
        uid_ptr, // point back to the UID buffer as a nominal address
        found_rights,
    );

    match with_current_process(|p| {
        match p.handle_table.insert(shadow) {
            Ok(id) => id as isize,
            Err(_) => -12, // ENOMEM
        }
    }) {
        Some(res) => {
            if res >= 0 {
                println!("[DCTP] cap_import: installed shadow handle={} (origin_domain={})",
                    res, found_type.0);
            }
            res
        }
        None => -3, // EPERM
    }
}

/// Revoke a distributed capability.
///
/// Steps:
/// 1. Find the UID in DIST_CAP_TABLE by local_handle == handle_id.
/// 2. Increment epoch (invalidates remote shadows).
/// 3. Send CapRevokeNotify via loopback.
pub fn cap_revoke(handle_id: usize, _target_domain: usize) -> isize {
    let uid = {
        let mut table = DIST_CAP_TABLE.lock();
        let mut found_uid = None;
        for slot in table.caps.iter_mut() {
            if let Some(cap) = slot {
                if cap.local_handle == Some(handle_id as u32) {
                    cap.epoch += 1;
                    found_uid = Some(cap.global_uid);
                    break;
                }
            }
        }
        match found_uid {
            Some(u) => u,
            None => {
                println!("[DCTP] cap_revoke: handle {} not found in DIST_CAP_TABLE", handle_id);
                return -9; // EBADF
            }
        }
    };

    // Send CapRevokeNotify
    let payload = CapExportPayload {
        global_uid: uid,
        rights: 0,
        object_type: 0,
        remote_handle: handle_id as u32,
        _pad: [0u8; 4],
    };
    let msg = DkcpMessage {
        kind: DkcpMessageKind::CapRevokeNotify,
        src_domain: KernelDomainId::LOCAL,
        dst_domain: KernelDomainId::LOCAL,
        seq: next_export_seq(),
        mac: [0u8; 16],
        payload: DkcpPayload { cap_export: payload },
    };
    let _ = dkcp_send(&msg);
    println!("[DCTP] cap_revoke: handle={} revoked, CapRevokeNotify sent", handle_id);
    0
}

/// Handle an incoming CapExportRequest (peer is exporting a cap to us).
pub fn handle_cap_export_request(msg: &DkcpMessage) {
    let payload = unsafe { msg.payload.cap_export };
    let uid = payload.global_uid;

    // Register in our local DIST_CAP_TABLE as a remote entry
    let rights = Rights::from_bits_truncate(payload.rights);
    let mut table = DIST_CAP_TABLE.lock();
    for slot in table.caps.iter_mut() {
        if slot.is_none() {
            *slot = Some(DistributedCapability {
                origin_domain: msg.src_domain,
                global_uid: uid,
                local_rights: rights,
                local_handle: None, // not yet imported
                epoch: 1,
            });
            println!("[DCTP] Registered incoming cap from domain {}", msg.src_domain.0);
            return;
        }
    }
    println!("[DCTP] handle_cap_export_request: table full, dropping");
}

/// Handle an incoming CapRevokeNotify (peer revoked a cap).
pub fn handle_cap_revoke_notify(msg: &DkcpMessage) {
    let payload = unsafe { msg.payload.cap_export };
    let uid = payload.global_uid;
    let mut table = DIST_CAP_TABLE.lock();
    for slot in table.caps.iter_mut() {
        if let Some(cap) = slot {
            if cap.global_uid == uid {
                cap.epoch += 1;
                cap.local_handle = None; // shadow invalidated
                println!("[DCTP] Cap revoked by domain {}, epoch bumped", msg.src_domain.0);
                return;
            }
        }
    }
}
