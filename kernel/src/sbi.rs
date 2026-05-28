//! OpenSBI (Supervisor Binary Interface) interface for VeridianOS
//!
//! Provides support for OpenSBI calls, specifically scheduling the hardware timer.
//!
//! References:
//! - RISC-V SBI Specification v2.0-rc1
//! - "OS in 1000 Lines" (SBI timer calls)

/// Low-level helper to execute an SBI call using the `ecall` assembly instruction.
#[inline]
fn sbi_call(extension: usize, function: usize, arg0: usize, arg1: usize) -> usize {
    let mut ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") extension,
            in("a6") function,
            in("a0") arg0,
            in("a1") arg1,
            lateout("a0") ret,
        );
    }
    ret
}

/// Set the hardware timer compare register to trigger a supervisor timer interrupt at the given time.
///
/// Parameters:
/// - `time`: The target absolute time value (based on `rdtime` ticks).
pub fn set_timer(time: u64) {
    // EID: 0x54494D45 (Timer Extension 'TIME' in ASCII)
    // FID: 0 (Set Timer function)
    sbi_call(0x54494D45, 0, time as usize, 0);
}

/// Read the current absolute hardware timer tick count.
#[inline]
pub fn get_time() -> u64 {
    let t;
    unsafe {
        // rdtime reads the 64-bit machine time counter register
        core::arch::asm!("rdtime {}", out(reg) t);
    }
    t
}

/// Send an inter-processor interrupt to the harts defined by the hart mask.
///
/// Parameters:
/// - `hart_mask`: A bitmask of harts to interrupt.
/// - `hart_mask_base`: The base hart ID for the mask.
pub fn sbi_send_ipi(hart_mask: usize, hart_mask_base: usize) {
    // EID: 0x735049 (sPI)
    // FID: 0 (send_ipi)
    sbi_call(0x735049, 0, hart_mask, hart_mask_base);
}

