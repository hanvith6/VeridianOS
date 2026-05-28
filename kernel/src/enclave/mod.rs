//! VeridianOS Phase 12 — Kernel-side Enclave Syscall Handlers
//!
//! This module bridges S-mode user-space processes and the M-mode TEE monitor.
//! User processes call `SYS_ENCLAVE_*` syscalls; the kernel validates arguments
//! and then issues SBI ecalls (EID 0x08424B45) to the monitor.
//!
//! ## Trust Model
//!
//! ```text
//!   User Process (U-mode)
//!       │  SYS_ENCLAVE_* syscall (ecall, a7=120..123)
//!   Kernel (S-mode) — this module
//!       │  SBI ecall (ecall, a7=0x08424B45)
//!   M-mode Monitor — monitor/src/
//!       │  PMP configuration + attestation
//!   Hardware (RISC-V PMP)
//! ```
//!
//! The kernel cannot bypass PMP or forge attestation reports. Once the monitor
//! locks an enclave region, even a kernel exploit cannot read enclave memory.
//!
//! ## Syscall Argument Validation
//!
//! The kernel validates user-supplied pointers (report buffer) before passing
//! them to the monitor. Physical addresses are not validated here — the monitor
//! performs its own checks on the physical address space.
//!
//! ## Error Propagation
//!
//! SBI error codes from the monitor (negative isize values) are returned
//! directly to user space. Kernel validation errors use standard POSIX codes.

use crate::sbi::SbiRet;

// -----------------------------------------------------------------------
// VeridianOS Enclave SBI Extension
// -----------------------------------------------------------------------

/// EID for the VeridianOS Enclave SBI extension (same as monitor's constant).
/// ASCII "BKE" packed: 0x08424B45
const VERIDIAN_ENCLAVE_EID: usize = 0x08424B45;

// FIDs within the extension.
const FID_ENCLAVE_CREATE: usize = 0;
const FID_ENCLAVE_ENTER:  usize = 1;
const FID_ENCLAVE_EXIT:   usize = 2;
const FID_ENCLAVE_ATTEST: usize = 3;

// -----------------------------------------------------------------------
// Internal SBI call helper (3-argument variant, mirrors kernel/src/sbi.rs)
// -----------------------------------------------------------------------

/// Issue a 3-argument SBI ecall and return the (error, value) pair.
///
/// Follows the SBI v2.0 ABI:
///   a7 = EID, a6 = FID, a0..a2 = args
///   a0 = error on return, a1 = value on return
#[inline]
fn sbi_enclave_call(fid: usize, arg0: usize, arg1: usize, arg2: usize) -> SbiRet {
    let mut err: isize;
    let mut val: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") VERIDIAN_ENCLAVE_EID,
            in("a6") fid,
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            lateout("a0") err,
            lateout("a1") val,
        );
    }
    SbiRet { error: err, value: val }
}

// -----------------------------------------------------------------------
// Syscall handlers — called from kernel/src/syscall/mod.rs dispatcher
// -----------------------------------------------------------------------

/// `SYS_ENCLAVE_CREATE` (120)
///
/// Request the M-mode monitor to create a hardware TEE enclave.
///
/// # Parameters (from user-space registers a0..a2)
///
/// - `phys_addr`: Physical base address of the enclave region.
///   Must be naturally aligned to `size` (NAPOT constraint). The kernel is
///   responsible for allocating this physical region before calling this syscall.
///
/// - `size`: Size of the enclave region in bytes. Must be a power of two
///   and >= 8. Recommended minimum: 4 KiB for a real enclave.
///
/// - `entry_pa`: Physical address of the enclave entry point. Must lie
///   within [phys_addr, phys_addr + size).
///
/// # Returns
///
/// Non-negative `enclave_id` (u8, 0..=7) on success. The caller uses this
/// ID for subsequent `enter`, `exit`, and `attest` calls.
/// Negative value on error (SBI error code from the monitor).
///
/// # Security
///
/// The monitor computes a SHA-256 measurement of the enclave memory at this
/// point. If the kernel modifies the memory after `enclave_create` returns,
/// the attestation report will not match the actual running code, allowing
/// a remote verifier to detect tampering.
pub fn sys_enclave_create(phys_addr: usize, size: usize, entry_pa: usize) -> isize {
    // Basic sanity: size must be a non-zero power of two.
    if size == 0 || !size.is_power_of_two() {
        return -22; // -EINVAL
    }

    // Alignment: phys_addr must be size-aligned (NAPOT requirement).
    if phys_addr & (size - 1) != 0 {
        return -22; // -EINVAL
    }

    // Entry point must be inside the region.
    if entry_pa < phys_addr || entry_pa >= phys_addr.saturating_add(size) {
        return -22; // -EINVAL
    }

    let ret = sbi_enclave_call(FID_ENCLAVE_CREATE, phys_addr, size, entry_pa);

    if ret.error != 0 {
        // Propagate monitor error codes (negative SBI_ERR_*) to user space.
        return ret.error as isize;
    }

    crate::println!("[ENCLAVE] Created enclave id={} phys=0x{:X} size=0x{:X}", ret.value, phys_addr, size);
    ret.value as isize
}

/// `SYS_ENCLAVE_ENTER` (121)
///
/// Instruct the M-mode monitor to seal the enclave's PMP region and transfer
/// execution to the enclave entry point in U-mode.
///
/// This syscall blocks from the kernel's perspective — it returns to user space
/// only after the enclave issues `SYS_ENCLAVE_EXIT`. The monitor saves the
/// kernel's return address and restores it on exit.
///
/// # Parameters
///
/// - `enclave_id`: ID returned by `sys_enclave_create`.
///
/// # Security
///
/// Once the monitor processes this call, the enclave region is PMP-locked:
/// S-mode cannot read, write, or execute enclave memory until `enclave_exit`
/// is called. This guarantee holds even if the kernel is compromised.
pub fn sys_enclave_enter(enclave_id: usize) -> isize {
    if enclave_id > u8::MAX as usize {
        return -22; // -EINVAL
    }

    let ret = sbi_enclave_call(FID_ENCLAVE_ENTER, enclave_id, 0, 0);

    if ret.error != 0 {
        return ret.error as isize;
    }

    crate::println!("[ENCLAVE] Enclave {} entered (returned from monitor)", enclave_id);
    0
}

/// `SYS_ENCLAVE_EXIT` (122)
///
/// Called from inside the enclave (U-mode) to terminate execution and
/// return control to the S-mode kernel.
///
/// The monitor unlocks the PMP entry for this enclave, restores the kernel
/// context, and returns via `mret` to S-mode at the instruction following
/// the `SYS_ENCLAVE_ENTER` call.
///
/// After this call, the enclave's physical memory is accessible to the kernel
/// and should be zeroed before being reclaimed.
///
/// # Parameters
///
/// - `enclave_id`: The running enclave's ID.
pub fn sys_enclave_exit(enclave_id: usize) -> isize {
    if enclave_id > u8::MAX as usize {
        return -22; // -EINVAL
    }

    let ret = sbi_enclave_call(FID_ENCLAVE_EXIT, enclave_id, 0, 0);

    if ret.error != 0 {
        return ret.error as isize;
    }

    crate::println!("[ENCLAVE] Enclave {} exited cleanly", enclave_id);
    0
}

/// `SYS_ENCLAVE_ATTEST` (123)
///
/// Request the M-mode monitor to generate an attestation report for an enclave.
///
/// The report is written to `report_buf_ptr` (73 bytes):
///
/// ```text
/// Offset  Bytes  Field
/// 0       1      enclave_id
/// 1       8      phys_start (little-endian u64)
/// 9       8      size (little-endian u64)
/// 17      32     SHA-256 measurement of enclave memory (at creation time)
/// 49      24     HMAC-SHA-256 signature (device_key || measurement), truncated
/// ```
///
/// A remote verifier can:
///   1. Extract the measurement.
///   2. Verify the HMAC using the device public key.
///   3. Compare the measurement against a trusted reference image hash.
///   4. Conclude that the enclave is running genuine, unmodified code on
///      a legitimate VeridianOS device.
///
/// # Security
///
/// - The kernel validates `report_buf_ptr` is a writeable user-space buffer
///   before passing the physical address to the monitor.
/// - The monitor writes directly to the physical address — the kernel cannot
///   intercept or forge the report content.
/// - The HMAC device key never leaves M-mode memory.
pub fn sys_enclave_attest(enclave_id: usize, report_buf_ptr: usize) -> isize {
    if enclave_id > u8::MAX as usize || report_buf_ptr == 0 {
        return -22; // -EINVAL
    }

    // Validate the user-space buffer is accessible and writable.
    const REPORT_SIZE: usize = 73;
    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(report_buf_ptr, REPORT_SIZE, true)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

    // Convert the user virtual address to physical for the monitor.
    // The monitor writes the report directly to physical memory.
    // For now we pass the virtual address — in a full implementation this
    // should be translated via the current process's page table.
    // TODO(phase12): translate report_buf_ptr through proc.page_table to PA.
    let report_phys = report_buf_ptr; // Stub: assume identity map for now

    let ret = sbi_enclave_call(FID_ENCLAVE_ATTEST, enclave_id, report_phys, 0);

    if ret.error != 0 {
        return ret.error as isize;
    }

    crate::println!("[ENCLAVE] Attestation report generated for enclave {}", enclave_id);
    0
}
