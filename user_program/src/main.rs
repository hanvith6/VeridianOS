//! VeridianOS User-Space Verification Program
//!
//! Compiles into a clean, minimal ELF64 RISC-V binary that invokes VeridianOS system calls.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// Executes a VeridianOS system call using the RISC-V `ecall` hardware instruction.
#[inline]
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

/// The entry point for the user-space program.
///
/// This function is placed in the `.text.entry` section so that the linker puts it first.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    let msg = "Hello from dynamically parsed ELF User-Mode program!\n";

    // Invoke SYS_WRITE (1)
    syscall(1, msg.as_ptr() as usize, msg.len(), 0);

    // Invoke SYS_EXIT (2) with status code 0
    syscall(2, 0, 0, 0);

    // Loop forever if SYS_EXIT doesn't terminate the context immediately
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
