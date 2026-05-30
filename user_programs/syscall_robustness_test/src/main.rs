//! VeridianOS Syscall Robustness Test
//!
//! Verifies that every tested syscall returns a negative error code when given
//! invalid or malicious arguments — and never causes a kernel panic.
//!
//! The key invariant: bad inputs must produce an error code, not a crash.
//! If the kernel panics, QEMU halts and this binary never prints — that failure
//! mode is also visible to the test harness.
//!
//! Test plan
//! ─────────
//! TEST 1 — SYS_WRITE null pointer:             ptr=0, len=10          → expect < 0
//! TEST 2 — SYS_WRITE kernel-space pointer:     ptr=0x80200000, len=4  → expect < 0
//! TEST 3 — SYS_HANDLE_CLOSE invalid handle:    id=255                 → expect < 0
//! TEST 4 — SYS_MAP zero length:                virt=0x3000000, len=0  → expect < 0
//! TEST 5 — SYS_MAP unaligned address:          virt=0x3000001, len=0x1000 → expect < 0
//! TEST 6 — SYS_NODE_CREATE invalid type:       type=0xFF, ptr=0, sz=0 → expect < 0
//! TEST 7 — SYS_CHANNEL_RECV invalid channel:   id=0xFFFF, buf=0, len=0 → expect < 0
//! TEST 8 — SYS_ENCLAVE_CREATE unaligned addr:  phys=0x86100001        → expect < 0

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

// ─── Syscall numbers (kernel/src/syscall/numbers.rs) ─────────────────────────

const SYS_WRITE:          usize = 1;
const SYS_EXIT:           usize = 2;
const SYS_HANDLE_CLOSE:   usize = 3;
const SYS_MAP:            usize = 8;
/// SYS_NODE_CREATE — create a semantic graph node.
/// Registers: a7=60, a0=node_type(u8), a1=ptr to config, a2=config_size
const SYS_NODE_CREATE:    usize = 60;
/// SYS_CHANNEL_RECV — receive a message from an IPC channel.
/// Registers: a7=73, a0=channel_id, a1=ptr to output buf, a2=ptr to output len
const SYS_CHANNEL_RECV:   usize = 73;
/// SYS_ENCLAVE_CREATE — create a hardware TEE enclave via M-mode monitor.
/// Registers: a7=120, a0=phys_addr, a1=size, a2=entry_pa
const SYS_ENCLAVE_CREATE: usize = 120;

// ─── I/O helpers ─────────────────────────────────────────────────────────────

fn print(s: &str) {
    syscall(SYS_WRITE, s.as_ptr() as usize, s.len(), 0);
}

fn print_isize(n: isize) {
    if n < 0 {
        print("-");
        print_usize(n.unsigned_abs());
    } else {
        print_usize(n as usize);
    }
}

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

// ─── Test state ───────────────────────────────────────────────────────────────

struct Results {
    passed: usize,
    total: usize,
}

impl Results {
    const fn new() -> Self {
        Self { passed: 0, total: 0 }
    }

    /// Record a PASS — the syscall correctly returned an error code (ret < 0).
    fn pass_if_error(&mut self, name: &str, ret: isize) {
        self.total += 1;
        if ret < 0 {
            self.passed += 1;
            print("[TEST] ");
            print(name);
            print(": PASS (returned ");
            print_isize(ret);
            print(")\n");
        } else {
            print("[TEST] ");
            print(name);
            print(": FAIL -- syscall returned ");
            print_isize(ret);
            print(", expected a negative error code\n");
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("\n");
    print("[USER] VeridianOS Syscall Robustness Test\n");
    print("[USER] ====================================\n\n");

    let mut r = Results::new();

    // ════════════════════════════════════════════════════════════════════════
    // TEST 1 — SYS_WRITE with null pointer.
    //
    // ptr=0 is never mapped in user space.  The kernel validates the buffer
    // via validate_user_buffer before touching it and must return -14 (EFAULT).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 1: SYS_WRITE(ptr=null, len=10)\n");
    let ret1 = syscall(SYS_WRITE, 0, 10, 0);
    r.pass_if_error("write-null-ptr", ret1);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 2 — SYS_WRITE with a kernel-space pointer.
    //
    // 0x80200000 is inside the kernel's physical-mapped region.  User processes
    // must not be able to read kernel memory via SYS_WRITE.  The buffer
    // validation must reject this and return a negative error.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 2: SYS_WRITE(ptr=0x80200000, len=4) — kernel-space ptr\n");
    let ret2 = syscall(SYS_WRITE, 0x8020_0000usize, 4, 0);
    r.pass_if_error("write-kernel-ptr", ret2);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 3 — SYS_HANDLE_CLOSE with an invalid handle ID.
    //
    // Handle 255 was never allocated in this process.  The kernel's handle
    // table lookup must return -2 (EBADF).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 3: SYS_HANDLE_CLOSE(handle=255) — never-allocated handle\n");
    let ret3 = syscall(SYS_HANDLE_CLOSE, 255, 0, 0);
    r.pass_if_error("close-invalid-handle", ret3);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 4 — SYS_MAP with zero length.
    //
    // len=0 is rejected by the sys_map length guard before any page-table
    // work occurs.  Expected: -22 (EINVAL).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 4: SYS_MAP(virt=0x3000000, len=0) — zero-length mapping\n");
    let ret4 = syscall(SYS_MAP, 0x0300_0000usize, 0, 0);
    r.pass_if_error("map-zero-len", ret4);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 5 — SYS_MAP with unaligned virtual address.
    //
    // Page-alignment is required (4 KiB).  virt=0x3000001 is off by one byte.
    // The length guard (0x1000 % PAGE_SIZE == 0) passes, but the address guard
    // (virt_addr % PAGE_SIZE != 0) must return -22 (EINVAL).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 5: SYS_MAP(virt=0x3000001, len=0x1000) — unaligned virtual address\n");
    let ret5 = syscall(SYS_MAP, 0x0300_0001usize, 0x1000, 0);
    r.pass_if_error("map-unaligned-virt", ret5);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 6 — SYS_NODE_CREATE with an invalid node type.
    //
    // 0xFF is not a valid SemanticNodeType variant.  The kernel's semantic
    // graph module performs an enum range check and must return a negative
    // error without panicking.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 6: SYS_NODE_CREATE(type=0xFF, ptr=0, size=0) — invalid node type\n");
    let ret6 = syscall(SYS_NODE_CREATE, 0xFF, 0, 0);
    r.pass_if_error("node-create-invalid-type", ret6);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 7 — SYS_CHANNEL_RECV on an invalid channel ID.
    //
    // Channel 0xFFFF does not exist.  The agent subsystem must return a
    // negative error without looking up garbage memory.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 7: SYS_CHANNEL_RECV(id=0xFFFF, buf=0, len=0) — invalid channel\n");
    let ret7 = syscall(SYS_CHANNEL_RECV, 0xFFFF, 0, 0);
    r.pass_if_error("channel-recv-invalid-id", ret7);

    // ════════════════════════════════════════════════════════════════════════
    // TEST 8 — SYS_ENCLAVE_CREATE with an unaligned physical address.
    //
    // The enclave region must be NAPOT-aligned (power-of-two, naturally
    // aligned).  phys=0x86100001 is off by one byte.  The M-mode monitor's
    // alignment check must reject this and return a negative error.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 8: SYS_ENCLAVE_CREATE(phys=0x86100001, size=0x4000, entry=0x86100000) — unaligned phys\n");
    let ret8 = syscall(SYS_ENCLAVE_CREATE, 0x8610_0001usize, 0x4000, 0x8610_0000usize);
    r.pass_if_error("enclave-create-unaligned", ret8);

    // ─── Summary ─────────────────────────────────────────────────────────

    print("\n[USER] ====================================\n");
    print("[RESULT] syscall_robustness_test: ");
    print_usize(r.passed);
    print("/");
    print_usize(r.total);
    print(" PASSED");
    if r.passed == r.total {
        print(" -- ALL TESTS PASSED");
    }
    print("\n");
    print("[USER] ====================================\n\n");

    syscall(SYS_EXIT, if r.passed == r.total { 0 } else { 1 }, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = "[PANIC] unexpected panic in syscall_robustness_test\n";
    syscall(SYS_WRITE, msg.as_ptr() as usize, msg.len(), 0);
    syscall(SYS_EXIT, 1, 0, 0);
    loop {}
}
