//! Trap handling and privilege transition logic for VeridianOS
//!
//! Handles exceptions and interrupts in Supervisor mode, and implements the
//! transition from Supervisor mode (S-mode) to User mode (U-mode).

use crate::process::thread;

/// The Structured TrapFrame saved onto the stack on trap entry.
/// Must match the assembly offsets in `trap.S` exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrapFrame {
    /// General purpose registers x0-x31 (x0 is unused/placeholder).
    pub regs: [usize; 32],
    /// Supervisor Status register.
    pub sstatus: usize,
    /// Supervisor Exception Program Counter.
    pub sepc: usize,
}

// External assembly symbols
unsafe extern "C" {
    /// Entry point defined in trap.S
    fn trap_vector();
}

/// Configure the trap vector address by loading the address of `trap_vector` into `stvec`.
pub fn init() {
    unsafe {
        core::arch::asm!("csrw stvec, {}", in(reg) trap_vector as *const () as usize);
        // Enable SUM (Supervisor User Memory access) so kernel can read/write user pages
        core::arch::asm!("csrs sstatus, {}", in(reg) 0x40000);
        // Enable STIE (Supervisor Timer Interrupt Enable) in sie (bit 5)
        core::arch::asm!("csrs sie, {}", in(reg) 0x20);
        // Schedule first timer interrupt (10ms / 100,000 ticks in the future)
        crate::sbi::set_timer(crate::sbi::get_time() + 100_000);
        crate::println!(
            "[TRAP] Trap vector initialized: stvec = 0x{:X}",
            trap_vector as *const () as usize
        );
    }
}

/// High-level trap handler called from assembly.
///
/// Parameters:
/// - `tf`: Pointer to the TrapFrame saved on the stack.
///
/// # Safety
///
/// The caller must guarantee that the pointer to the trap frame is valid
/// and corresponds to a properly saved stack frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trap_handler(tf: *mut TrapFrame) {
    let scause: usize;
    let stval: usize;
    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) scause);
        core::arch::asm!("csrr {}, stval", out(reg) stval);
    }

    let is_interrupt = (scause as isize) < 0;
    let code = scause & 0xff;

    if is_interrupt {
        if code == 5 {
            // Schedule the next timer interrupt (10ms in the future)
            crate::sbi::set_timer(crate::sbi::get_time() + 100_000);
            // Decrement lifespans of active domains in the cluster
            crate::dist::cluster::heartbeat_tick();
            // Drive Raft election timeouts and leader heartbeats
            crate::dist::raft::raft_tick();
            // Process incoming DKCP messages (Raft, caps, NES results)
            crate::dist::nes_dist::process_incoming();
            // Supervisor timer interrupt: yield current thread for preemption
            thread::schedule();

        } else {
            unsafe {
                crate::println!(
                    "[TRAP] Unhandled interrupt: scause = 0x{:X}, sepc = 0x{:X}",
                    scause,
                    (*tf).sepc
                );
            }
        }
    } else {
        if code == 8 {
            // Environment call from User Mode (U-mode ecall)
            let (id, arg0, arg1, arg2, arg3, arg4) = unsafe {
                (
                    (*tf).regs[17], // a7: syscall number
                    (*tf).regs[10], // a0: arg0
                    (*tf).regs[11], // a1: arg1
                    (*tf).regs[12], // a2: arg2
                    (*tf).regs[13], // a3: arg3
                    (*tf).regs[14], // a4: arg4
                )
            };

            let ret = crate::syscall::syscall_handler(id, arg0, arg1, arg2, arg3, arg4);
            unsafe {
                (*tf).regs[10] = ret as usize;
                // Advance program counter past the ecall instruction (4 bytes)
                (*tf).sepc += 4;
            }
        } else {
            // Unhandled exception: print detailed diagnostic and panic
            unsafe {
                let active_satp: usize;
                core::arch::asm!("csrr {}, satp", out(reg) active_satp);
                crate::println!("\n================ KERNEL PANIC ==================");
                crate::println!("UNHANDLED EXCEPTION");
                crate::println!("Cause (scause):         0x{:X} (code: {})", scause, code);
                crate::println!("Fault Address (stval):  0x{:X}", stval);
                crate::println!("Program Counter (sepc): 0x{:X}", (*tf).sepc);
                crate::println!("Active satp:            0x{:X}", active_satp);
                crate::println!("TrapFrame Address:      {:?}", tf);
                crate::println!("Saved Registers:");

                for i in 1..32 {
                    crate::println!("  x{:02}: 0x{:016X}", i, (*tf).regs[i]);
                }
                crate::println!("================================================");
            }
            panic!("Unhandled exception in privilege mode!");
        }
    }
}

/// Transition function to switch to User Mode (U-mode) and run a program.
///
/// Parameters:
/// - `entry_point`: The address in user space where execution starts.
/// - `user_sp`: The stack pointer to use in U-mode.
/// - `kernel_sp`: The kernel stack top to restore on next trap entry (stored in `sscratch`).
///
/// # Why `kernel_sp` must be passed explicitly
///
/// If we did `csrw sscratch, sp` inside this function, `sp` would point to
/// a location deep in the kernel call chain (inside this function's stack frame).
/// When the next `ecall` trap fires, the trap vector allocates a 272-byte
/// TrapFrame below that address — potentially overwriting kernel code or data.
///
/// By passing `kernel_sp` (the thread's stack top), we guarantee the TrapFrame
/// is allocated safely inside the thread's 16KB kernel stack region.
///
/// # Safety
///
/// This switches CPU modes to U-mode using raw assembly inline registers.
/// The caller must ensure that the entry point and stack pointer are valid
/// user-space addresses, and that `kernel_sp` is the correct kernel stack top.
#[inline(never)]
pub unsafe fn enter_user_mode(entry_point: usize, user_sp: usize, kernel_sp: usize) -> ! {
    // Sanity-check: kernel_sp must be above kernel text (loaded at 0x8020_0000).
    // Kernel stacks live in BSS/data well above 0x8021_0000. If this fires, the
    // caller passed a wrong value and we would corrupt kernel text on the next trap.
    assert!(
        kernel_sp >= 0x8021_0000,
        "[TRAP] BUG: kernel_sp=0x{:X} is inside/below kernel text! sscratch would corrupt code.",
        kernel_sp
    );

    unsafe {
        core::arch::asm!(
            // Clear SPP (Supervisor Previous Privilege, bit 8) → sret will drop to U-mode
            "li t0, 0x100",
            "csrc sstatus, t0",
            // Set SPIE (Supervisor Previous Interrupt Enable, bit 5) → interrupts enabled in U-mode
            "li t0, 0x20",
            "csrs sstatus, t0",
            // Set sepc to the user entry point; sret will jump there
            "csrw sepc, {entry_point}",
            // *** THE CRITICAL FIX ***
            // Write the thread's kernel stack TOP into sscratch.
            // When the next trap fires from U-mode, trap_vector does:
            //   csrrw sp, sscratch, sp   → sp = kernel_sp, sscratch = user_sp
            // and then allocates the TrapFrame downward from kernel_sp.
            "csrw sscratch, {kernel_sp}",
            // Switch to user stack and sret into U-mode
            "mv sp, {user_sp}",
            "sret",
            entry_point = in(reg) entry_point,
            user_sp     = in(reg) user_sp,
            kernel_sp   = in(reg) kernel_sp,
            options(noreturn)
        );
    }
}
