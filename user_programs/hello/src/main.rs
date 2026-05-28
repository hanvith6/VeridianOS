//! VeridianOS Hello World User-Space Process
//!
//! This is a standalone user-space process compiled to a RISC-V ELF binary,
//! stored in the disk image, and dynamically loaded at runtime by the
//! VeridianOS kernel via the VirtIO block driver and InitRAMFS.
//!
//! It demonstrates the full Phase 6 pipeline:
//! disk image → VirtIO read → ustar parse → ELF load → U-mode execution

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// Execute a VeridianOS system call using the RISC-V `ecall` instruction.
///
/// Syscall ABI (matching kernel/src/syscall/mod.rs):
/// - a7 = syscall number
/// - a0 = arg0 (first argument / return value)
/// - a1 = arg1
/// - a2 = arg2
#[inline(always)]
pub fn syscall(id: usize, arg0: usize, arg1: usize, arg2: usize) -> isize {
    let ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") id,
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            lateout("a0") ret,
        );
    }
    ret
}

/// SYS_WRITE syscall number (write bytes to UART console)
const SYS_WRITE: usize = 1;
/// SYS_EXIT syscall number (terminate this process)
const SYS_EXIT: usize = 2;

/// Write a string to the kernel UART console via SYS_WRITE.
fn print(s: &str) {
    syscall(SYS_WRITE, s.as_ptr() as usize, s.len(), 0);
}

/// The user-space program entry point.
///
/// Placed in `.text.entry` so the linker puts it at the very start
/// of the `.text` section — exactly at the ELF entry point address.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("\n");
    print("╔══════════════════════════════════════════════════╗\n");
    print("║   Hello from VeridianOS User Process!           ║\n");
    print("║                                                  ║\n");
    print("║  This process was:                               ║\n");
    print("║  1. Stored as an ELF in a TAR disk image         ║\n");
    print("║  2. Read from disk via VirtIO block driver       ║\n");
    print("║  3. Found by name in the InitRAMFS               ║\n");
    print("║  4. Loaded by the ELF64 parser                   ║\n");
    print("║  5. Executed in U-mode with isolated page tables ║\n");
    print("╚══════════════════════════════════════════════════╝\n");
    print("\n");

    // Exit cleanly via SYS_EXIT
    syscall(SYS_EXIT, 0, 0, 0);

    // Unreachable — SYS_EXIT terminates the thread
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall(SYS_EXIT, 1, 0, 0);
    loop {}
}
