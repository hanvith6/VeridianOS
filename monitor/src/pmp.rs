//! Physical Memory Protection (PMP) configuration for the M-mode monitor.
//!
//! PMP is the RISC-V mechanism by which M-mode restricts which physical
//! memory regions S-mode and U-mode can access. The monitor uses it to
//! create hardware-enforced enclave isolation: once `lock_region` is called
//! for an enclave's memory, even a fully-compromised kernel cannot read or
//! write that region.
//!
//! ## PMP on RISC-V (Privileged Spec §3.6)
//!
//! Each PMP entry consists of:
//!   - `pmpaddr<n>` CSR: the address (right-shifted by 2 for NAPOT)
//!   - `pmpcfg<n/4>` CSR: packed 8-bit config fields (R, W, X, A[1:0], L)
//!
//! Address modes:
//!   - OFF  (A=00): entry disabled
//!   - TOR  (A=01): top-of-range — [pmpaddr[n-1], pmpaddr[n])
//!   - NA4  (A=10): naturally aligned 4-byte region
//!   - NAPOT(A=11): naturally aligned power-of-two region (most efficient)
//!
//! ## NAPOT encoding
//!
//! For a region of size 2^k starting at address `base` (must be 2^k aligned):
//!   pmpaddr = (base >> 2) | ((1 << (k-3)) - 1)
//!
//! Example: 64 KiB region at 0x8800_0000:
//!   k = 16, pmpaddr = (0x8800_0000 >> 2) | 0x1FFF = 0x2200_1FFF
//!
//! ## PMP entry assignment in VeridianOS
//!
//!   Entry 0..=7 : enclave slots 0..=7 (dynamically assigned)
//!   Entry 8..=14: reserved for future use (kernel heap guard, etc.)
//!   Entry 15    : monitor self-protection (always locked, highest priority)
//!
//! The L (Lock) bit makes an entry immune to further modification until reset.
//! We set L on entry 15 (monitor self) immediately at boot and on enclave
//! entries only after the enclave is finalized.

use crate::{csr_read, csr_write};

// -----------------------------------------------------------------------
// PMP configuration bit fields (8-bit pmpcfg entry)
// -----------------------------------------------------------------------

/// PMP config: region is readable by S/U-mode
const PMP_R: u8 = 1 << 0;
/// PMP config: region is writable by S/U-mode
const PMP_W: u8 = 1 << 1;
/// PMP config: region is executable by S/U-mode
const PMP_X: u8 = 1 << 2;
/// Address mode bits [4:3]: NAPOT = 0b11
const PMP_A_NAPOT: u8 = 0b11 << 3;
/// Lock bit: entry cannot be modified while set (until reset)
const PMP_L: u8 = 1 << 7;

// -----------------------------------------------------------------------
// PMP register layout constants
// -----------------------------------------------------------------------

/// Number of PMP entries available (RISC-V spec guarantees at least 16 on
/// implementations that support PMP at all; QEMU virt provides 16).
pub const PMP_ENTRY_COUNT: usize = 16;

/// Entry index reserved for monitor self-protection.
const MONITOR_SELF_ENTRY: usize = 15;

/// Entry indices 0..=7 are used for the 8 enclave slots.
pub const ENCLAVE_PMP_BASE: usize = 0;

// -----------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------

/// Compute the NAPOT-encoded pmpaddr value for a naturally aligned power-of-two
/// region `[base, base + size)`.
///
/// # Panics (compile-time style)
///
/// `size` must be a power of two and >= 8. `base` must be `size`-aligned.
/// Violations produce an address that will silently map the wrong region; the
/// caller is responsible for supplying valid parameters.
#[inline]
fn napot_encode(base: usize, size: usize) -> usize {
    // pmpaddr = (base >> 2) | ((size/8) - 1)
    (base >> 2) | ((size / 8) - 1)
}

/// Read one 8-bit PMP configuration field from the packed CSR.
///
/// RISC-V packs 8 entries into each 64-bit `pmpcfgN` CSR on RV64:
///   pmpcfg0 holds entries 0-7, pmpcfg2 holds entries 8-15 (pmpcfg1/3 unused on RV64)
unsafe fn read_pmpcfg_byte(entry: usize) -> u8 {
    let csr_val: usize = match entry / 8 {
        0 => csr_read!("pmpcfg0"),
        _ => csr_read!("pmpcfg2"),
    };
    let byte_shift = (entry % 8) * 8;
    ((csr_val >> byte_shift) & 0xFF) as u8
}

/// Write one 8-bit PMP configuration field into the packed CSR.
/// Preserves all other entries in the same CSR.
unsafe fn write_pmpcfg_byte(entry: usize, cfg: u8) {
    let byte_shift = (entry % 8) * 8;
    let mask: usize = !(0xFF << byte_shift);

    match entry / 8 {
        0 => {
            let old = csr_read!("pmpcfg0");
            let new_val = (old & mask) | ((cfg as usize) << byte_shift);
            csr_write!("pmpcfg0", new_val);
        }
        _ => {
            let old = csr_read!("pmpcfg2");
            let new_val = (old & mask) | ((cfg as usize) << byte_shift);
            csr_write!("pmpcfg2", new_val);
        }
    }
}

/// Write a pmpaddr CSR by entry index (0..=15).
unsafe fn write_pmpaddr(entry: usize, addr: usize) {
    // We cannot use a single csrw with a runtime register number; each CSR
    // address is a compile-time constant in RISC-V assembly. Use a match to
    // select the correct instruction.
    match entry {
        0  => core::arch::asm!("csrw pmpaddr0,  {}", in(reg) addr),
        1  => core::arch::asm!("csrw pmpaddr1,  {}", in(reg) addr),
        2  => core::arch::asm!("csrw pmpaddr2,  {}", in(reg) addr),
        3  => core::arch::asm!("csrw pmpaddr3,  {}", in(reg) addr),
        4  => core::arch::asm!("csrw pmpaddr4,  {}", in(reg) addr),
        5  => core::arch::asm!("csrw pmpaddr5,  {}", in(reg) addr),
        6  => core::arch::asm!("csrw pmpaddr6,  {}", in(reg) addr),
        7  => core::arch::asm!("csrw pmpaddr7,  {}", in(reg) addr),
        8  => core::arch::asm!("csrw pmpaddr8,  {}", in(reg) addr),
        9  => core::arch::asm!("csrw pmpaddr9,  {}", in(reg) addr),
        10 => core::arch::asm!("csrw pmpaddr10, {}", in(reg) addr),
        11 => core::arch::asm!("csrw pmpaddr11, {}", in(reg) addr),
        12 => core::arch::asm!("csrw pmpaddr12, {}", in(reg) addr),
        13 => core::arch::asm!("csrw pmpaddr13, {}", in(reg) addr),
        14 => core::arch::asm!("csrw pmpaddr14, {}", in(reg) addr),
        _  => core::arch::asm!("csrw pmpaddr15, {}", in(reg) addr),
    }
}

// -----------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------

/// Configure PMP entry `slot` to make the region `[phys_start, phys_start+size)`
/// inaccessible to S-mode and U-mode.
///
/// The region must be naturally aligned and a power-of-two in size (NAPOT mode).
/// After this call, any S-mode or U-mode access to the region raises a PMP fault.
///
/// # Safety
///
/// Must be called from M-mode. `phys_start` must be `size`-aligned and `size`
/// must be a power of two >= 8 bytes.
pub unsafe fn lock_region(slot: usize, phys_start: usize, size: usize) {
    let addr = napot_encode(phys_start, size);
    write_pmpaddr(slot, addr);

    // No R/W/X permissions — deny access. NAPOT mode. No Lock bit yet;
    // locking prevents the monitor from later unlocking (we lock only at
    // enclave finalization, not at creation time so we can still write the
    // enclave image).
    let cfg = PMP_A_NAPOT; // R=0 W=0 X=0 A=NAPOT L=0
    write_pmpcfg_byte(slot, cfg);

    // sfence.vma ensures PMP changes take effect before any subsequent
    // S-mode memory access. RISC-V spec §3.6: PMP changes require a fence.
    core::arch::asm!("sfence.vma");
}

/// Remove the PMP lock on slot `slot`, restoring S-mode access to the region.
///
/// Called when an enclave exits and its memory is returned to the kernel.
/// Note: we can only unlock entries whose Lock bit (L) is NOT set. Entries
/// with L=1 require a full system reset to clear.
///
/// # Safety
///
/// Must be called from M-mode.
pub unsafe fn unlock_region(slot: usize) {
    // Disable the entry by setting A=OFF (all zeros).
    write_pmpcfg_byte(slot, 0);
    write_pmpaddr(slot, 0);
    core::arch::asm!("sfence.vma");
}

/// Configure PMP entry 15 to protect the monitor's own image and stack.
///
/// Monitor lives at [0x8000_0000, 0x8004_0000) — 256 KiB.
/// This entry uses the Lock bit so it cannot be tampered with even if M-mode
/// code is somehow redirected (defense-in-depth).
///
/// # Safety
///
/// Must be called exactly once at monitor startup before dropping to S-mode.
pub unsafe fn lock_monitor_self() {
    const MONITOR_BASE: usize = 0x8000_0000;
    const MONITOR_SIZE: usize = 256 * 1024; // 256 KiB

    let addr = napot_encode(MONITOR_BASE, MONITOR_SIZE);
    write_pmpaddr(MONITOR_SELF_ENTRY, addr);

    // No permissions to S/U, NAPOT, Lock bit set.
    let cfg = PMP_A_NAPOT | PMP_L;
    write_pmpcfg_byte(MONITOR_SELF_ENTRY, cfg);

    core::arch::asm!("sfence.vma");
}

/// Grant R/W/X access to S-mode for a region on the given PMP slot.
///
/// Used temporarily when the monitor needs to let the kernel load an enclave
/// binary into the enclave region before locking it. After loading, call
/// `lock_region` to remove the permissions.
///
/// # Safety
///
/// Must be called from M-mode.
pub unsafe fn grant_region(slot: usize, phys_start: usize, size: usize) {
    let addr = napot_encode(phys_start, size);
    write_pmpaddr(slot, addr);
    let cfg = PMP_R | PMP_W | PMP_X | PMP_A_NAPOT; // Full access, no Lock
    write_pmpcfg_byte(slot, cfg);
    core::arch::asm!("sfence.vma");
}

/// Check whether a PMP slot is currently active (A != OFF).
///
/// # Safety
///
/// Must be called from M-mode.
pub unsafe fn slot_is_active(slot: usize) -> bool {
    let cfg = read_pmpcfg_byte(slot);
    (cfg & (0b11 << 3)) != 0 // A field != OFF
}
