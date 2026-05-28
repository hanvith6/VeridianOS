//! SBI ecall dispatcher for the VeridianOS M-mode monitor.
//!
//! When S-mode executes `ecall` with EID = 0x08424B45 ("BKE" — VeridianOS
//! Enclave extension), the M-mode trap vector calls `dispatch()` here.
//! All other EIDs are forwarded to OpenSBI via a software call chain
//! (or handled minimally if the monitor runs as the sole M-mode firmware).
//!
//! ## SBI ABI (from SBI Specification v2.0)
//!
//! ```text
//!   ecall instruction
//!   a7 = EID  (Extension ID)
//!   a6 = FID  (Function ID within the extension)
//!   a0..a5 = arguments
//!   Return: a0 = SBI_ERRx (0 = success), a1 = return value
//! ```
//!
//! ## VeridianOS Enclave Extension (EID 0x08424B45)
//!
//! | FID | Function             | Args                           | Returns     |
//! |-----|----------------------|--------------------------------|-------------|
//! |  0  | enclave_create       | a0=phys_addr, a1=size, a2=entry | a1=enclave_id |
//! |  1  | enclave_enter        | a0=enclave_id                  | —           |
//! |  2  | enclave_exit         | a0=enclave_id                  | —           |
//! |  3  | enclave_attest       | a0=enclave_id, a1=report_phys  | —           |
//!
//! All functions return `SBI_SUCCESS (0)` in a0 on success or a negative
//! `SBI_ERR_*` code on failure. On success, any additional output value is
//! in a1.

use crate::enclave;

/// EID for the VeridianOS Enclave SBI extension.
/// ASCII "BKE" packed big-endian: 0x08 (BEL) 0x42 ('B') 0x4B ('K') 0x45 ('E')
pub const VERIDIAN_ENCLAVE_EID: usize = 0x08424B45;

/// Standard SBI timer extension EID (for pass-through).
const SBI_EID_TIMER: usize = 0x54494D45;
/// Standard SBI console putchar EID (legacy, for pass-through).
const SBI_EID_LEGACY_PUTCHAR: usize = 0x01;

/// Return type for an SBI call: (error_code, value).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SbiRet {
    pub error: isize,
    pub value: isize,
}

impl SbiRet {
    pub const fn ok(value: isize) -> Self {
        Self { error: 0, value }
    }

    pub const fn err(code: isize) -> Self {
        Self { error: code, value: 0 }
    }
}

/// Main SBI ecall dispatcher invoked from the M-mode trap handler.
///
/// # Parameters
///
/// - `eid`: Extension ID (from a7).
/// - `fid`: Function ID (from a6).
/// - `a0..a2`: Arguments from registers a0, a1, a2.
///
/// # Returns
///
/// `SbiRet { error, value }` which the trap handler writes back to a0/a1.
pub fn dispatch(eid: usize, fid: usize, a0: usize, a1: usize, a2: usize) -> SbiRet {
    match eid {
        VERIDIAN_ENCLAVE_EID => dispatch_enclave(fid, a0, a1, a2),

        // Pass-through to OpenSBI (or handle inline) for standard extensions.
        // In a standalone monitor (no OpenSBI), implement the minimum subset.
        SBI_EID_TIMER => handle_timer(fid, a0),
        SBI_EID_LEGACY_PUTCHAR => handle_putchar(a0),

        // Probe: any unknown EID returns SBI_ERR_NOT_SUPPORTED.
        _ => SbiRet::err(enclave::SBI_ERR_NOT_SUPPORTED),
    }
}

// -----------------------------------------------------------------------
// VeridianOS Enclave Extension handlers
// -----------------------------------------------------------------------

fn dispatch_enclave(fid: usize, a0: usize, a1: usize, a2: usize) -> SbiRet {
    match fid {
        0 => handle_enclave_create(a0, a1, a2),
        1 => handle_enclave_enter(a0),
        2 => handle_enclave_exit(a0),
        3 => handle_enclave_attest(a0, a1),
        _ => SbiRet::err(enclave::SBI_ERR_NOT_SUPPORTED),
    }
}

/// FID 0: `enclave_create(phys_addr, size, entry_pa)` → `enclave_id`
///
/// The kernel calls this after allocating a physically contiguous region and
/// copying the enclave binary into it. The monitor:
///   1. Validates alignment and size constraints.
///   2. Computes SHA-256 measurement over [phys_addr, phys_addr+size).
///   3. Configures a PMP entry granting S-mode R/W (no X) for binary loading.
///   4. Allocates an enclave slot and returns its ID in a1.
///
/// # Security
///
/// The kernel MUST NOT modify enclave memory after calling `enclave_create` if
/// it wants the remote attestation measurement to be valid. The measurement
/// is locked at creation time; any post-creation modification will make the
/// attestation report inconsistent with the actual running code.
fn handle_enclave_create(phys_addr: usize, size: usize, entry_pa: usize) -> SbiRet {
    // Safety: we are in M-mode, single-threaded trap handler.
    match unsafe { enclave::enclave_create(phys_addr, size, entry_pa) } {
        Ok(id)   => SbiRet::ok(id as isize),
        Err(code) => SbiRet::err(code),
    }
}

/// FID 1: `enclave_enter(enclave_id)`
///
/// Seals the enclave PMP region and drops to U-mode at the enclave entry point.
/// The kernel passes the current mepc (its own return address) implicitly via
/// the trap frame — the monitor saves it for restore on enclave_exit.
///
/// After this call the CPU is running in U-mode inside the enclave. The
/// kernel will not regain control until the enclave issues `enclave_exit`.
///
/// # Security
///
/// The monitor verifies the enclave is in `Created` state (not yet entered
/// and not already Running) before proceeding. Attempting to enter a running
/// or exited enclave returns `SBI_ERR_DENIED`.
fn handle_enclave_enter(enclave_id: usize) -> SbiRet {
    if enclave_id > u8::MAX as usize {
        return SbiRet::err(enclave::SBI_ERR_INVALID_PARAM);
    }

    // Read current mepc and mstatus from CSRs to save the kernel context.
    let kernel_mepc:    usize;
    let kernel_mstatus: usize;
    unsafe {
        core::arch::asm!("csrr {}, mepc",    out(reg) kernel_mepc);
        core::arch::asm!("csrr {}, mstatus", out(reg) kernel_mstatus);
    }

    // enclave_enter writes the new mepc (entry_pa) and mstatus (U-mode) CSRs.
    // The trap handler's `mret` at the end of the ecall path will then jump to
    // the enclave entry point in U-mode.
    match unsafe {
        enclave::enclave_enter(enclave_id as u8, kernel_mepc, kernel_mstatus)
    } {
        Ok(()) => SbiRet::ok(0),
        Err(code) => SbiRet::err(code),
    }
}

/// FID 2: `enclave_exit(enclave_id)`
///
/// Called from inside an enclave to return to S-mode (the kernel). The monitor:
///   1. Unlocks the PMP region so the kernel can reclaim the memory.
///   2. Restores the saved kernel mepc so the kernel resumes at the
///      instruction following its `SBI_ENCLAVE_ENTER` ecall.
///   3. Sets mstatus.MPP = S-mode so `mret` returns to the kernel.
///
/// # Security
///
/// After this call, the enclave's physical memory is accessible to S-mode.
/// The kernel is responsible for zeroing or freeing it before reuse.
fn handle_enclave_exit(enclave_id: usize) -> SbiRet {
    if enclave_id > u8::MAX as usize {
        return SbiRet::err(enclave::SBI_ERR_INVALID_PARAM);
    }

    match unsafe { enclave::enclave_exit(enclave_id as u8) } {
        Ok((saved_mepc, saved_mstatus)) => {
            // Restore the kernel's return context so the trap handler's mret
            // takes the CPU back to S-mode at the right address.
            unsafe {
                core::arch::asm!("csrw mepc, {}", in(reg) saved_mepc);
                core::arch::asm!("csrw mstatus, {}", in(reg) saved_mstatus);
            }
            SbiRet::ok(0)
        }
        Err(code) => SbiRet::err(code),
    }
}

/// FID 3: `enclave_attest(enclave_id, report_phys)`
///
/// Fills the 73-byte attestation report at `report_phys` with:
///   - enclave_id (1 byte)
///   - phys_start  (8 bytes LE)
///   - size        (8 bytes LE)
///   - measurement (32 bytes SHA-256)
///   - signature   (24 bytes HMAC-SHA-256 truncated)
///
/// The kernel is responsible for copying the report to user space. The
/// physical address `report_phys` must be in kernel memory (S-mode accessible).
///
/// # Security
///
/// The signature uses `DEVICE_KEY` — in production this must be a key
/// provisioned in OTP memory and never exposed to S-mode. The remote verifier
/// checks the HMAC against the expected device public key to confirm the
/// measurement is authentic.
fn handle_enclave_attest(enclave_id: usize, report_phys: usize) -> SbiRet {
    if enclave_id > u8::MAX as usize {
        return SbiRet::err(enclave::SBI_ERR_INVALID_PARAM);
    }
    if report_phys == 0 {
        return SbiRet::err(enclave::SBI_ERR_INVALID_ADDRESS);
    }

    match unsafe { enclave::enclave_attest(enclave_id as u8, report_phys) } {
        Ok(()) => SbiRet::ok(0),
        Err(code) => SbiRet::err(code),
    }
}

// -----------------------------------------------------------------------
// Minimal standard SBI handlers (inline, no OpenSBI dependency)
// -----------------------------------------------------------------------

/// Handle SBI timer extension (EID 0x54494D45, FID 0: sbi_set_timer).
///
/// Programs the RISC-V CLINT timer compare register (mtimecmp) to fire a
/// timer interrupt at `time_val`.
fn handle_timer(fid: usize, time_val: usize) -> SbiRet {
    if fid != 0 {
        return SbiRet::err(enclave::SBI_ERR_NOT_SUPPORTED);
    }

    // QEMU virt: CLINT mtimecmp for hart 0 lives at 0x0200_4000.
    // In production read the CLINT address from the DTB.
    const CLINT_MTIMECMP: *mut u64 = 0x0200_4000 as *mut u64;
    unsafe {
        core::ptr::write_volatile(CLINT_MTIMECMP, time_val as u64);
        // Clear pending timer interrupt in mip.
        core::arch::asm!("csrc mip, {}", in(reg) 1usize << 7); // MTIP
    }

    // Raise STIP so S-mode sees a supervisor timer interrupt.
    unsafe {
        core::arch::asm!("csrs mip, {}", in(reg) 1usize << 5); // STIP
    }

    SbiRet::ok(0)
}

/// Handle SBI legacy putchar (EID 0x01).
///
/// Writes a single byte to the UART. Used by the kernel before it has its
/// own UART driver (early boot console).
fn handle_putchar(ch: usize) -> SbiRet {
    // QEMU virt UART0 at 0x1000_0000 (NS16550-compatible).
    const UART_THR: *mut u8 = 0x1000_0000 as *mut u8;
    unsafe {
        core::ptr::write_volatile(UART_THR, ch as u8);
    }
    SbiRet::ok(0)
}
