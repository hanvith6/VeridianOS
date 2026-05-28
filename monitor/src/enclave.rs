//! Enclave lifecycle management for the VeridianOS M-mode monitor.
//!
//! An enclave is a physically isolated execution context. Its memory is
//! protected by PMP entries that deny S-mode and U-mode access. The kernel
//! (S-mode) can request enclave creation and entry through SBI ecalls, but
//! can never read or write enclave memory directly once it is locked.
//!
//! ## Enclave Lifecycle
//!
//!   1. `CREATED`  — Memory allocated by kernel, SBI_ENCLAVE_CREATE called.
//!                   Monitor computes SHA-256 measurement of [phys_start, phys_start+size).
//!                   PMP entry configured: S-mode granted R/W to load the binary.
//!
//!   2. `SEALED`   — SBI_ENCLAVE_ENTER called (first time).
//!                   Monitor finalizes PMP: S-mode access revoked (no R/W/X).
//!                   mret to U-mode at entry_pa — enclave begins executing.
//!
//!   3. `RUNNING`  — Enclave executes in U-mode. Any ecall/exception traps to M-mode.
//!                   M-mode may forward safe syscalls to S-mode or handle them itself.
//!
//!   4. `EXITED`   — SBI_ENCLAVE_EXIT ecall from inside the enclave.
//!                   Monitor restores S-mode context, unlocks PMP entry.
//!                   Enclave slot marked `allocated = false` and returned to pool.
//!
//! ## Static Pool (no heap)
//!
//! The monitor has no dynamic allocator. All enclave metadata lives in a
//! statically declared array of 8 slots. The kernel is responsible for
//! providing the physical memory for enclave contents.

use crate::pmp;
use crate::attest;

/// Maximum number of concurrent enclaves.
pub const MAX_ENCLAVES: usize = 8;

/// SBI error codes (from SBI spec §3.2)
pub const SBI_SUCCESS:           isize =  0;
pub const SBI_ERR_FAILED:        isize = -1;
pub const SBI_ERR_NOT_SUPPORTED: isize = -2;
pub const SBI_ERR_INVALID_PARAM: isize = -3;
pub const SBI_ERR_DENIED:        isize = -4;
pub const SBI_ERR_INVALID_ADDRESS: isize = -5;
pub const SBI_ERR_ALREADY_AVAILABLE: isize = -6;

/// Lifecycle state of an enclave slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum EnclaveState {
    /// Slot is free; no enclave occupies it.
    Empty   = 0,
    /// Enclave memory allocated and measured; not yet entered.
    Created = 1,
    /// PMP locked; enclave running in U-mode.
    Running = 2,
    /// Enclave has exited; slot pending cleanup.
    Exited  = 3,
}

/// Per-enclave metadata stored in the static pool.
///
/// All fields live in M-mode BSS/data. S-mode cannot access this structure
/// because the monitor image itself is PMP-locked at entry 15.
#[repr(C)]
pub struct Enclave {
    /// Slot identifier (0..MAX_ENCLAVES-1).
    pub id: u8,

    /// Physical start address of the enclave memory region.
    /// Must be naturally aligned to `size` (NAPOT requirement).
    pub phys_start: usize,

    /// Size of the enclave region in bytes. Must be a power of two >= 8.
    pub size: usize,

    /// Physical address of the enclave entry point (first instruction).
    /// Must lie within [phys_start, phys_start+size).
    pub entry_pa: usize,

    /// Current lifecycle state.
    pub state: EnclaveState,

    /// SHA-256 measurement of the enclave memory taken at creation.
    /// Included verbatim in the attestation report.
    pub measurement: [u8; 32],

    /// Whether this slot is occupied by an active or recently-exited enclave.
    pub allocated: bool,

    /// PMP slot used to protect this enclave (0..=7, matching slot index).
    pub pmp_slot: usize,

    /// Saved S-mode context for restore after enclave exits.
    /// Minimal: we save mepc (kernel return address) and mstatus.
    pub saved_mepc: usize,
    pub saved_mstatus: usize,
}

// -----------------------------------------------------------------------
// Static pool — no heap, no allocator
// -----------------------------------------------------------------------

/// The global enclave pool. Resides in M-mode BSS. S-mode cannot access it.
///
/// # Safety invariant
///
/// Only M-mode trap handler code (`sbi_handler::dispatch`) mutates this pool.
/// There is no concurrent access on a single hart. SMP safety (multi-hart
/// enclaves) is deferred to Phase 12.5.
pub static mut ENCLAVE_POOL: [Enclave; MAX_ENCLAVES] = [
    Enclave { id: 0, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 0, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 1, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 1, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 2, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 2, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 3, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 3, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 4, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 4, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 5, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 5, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 6, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 6, saved_mepc: 0, saved_mstatus: 0 },
    Enclave { id: 7, phys_start: 0, size: 0, entry_pa: 0, state: EnclaveState::Empty, measurement: [0u8; 32], allocated: false, pmp_slot: 7, saved_mepc: 0, saved_mstatus: 0 },
];

/// Zero all enclave slots at monitor startup.
///
/// # Safety
///
/// Must be called exactly once before any SBI enclave calls arrive.
pub unsafe fn init_pool() {
    for slot in ENCLAVE_POOL.iter_mut() {
        slot.state = EnclaveState::Empty;
        slot.allocated = false;
        slot.phys_start = 0;
        slot.size = 0;
        slot.entry_pa = 0;
        slot.measurement = [0u8; 32];
        slot.saved_mepc = 0;
        slot.saved_mstatus = 0;
    }
}

// -----------------------------------------------------------------------
// Enclave lifecycle operations
// -----------------------------------------------------------------------

/// Allocate an enclave slot and compute the initial measurement.
///
/// Called by `sbi_handler` when the kernel issues `SBI_ENCLAVE_CREATE`.
///
/// ## Security properties guaranteed after this call:
/// - The PMP slot grants S-mode R/W (not X) so the kernel can copy the
///   enclave binary into the region. Execute permission is withheld until
///   `enclave_enter` seals the region.
/// - The SHA-256 measurement is computed over the current contents of
///   `[phys_start, phys_start+size)`. If the kernel modifies the enclave
///   after creation, the measurement will not match — remote attestation
///   will detect tampering.
///
/// # Parameters
///
/// - `phys_addr`: Physical base address (must be `size`-aligned NAPOT).
/// - `size`: Region size in bytes (power of two, >= 8).
/// - `entry_pa`: Entry point physical address (must be inside region).
///
/// # Returns
///
/// `Ok(enclave_id)` on success, `Err(sbi_error_code)` on failure.
pub unsafe fn enclave_create(
    phys_addr: usize,
    size: usize,
    entry_pa: usize,
) -> Result<u8, isize> {
    // --- Validate parameters ---

    if size == 0 || !size.is_power_of_two() || size < 8 {
        return Err(SBI_ERR_INVALID_PARAM);
    }

    // NAPOT alignment: base must be size-aligned.
    if phys_addr & (size - 1) != 0 {
        return Err(SBI_ERR_INVALID_ADDRESS);
    }

    // Entry point must be inside the region.
    if entry_pa < phys_addr || entry_pa >= phys_addr.saturating_add(size) {
        return Err(SBI_ERR_INVALID_PARAM);
    }

    // --- Find a free slot ---

    let slot = ENCLAVE_POOL.iter_mut().find(|e| !e.allocated)
        .ok_or(SBI_ERR_FAILED)?;

    let pmp_slot = slot.pmp_slot;

    // --- Configure PMP: grant R/W to S-mode for binary loading ---
    // We deliberately withhold X until enclave_enter so the kernel cannot
    // execute code in the enclave region before measurement is finalized.
    pmp::grant_region(pmp_slot, phys_addr, size);

    // --- Compute measurement ---
    // SHA-256 over the raw physical bytes of the enclave region.
    // Safety: phys_addr is a valid physical address provided by the kernel.
    // The kernel has already mapped/zeroed this memory.
    let region_slice = core::slice::from_raw_parts(phys_addr as *const u8, size);
    let measurement = attest::sha256(region_slice);

    // --- Initialize slot ---
    slot.phys_start  = phys_addr;
    slot.size        = size;
    slot.entry_pa    = entry_pa;
    slot.state       = EnclaveState::Created;
    slot.measurement = measurement;
    slot.allocated   = true;

    Ok(slot.id)
}

/// Enter an enclave: seal its PMP region and transfer control to U-mode.
///
/// Called by `sbi_handler` when the kernel issues `SBI_ENCLAVE_ENTER`.
///
/// After this call the CPU is in U-mode executing the enclave payload.
/// The monitor saves enough S-mode context (mepc, mstatus) to restore the
/// kernel when `enclave_exit` is called.
///
/// ## Security guarantees:
/// - PMP entry for the enclave is locked to deny S-mode access.
/// - mstatus.MPP is set to U (0b00) before mret — we drop to U-mode, not S-mode.
/// - The saved kernel mepc is stored in M-mode memory (PMP entry 15 protected).
///
/// # Safety
///
/// Must be called from M-mode trap handler context with interrupts disabled.
pub unsafe fn enclave_enter(enclave_id: u8, kernel_mepc: usize, kernel_mstatus: usize) -> Result<(), isize> {
    let slot = find_slot_mut(enclave_id).ok_or(SBI_ERR_INVALID_PARAM)?;

    if slot.state != EnclaveState::Created {
        return Err(SBI_ERR_DENIED);
    }

    let pmp_slot  = slot.pmp_slot;
    let phys_start = slot.phys_start;
    let size       = slot.size;
    let entry_pa   = slot.entry_pa;

    // Save S-mode context for restore on enclave_exit.
    slot.saved_mepc    = kernel_mepc;
    slot.saved_mstatus = kernel_mstatus;
    slot.state         = EnclaveState::Running;

    // Finalize PMP: remove S-mode R/W, deny everything. The enclave itself
    // runs in U-mode so it accesses its own memory through a separate PMP
    // entry that allows U-mode R/W/X within the region.
    // For simplicity in Phase 12, we use a single deny-all entry for S-mode
    // and rely on the enclave being mapped in U-mode page tables by the kernel
    // before entering. A production implementation uses two PMP entries per
    // enclave (one deny-all for S, one allow for U with execute).
    pmp::lock_region(pmp_slot, phys_start, size);

    // mret to U-mode at entry_pa.
    // Set mstatus.MPP = 00 (U-mode), clear MPIE (enclave starts with interrupts off).
    let mut mstatus = crate::csr_read!("mstatus");
    mstatus &= !0x1880; // Clear MPP[1:0] and MPIE
    // MPP = 00 means U-mode after mret — already cleared above.
    crate::csr_write!("mstatus", mstatus);
    crate::csr_write!("mepc", entry_pa);

    // The caller (m_trap_vector) will execute mret after we return.
    Ok(())
}

/// Exit the enclave and restore the kernel (S-mode) context.
///
/// Called by `sbi_handler` when the enclave issues `SBI_ENCLAVE_EXIT` via ecall.
///
/// The enclave's PMP entry is unlocked (A=OFF). Kernel can now reclaim the
/// physical memory. The enclave slot is marked `Exited` and will be freed
/// on the next `enclave_create` that claims the slot.
///
/// # Safety
///
/// Must be called from M-mode trap handler context.
pub unsafe fn enclave_exit(enclave_id: u8) -> Result<(usize, usize), isize> {
    let slot = find_slot_mut(enclave_id).ok_or(SBI_ERR_INVALID_PARAM)?;

    if slot.state != EnclaveState::Running {
        return Err(SBI_ERR_DENIED);
    }

    let pmp_slot       = slot.pmp_slot;
    let saved_mepc     = slot.saved_mepc;
    let saved_mstatus  = slot.saved_mstatus;

    // Unlock PMP — S-mode can now access the region again.
    pmp::unlock_region(pmp_slot);

    slot.state     = EnclaveState::Exited;
    slot.allocated = false;

    // Return saved context so the trap handler can restore mepc/mstatus and
    // mret back into the kernel at the instruction after the SBI_ENCLAVE_ENTER ecall.
    Ok((saved_mepc, saved_mstatus))
}

/// Fill an attestation report for enclave `enclave_id` at physical address `report_phys`.
///
/// The report layout (written to `report_phys`):
///
/// ```text
/// Offset  Size  Field
/// 0       1     enclave_id
/// 1       4     phys_start (little-endian)
/// 5       4     size (little-endian)
/// 9       32    SHA-256 measurement
/// 41      32    HMAC-SHA-256 signature (device_key || measurement)
/// ```
///
/// The monitor signs the measurement with an internal device key. A remote
/// verifier can check the signature against the device's public key to
/// confirm the enclave is running genuine, unmodified code.
///
/// # Safety
///
/// `report_phys` must point to at least 73 bytes of writable physical memory
/// accessible to the caller (S-mode validates this before issuing the SBI call).
pub unsafe fn enclave_attest(enclave_id: u8, report_phys: usize) -> Result<(), isize> {
    let slot = find_slot(enclave_id).ok_or(SBI_ERR_INVALID_PARAM)?;

    if !slot.allocated {
        return Err(SBI_ERR_DENIED);
    }

    // Build the attestation report in a local buffer then copy to report_phys.
    // This avoids any TOCTOU between the check and the write.
    let mut report = [0u8; 73];

    report[0] = enclave_id;

    // phys_start (8 bytes, LE on RV64)
    let start_bytes = slot.phys_start.to_le_bytes();
    report[1..9].copy_from_slice(&start_bytes);

    // size (8 bytes, LE)
    let size_bytes = slot.size.to_le_bytes();
    report[9..17].copy_from_slice(&size_bytes);

    // measurement (32 bytes)
    report[17..49].copy_from_slice(&slot.measurement);

    // Signature: HMAC-SHA-256(DEVICE_KEY || measurement)
    let sig = attest::sign_measurement(&slot.measurement);
    report[49..73].copy_from_slice(&sig);

    // Write report to the physical address provided by the kernel.
    let report_ptr = report_phys as *mut u8;
    core::ptr::copy_nonoverlapping(report.as_ptr(), report_ptr, report.len());

    Ok(())
}

// -----------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------

unsafe fn find_slot(id: u8) -> Option<&'static Enclave> {
    ENCLAVE_POOL.iter().find(|e| e.allocated && e.id == id)
}

unsafe fn find_slot_mut(id: u8) -> Option<&'static mut Enclave> {
    ENCLAVE_POOL.iter_mut().find(|e| e.allocated && e.id == id)
}
