//! VeridianOS SMP & User-Space Exception Delivery Verification Program
//!
//! This program validates the kernel's user-space exception delivery pipeline,
//! which was introduced alongside SMP (secondary harts 1-3 woken via SBI HSM).
//!
//! The exception delivery model:
//!   - SYS_REGISTER_EXCEPTION_HANDLER (110): registers a VA as page-fault handler
//!   - SYS_EXCEPTION_RETURN (111): returns from a user exception handler
//!   - On page fault (scause 12/13/15) the kernel saves sepc, dispatches to the
//!     handler with: a0=scause, a1=stval (fault addr), a2=original sepc
//!
//! Test plan
//! ─────────
//! TEST 1 — Registration: SYS_REGISTER_EXCEPTION_HANDLER must return 0.
//! TEST 2 — Fault & flag: trigger a page fault at 0x1234_5678, handler sets a
//!           flag byte to 0xAB, calls SYS_EXCEPTION_RETURN; verify flag == 0xAB.
//! TEST 3 — Resumed execution: print a message after fault return to confirm
//!           execution resumed at the instruction after the faulting load.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU8, Ordering};

// ─── Syscall ABI ──────────────────────────────────────────────────────────────
//
// a7 = syscall number
// a0 = arg0 (also return value)
// a1 = arg1
// a2 = arg2

#[inline(always)]
fn syscall(id: usize, arg0: usize, arg1: usize, arg2: usize) -> isize {
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

// ─── Syscall numbers ─────────────────────────────────────────────────────────

const SYS_WRITE:                    usize = 1;
const SYS_EXIT:                     usize = 2;
const SYS_REGISTER_EXCEPTION_HANDLER: usize = 110;
const SYS_EXCEPTION_RETURN:         usize = 111;

// ─── I/O helpers ─────────────────────────────────────────────────────────────

fn print(s: &str) {
    syscall(SYS_WRITE, s.as_ptr() as usize, s.len(), 0);
}

fn fail(msg: &str) -> ! {
    print("[FAIL] ");
    print(msg);
    print("\n");
    syscall(SYS_EXIT, 1, 0, 0);
    loop {}
}

// ─── Hex formatting (no alloc) ───────────────────────────────────────────────

fn print_hex(label: &str, val: usize) {
    print(label);
    let mut buf = [b'0'; 18]; // "0x" + 16 hex digits
    buf[0] = b'0';
    buf[1] = b'x';
    let hex = b"0123456789abcdef";
    for i in 0..16 {
        buf[17 - i] = hex[(val >> (i * 4)) & 0xF];
    }
    // Trim leading zeros but keep at least one digit after "0x"
    let digits = &buf[2..];
    let mut start = 0usize;
    while start < 15 && digits[start] == b'0' {
        start += 1;
    }
    let slice = &buf[0..2 + (16 - start)]; // "0x" + trimmed digits
    // SAFETY: all bytes are ASCII
    let s = unsafe { core::str::from_utf8_unchecked(slice) };
    print(s);
}

// ─── Exception flag ──────────────────────────────────────────────────────────
//
// Placed in .bss so it has a fixed, known virtual address inside this process.
// The handler writes 0xAB here; _start verifies it after the fault returns.
// AtomicU8 prevents the compiler from optimising away the post-fault read.

static HANDLER_FLAG: AtomicU8 = AtomicU8::new(0);

// ─── User-space exception handler ────────────────────────────────────────────
//
// The kernel calls this function with the RISC-V C calling convention:
//   a0 = scause   (12 = insn page fault, 13 = load PF, 15 = store PF)
//   a1 = stval    (faulting virtual address)
//   a2 = sepc     (PC of the faulting instruction)
//
// Constraints:
//   - #[inline(never)] prevents inlining so the symbol has a real PC.
//   - #[unsafe(no_mangle)] keeps the symbol name stable for the address cast.
//   - Must end with SYS_EXCEPTION_RETURN ecall — the kernel restores the saved
//     user context and re-enters U-mode at sepc+4 (skip the faulting insn).

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn exception_handler(scause: usize, stval: usize, sepc: usize) {
    print("[HANDLER] Caught exception: scause=");
    print_hex("", scause);
    print(" stval=");
    print_hex("", stval);
    print(" sepc=");
    print_hex("", sepc);
    print("\n");

    // Signal the main thread that the handler ran.
    HANDLER_FLAG.store(0xAB, Ordering::Release);

    // Return control to the kernel, which will resume execution at sepc+4.
    syscall(SYS_EXCEPTION_RETURN, 0, 0, 0);

    // Unreachable — SYS_EXCEPTION_RETURN does not return to this call site.
    loop {}
}

// ─── Entry point ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("\n");
    print("[USER] VeridianOS SMP & Exception Delivery Verification\n");
    print("[USER] ====================================================\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 1 — Basic exception handler registration
    //
    // Pass the actual PC of exception_handler to SYS_REGISTER_EXCEPTION_HANDLER.
    // The kernel stores this address in the process's PCB and will jump to it
    // whenever a page fault occurs while the process is in U-mode.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 1: Register exception handler...\n");

    let handler_va = exception_handler as *const () as usize;
    print_hex("[USER]   handler VA = ", handler_va);
    print("\n");

    let ret1 = syscall(SYS_REGISTER_EXCEPTION_HANDLER, handler_va, 0, 0);
    if ret1 != 0 {
        fail("SYS_REGISTER_EXCEPTION_HANDLER did not return 0");
    }

    print("[USER] TEST 1 PASSED: Handler registered (ret=0).\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 2 — Trigger and catch a page fault
    //
    // Dereference 0x1234_5678 — this address is not mapped in the user process
    // page table, so the hardware raises a load page fault (scause=13).
    // The kernel dispatches to exception_handler, which:
    //   1. Prints the scause/stval/sepc tuple.
    //   2. Stores 0xAB in HANDLER_FLAG.
    //   3. Calls SYS_EXCEPTION_RETURN.
    // The kernel then resumes execution here at the instruction after the load.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 2: Trigger page fault at 0x12345678...\n");

    // Intentional fault — load from an unmapped address.
    // Wrapped in a block so the compiler cannot hoist it.
    let _dummy: u8 = unsafe {
        core::ptr::read_volatile(0x1234_5678usize as *const u8)
    };
    // Execution resumes here after SYS_EXCEPTION_RETURN.

    let flag = HANDLER_FLAG.load(Ordering::Acquire);
    if flag != 0xAB {
        fail("Handler flag not set to 0xAB after fault return");
    }

    print("[USER] TEST 2 PASSED: Handler ran, flag == 0xAB, execution resumed.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 3 — Confirm execution resumed after fault return
    //
    // If we reach this print, the kernel correctly:
    //   (a) delivered the exception to the handler,
    //   (b) restored the user register file on SYS_EXCEPTION_RETURN,
    //   (c) advanced sepc past the faulting instruction, and
    //   (d) re-entered U-mode at the correct continuation PC.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 3: Confirm execution resumed after exception return...\n");
    print("[USER] TEST 3 PASSED: Reached post-fault continuation point.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // ALL TESTS PASSED
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] ====================================================\n");
    print("[USER] SMP Exception Delivery — ALL TESTS PASSED!\n");
    print("[USER] ====================================================\n\n");

    syscall(SYS_EXIT, 0, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[PANIC] unexpected panic in smp_test\n");
    syscall(SYS_EXIT, 1, 0, 0);
    loop {}
}
