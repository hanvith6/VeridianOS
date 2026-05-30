//! VeridianOS Capability Rights Enforcement Test
//!
//! Verifies that SYS_HANDLE_DUPLICATE correctly enforces the capability rights
//! model: attenuation is permitted, amplification is denied.
//!
//! Test plan
//! ─────────
//! TEST 1 — Full rights inheritance (mask=0): duplicate handle 0 with no mask.
//! TEST 2 — Attenuated rights (READ only, mask=0x1): duplicate handle 0 to READ-only.
//! TEST 3 — Rights amplification rejected: duplicate a READ-only handle requesting
//!           ALL rights (mask=0xFF); kernel must return -13 (EACCES).
//! TEST 4 — Use-after-close: close handle 0, then try to duplicate it; must return -2 (EBADF).
//! TEST 5 — Invalid handle ID: duplicate handle 255; must return any negative error.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

// ─── Syscall ABI ──────────────────────────────────────────────────────────────

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

const SYS_WRITE:            usize = 1;
const SYS_EXIT:             usize = 2;
const SYS_HANDLE_CLOSE:     usize = 3;
const SYS_HANDLE_DUPLICATE: usize = 4;

// ─── Rights bitmask constants (mirrors kernel/src/capability/rights.rs) ──────

/// READ right — allows reading the object's contents or state.
const RIGHT_READ:      usize = 1 << 0; // 0x01
/// WRITE right — allows modifying the object's contents or state.
const RIGHT_WRITE:     usize = 1 << 1; // 0x02
/// EXECUTE right — allows executing memory mapped from the object.
const RIGHT_EXECUTE:   usize = 1 << 2; // 0x04
/// DUPLICATE right — allows duplicating the handle.
const RIGHT_DUPLICATE: usize = 1 << 3; // 0x08
/// TRANSFER right — allows transferring the handle to another process.
const RIGHT_TRANSFER:  usize = 1 << 4; // 0x10

/// All rights combined — used for the amplification-rejection test.
const ALL_RIGHTS: usize = RIGHT_READ | RIGHT_WRITE | RIGHT_EXECUTE | RIGHT_DUPLICATE | RIGHT_TRANSFER;

// ─── I/O helpers ─────────────────────────────────────────────────────────────

fn print(s: &str) {
    syscall(SYS_WRITE, s.as_ptr() as usize, s.len(), 0);
}

// Minimal decimal formatter for small non-negative values (no alloc).
fn print_usize(n: usize) {
    let mut buf = [b'0'; 20];
    let mut pos = 20usize;
    let mut val = n;
    if val == 0 {
        print("0");
        return;
    }
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf[pos..]) };
    print(s);
}

// Print a signed isize (handles negatives).
fn print_isize(n: isize) {
    if n < 0 {
        print("-");
        print_usize(n.unsigned_abs());
    } else {
        print_usize(n as usize);
    }
}

// ─── Test state ───────────────────────────────────────────────────────────────

struct Results {
    passed: usize,
    total: usize,
}

impl Results {
    const fn new() -> Self {
        Self { passed: 0, total: 0 }
    }

    fn pass(&mut self, name: &str) {
        self.total += 1;
        self.passed += 1;
        print("[TEST] ");
        print(name);
        print(": PASS\n");
    }

    fn fail(&mut self, name: &str, reason: &str) {
        self.total += 1;
        print("[TEST] ");
        print(name);
        print(": FAIL -- ");
        print(reason);
        print("\n");
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("\n");
    print("[USER] VeridianOS Capability Rights Enforcement Test\n");
    print("[USER] =============================================\n\n");

    let mut r = Results::new();

    // ════════════════════════════════════════════════════════════════════════
    // TEST 1 — Full rights inheritance: duplicate handle 0 with mask=0.
    //
    // rights_mask=0 means "inherit all rights from parent".  The kernel
    // should return a new handle ID (>= 0) with the same rights as handle 0.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 1: SYS_HANDLE_DUPLICATE(handle=0, mask=0) — full inheritance\n");
    let ret1 = syscall(SYS_HANDLE_DUPLICATE, 0, 0, 0);
    if ret1 >= 0 {
        print("[USER]   New handle ID = ");
        print_isize(ret1);
        print("\n");
        r.pass("full-rights-inherit");
    } else {
        print("[USER]   Returned ");
        print_isize(ret1);
        print("\n");
        r.fail("full-rights-inherit", "SYS_HANDLE_DUPLICATE with mask=0 should succeed");
    }

    // ════════════════════════════════════════════════════════════════════════
    // TEST 2 — Attenuated rights: duplicate handle 0 to READ-only.
    //
    // 0x1 = READ right only.  This is a subset of DEFAULT rights, so the
    // kernel must accept it and return a new READ-only handle.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 2: SYS_HANDLE_DUPLICATE(handle=0, mask=0x1) — READ-only attenuation\n");
    let ret2 = syscall(SYS_HANDLE_DUPLICATE, 0, RIGHT_READ, 0);
    if ret2 >= 0 {
        print("[USER]   READ-only handle ID = ");
        print_isize(ret2);
        print("\n");
        r.pass("rights-attenuation");
    } else {
        print("[USER]   Returned ");
        print_isize(ret2);
        print("\n");
        r.fail("rights-attenuation", "Attenuation to READ-only must succeed when parent holds READ");
    }

    // ════════════════════════════════════════════════════════════════════════
    // TEST 3 — Rights amplification rejected.
    //
    // We take the READ-only handle produced by TEST 2 and try to duplicate it
    // requesting ALL_RIGHTS (0x1F).  The kernel must intersect with the parent
    // rights, discover WRITE/EXECUTE/DUPLICATE/TRANSFER are not held, and
    // return -13 (EACCES — permission denied / rights amplification).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 3: SYS_HANDLE_DUPLICATE(read_only_handle, mask=0xFF) — amplification blocked\n");
    if ret2 >= 0 {
        let read_only_id = ret2 as usize;
        let ret3 = syscall(SYS_HANDLE_DUPLICATE, read_only_id, ALL_RIGHTS, 0);
        if ret3 == -13 {
            print("[USER]   Correctly returned -13 (EACCES)\n");
            r.pass("amplification-rejected");
        } else {
            print("[USER]   Returned ");
            print_isize(ret3);
            print(" (expected -13 EACCES)\n");
            r.fail(
                "amplification-rejected",
                "Rights amplification must be denied with -13 EACCES",
            );
        }
    } else {
        // TEST 2 produced an invalid handle — skip with a fail so the total
        // count stays accurate.
        r.fail(
            "amplification-rejected",
            "Skipped: READ-only handle from TEST 2 was invalid",
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // TEST 4 — Use-after-close.
    //
    // Close handle 0, then try to duplicate it.  The kernel's handle table
    // should no longer have an entry for ID 0 and must return -2 (EBADF).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 4: close handle 0, then SYS_HANDLE_DUPLICATE(0, 0) — use-after-close\n");
    let close_ret = syscall(SYS_HANDLE_CLOSE, 0, 0, 0);
    if close_ret != 0 {
        // If close fails the handle may already have been consumed; continue
        // with the duplicate attempt regardless — what matters is whether the
        // duplicate correctly rejects it.
        print("[USER]   Warning: SYS_HANDLE_CLOSE(0) returned ");
        print_isize(close_ret);
        print(" (continuing test)\n");
    }
    let ret4 = syscall(SYS_HANDLE_DUPLICATE, 0, 0, 0);
    if ret4 == -2 {
        print("[USER]   Correctly returned -2 (EBADF)\n");
        r.pass("use-after-close");
    } else {
        print("[USER]   Returned ");
        print_isize(ret4);
        print(" (expected -2 EBADF)\n");
        r.fail("use-after-close", "Duplicate of closed handle must return -2 EBADF");
    }

    // ════════════════════════════════════════════════════════════════════════
    // TEST 5 — Invalid handle ID.
    //
    // Handle 255 was never allocated.  Any negative return is acceptable —
    // the kernel must not crash or return a usable new handle.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 5: SYS_HANDLE_DUPLICATE(handle=255, mask=0) — invalid handle ID\n");
    let ret5 = syscall(SYS_HANDLE_DUPLICATE, 255, 0, 0);
    if ret5 < 0 {
        print("[USER]   Correctly returned error ");
        print_isize(ret5);
        print("\n");
        r.pass("invalid-handle-id");
    } else {
        print("[USER]   Returned ");
        print_isize(ret5);
        print(" (expected a negative error code)\n");
        r.fail("invalid-handle-id", "Duplicate of never-allocated handle 255 must fail");
    }

    // ─── Summary ─────────────────────────────────────────────────────────

    print("\n[USER] =============================================\n");
    print("[RESULT] rights_test: ");
    print_usize(r.passed);
    print("/");
    print_usize(r.total);
    print(" PASSED");
    if r.passed == r.total {
        print(" -- ALL TESTS PASSED");
    }
    print("\n");
    print("[USER] =============================================\n\n");

    syscall(SYS_EXIT, if r.passed == r.total { 0 } else { 1 }, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = "[PANIC] unexpected panic in rights_test\n";
    syscall(SYS_WRITE, msg.as_ptr() as usize, msg.len(), 0);
    syscall(SYS_EXIT, 1, 0, 0);
    loop {}
}
