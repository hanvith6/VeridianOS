//! VeridianOS Phase 12 — M-mode TEE Security Monitor
//!
//! This is the M-mode firmware entry point. It sits below OpenSBI (or replaces it)
//! and implements the VeridianOS Enclave SBI extension (EID 0x08424B45 "BKE").
//!
//! Privilege levels on RISC-V:
//!   M-mode (Machine): This monitor — full hardware access, PMP authority
//!   S-mode (Supervisor): VeridianOS kernel — cannot access PMP CSRs
//!   U-mode (User): User processes and enclave payloads
//!
//! The monitor intercepts ecalls from S-mode with EID=0x08424B45 and dispatches
//! to the enclave lifecycle handlers. All other ecalls are forwarded to OpenSBI
//! (if present) or handled minimally.
//!
//! References:
//!   - Keystone Enclave (Lee et al., USENIX Security '20)
//!   - RISC-V Privileged Specification v1.12, §3.6 (PMP), §3.3 (M-mode traps)
//!   - SBI Specification v2.0 (ecall ABI, EID/FID convention)

#![no_std]
#![no_main]

pub mod attest;
pub mod enclave;
pub mod pmp;
pub mod sbi_handler;

use core::panic::PanicInfo;

/// RISC-V CSR read macro.
/// Reads a Control and Status Register into a usize.
macro_rules! csr_read {
    ($csr:literal) => {{
        let val: usize;
        unsafe {
            core::arch::asm!(
                concat!("csrr {}, ", $csr),
                out(reg) val,
            );
        }
        val
    }};
}

/// RISC-V CSR write macro.
macro_rules! csr_write {
    ($csr:literal, $val:expr) => {
        unsafe {
            core::arch::asm!(
                concat!("csrw ", $csr, ", {}"),
                in(reg) $val as usize,
            );
        }
    };
}

pub(crate) use csr_read;
pub(crate) use csr_write;

// -----------------------------------------------------------------------
// M-mode trap frame — saved by the assembly trap entry stub
// -----------------------------------------------------------------------

/// Saved register state for M-mode trap handling.
/// Layout must match the assembly save/restore sequence in entry.
///
/// Security note: this is stored on the M-mode stack, which S-mode and
/// U-mode cannot access (PMP entry 15 marks the monitor stack inaccessible
/// to lower privilege levels).
#[repr(C)]
pub struct TrapFrame {
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    /// a0: SBI return value (error code) / first argument
    pub a0: usize,
    /// a1: SBI return value (value) / second argument
    pub a1: usize,
    /// a2–a5: additional SBI arguments
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    /// a6: SBI FID (Function ID)
    pub a6: usize,
    /// a7: SBI EID (Extension ID)
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    /// mepc: address to return to after the trap
    pub mepc: usize,
}

// -----------------------------------------------------------------------
// M-mode entry point
// -----------------------------------------------------------------------

/// M-mode entry: a0 = hart_id, a1 = dtb_ptr
///
/// Called by QEMU before the kernel when this binary is passed as `-bios`.
/// We must:
///   1. Set up M-mode stack and trap vector
///   2. Configure PMP to protect monitor memory from S-mode
///   3. Delegate standard SBI extensions to OpenSBI pass-through (or handle inline)
///   4. Drop privilege to S-mode and jump to the kernel
///
/// For `cargo check` purposes the full assembly entry stub is stubbed out here.
/// A real deployment requires a linker script placing this at 0x80000000 and
/// an assembly trampoline that saves registers and calls `m_trap_handler`.
#[unsafe(no_mangle)]
pub extern "C" fn _start(hart_id: usize, dtb_ptr: usize) -> ! {
    // Safety: We are in M-mode at firmware entry. No other privilege level is active.

    // 1. Set mtvec to our trap handler (direct mode, bit 0 = 0).
    //    All M-mode exceptions and interrupts vector here.
    csr_write!("mtvec", m_trap_vector as *const () as usize);

    // 2. Lock the monitor's own memory with PMP entry 15 (highest priority).
    //    Monitor image lives at [0x80000000, 0x80040000) — 256 KiB.
    //    S-mode and U-mode must NOT read/write/execute this region.
    //    This protects the device key, enclave pool, and M-mode stack.
    unsafe {
        pmp::lock_monitor_self();
    }

    // 3. Initialize the enclave pool (zero all slots).
    unsafe {
        enclave::init_pool();
    }

    // 4. Delegate standard RISC-V interrupts and SBI extensions to S-mode.
    //    Any exception NOT listed in medeleg/mideleg is handled in M-mode.
    //    We keep ecalls from S-mode (cause = 9) in M-mode so we intercept them.
    unsafe {
        // Delegate all standard exceptions except M-mode ecall (cause 11)
        // and S-mode ecall (cause 9) — both stay in M-mode for our interception.
        // Bit mask: delegate everything except bits 9 and 11.
        let exceptions: usize = !((1 << 9) | (1 << 11));
        core::arch::asm!("csrw medeleg, {}", in(reg) exceptions);

        // Delegate supervisor-level timer/software/external interrupts to S-mode.
        let interrupts: usize = (1 << 1) | (1 << 5) | (1 << 9); // SSIP, STIP, SEIP
        core::arch::asm!("csrw mideleg, {}", in(reg) interrupts);
    }

    // 5. Set up mepc to jump to the kernel at 0x80200000 (typical OpenSBI handoff).
    //    In a real deployment the DTB scan would locate the kernel entry.
    //    For now we use the conventional QEMU virt layout address.
    let kernel_entry: usize = 0x8020_0000;
    csr_write!("mepc", kernel_entry);

    // 6. Set mstatus.MPP = 01 (S-mode) so mret drops us to S-mode.
    //    Also set mstatus.MPIE = 1 to enable interrupts after mret.
    let mut mstatus = csr_read!("mstatus");
    mstatus &= !0x1800; // Clear MPP[1:0]
    mstatus |= 0x0800;  // Set MPP = 01 (Supervisor)
    mstatus |= 0x0080;  // Set MPIE
    csr_write!("mstatus", mstatus);

    // 7. Pass hart_id and dtb_ptr to the kernel via a0/a1 (SBI convention).
    unsafe {
        core::arch::asm!(
            "mv a0, {hart}",
            "mv a1, {dtb}",
            "mret",
            hart = in(reg) hart_id,
            dtb  = in(reg) dtb_ptr,
            options(noreturn),
        );
    }
}

/// M-mode trap vector (direct mode).
///
/// In a production build this would be a naked assembly function that saves
/// all registers into a TrapFrame on the M-mode stack and calls `m_trap_handler`.
/// For cargo check / initial scaffold we expose the Rust-level handler directly
/// and mark the vector symbol with `no_mangle` so the linker can reference it.
#[unsafe(no_mangle)]
pub extern "C" fn m_trap_vector() -> ! {
    // Read trap cause from mcause CSR.
    let mcause = csr_read!("mcause");
    let mepc   = csr_read!("mepc");

    // Interrupt bit is the MSB on RV64.
    let is_interrupt = (mcause >> 63) != 0;
    let cause_code   = mcause & !(1 << 63);

    if !is_interrupt {
        match cause_code {
            // Ecall from S-mode (cause = 9).
            9 => {
                // Read SBI arguments from registers.
                // In a full implementation the assembly stub copies these into
                // the TrapFrame before calling here.
                let (a0, a1, a2, a6, a7): (usize, usize, usize, usize, usize);
                unsafe {
                    core::arch::asm!(
                        "mv {a0}, a0",
                        "mv {a1}, a1",
                        "mv {a2}, a2",
                        "mv {a6}, a6",
                        "mv {a7}, a7",
                        a0 = out(reg) a0,
                        a1 = out(reg) a1,
                        a2 = out(reg) a2,
                        a6 = out(reg) a6,
                        a7 = out(reg) a7,
                    );
                }

                let ret = sbi_handler::dispatch(a7, a6, a0, a1, a2);

                // Advance mepc past the ecall instruction (4 bytes).
                csr_write!("mepc", mepc.wrapping_add(4));

                // Write SBI return values back into a0 (error) and a1 (value).
                unsafe {
                    core::arch::asm!(
                        "mv a0, {err}",
                        "mv a1, {val}",
                        "mret",
                        err = in(reg) ret.error as usize,
                        val = in(reg) ret.value as usize,
                        options(noreturn),
                    );
                }
            }
            // All other synchronous exceptions: halt.
            _ => {
                loop { unsafe { core::arch::asm!("wfi"); } }
            }
        }
    } else {
        // Timer interrupt delegated to S-mode by setting mtip in mip.
        // For now simply mret — in production forward to S-mode via CLINT.
        unsafe { core::arch::asm!("mret", options(noreturn)); }
    }
}

/// Dispatcher entry called from trap vector after register save.
///
/// `frame` points to the TrapFrame on the M-mode stack. In a full assembly
/// implementation this is called as `call m_trap_handler` after saving ra.
#[unsafe(no_mangle)]
pub extern "C" fn m_trap_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };

    let mcause = csr_read!("mcause");
    let is_interrupt = (mcause >> 63) != 0;
    let cause_code   = mcause & !(1 << 63);

    if !is_interrupt && cause_code == 9 {
        // S-mode ecall: dispatch through the SBI handler.
        let ret = sbi_handler::dispatch(frame.a7, frame.a6, frame.a0, frame.a1, frame.a2);

        // Write return values into the saved register frame so they are
        // restored to a0/a1 by the assembly stub on the way out.
        frame.a0 = ret.error as usize;
        frame.a1 = ret.value as usize;

        // Advance saved mepc past the ecall instruction.
        frame.mepc = frame.mepc.wrapping_add(4);
    }
    // All other causes are handled by their delegated exception in S/U mode,
    // or cause a monitor halt for unexpected M-mode faults.
}

// -----------------------------------------------------------------------
// Panic handler — required by no_std
// -----------------------------------------------------------------------

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // M-mode panic: freeze the hart. In a production monitor this would
    // write a diagnostic to a reserved memory region and reset the board.
    loop {
        unsafe { core::arch::asm!("wfi"); }
    }
}
