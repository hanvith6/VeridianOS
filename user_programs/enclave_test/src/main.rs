//! VeridianOS Phase 12 Enclave Security Verification Program
//!
//! This program validates the Keystone-style M-mode TEE Security Monitor.
//!
//! Test plan
//! ─────────
//! TEST 1 — Enclave Creation: call SYS_ENCLAVE_CREATE on a 16KB region at 0x8610_0000.
//! TEST 2 — Enclave Entry & Exit: copy ecall payload, call SYS_ENCLAVE_ENTER, and verify it exits cleanly.
//! TEST 3 — Remote Attestation: call SYS_ENCLAVE_ATTEST and cryptographically verify the signature.

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

const SYS_WRITE:          usize = 1;
const SYS_EXIT:           usize = 2;
const SYS_ENCLAVE_CREATE: usize = 120;
const SYS_ENCLAVE_ENTER:  usize = 121;
const SYS_ENCLAVE_ATTEST: usize = 123;

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

fn print_hex(label: &str, val: usize) {
    print(label);
    print("0x");
    let mut buf = [b'0'; 16];
    let hex = b"0123456789abcdef";
    for i in 0..16 {
        buf[15 - i] = hex[(val >> (i * 4)) & 0xF];
    }
    let mut start = 0usize;
    while start < 15 && buf[start] == b'0' {
        start += 1;
    }
    let slice = &buf[start..];
    let s = unsafe { core::str::from_utf8_unchecked(slice) };
    print(s);
}

// ─── SHA-256 & HMAC-SHA-256 for verification ─────────────────────────────────

const H: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        w[i] = (w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10))
            .wrapping_add(w[i - 7])
            .wrapping_add(w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3))
            .wrapping_add(w[i - 16]);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;

    for i in 0..64 {
        let t1 = h
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ (!e & g))
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) ^ (a & c) ^ (b & c));

        h = g; g = f; f = e; e = d.wrapping_add(t1);
        d = c; c = b; b = a; a = t1.wrapping_add(t2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

fn sha256_two_parts(part1: &[u8], part2: &[u8]) -> [u8; 32] {
    let mut state = H;
    let mut buf = [0u8; 64];
    let mut buf_len = 0usize;
    let mut total_bits: u64 = 0;

    for &byte in part1 {
        buf[buf_len] = byte;
        buf_len += 1;
        total_bits = total_bits.wrapping_add(8);
        if buf_len == 64 { sha256_block(&mut state, &buf); buf_len = 0; }
    }

    for &byte in part2 {
        buf[buf_len] = byte;
        buf_len += 1;
        total_bits = total_bits.wrapping_add(8);
        if buf_len == 64 { sha256_block(&mut state, &buf); buf_len = 0; }
    }

    buf[buf_len] = 0x80; buf_len += 1;
    if buf_len > 56 {
        while buf_len < 64 { buf[buf_len] = 0; buf_len += 1; }
        sha256_block(&mut state, &buf);
        buf_len = 0;
    }
    while buf_len < 56 { buf[buf_len] = 0; buf_len += 1; }
    buf[56..64].copy_from_slice(&total_bits.to_be_bytes());
    sha256_block(&mut state, &buf);

    let mut digest = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn verify_hmac(key: &[u8; 32], measurement: &[u8; 32], expected_sig: &[u8]) -> bool {
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..32 {
        ipad[i] = key[i] ^ 0x36;
        opad[i] = key[i] ^ 0x5c;
    }
    for i in 32..64 {
        ipad[i] = 0x36;
        opad[i] = 0x5c;
    }

    let inner_hash = sha256_two_parts(&ipad, measurement);
    let full_hmac = sha256_two_parts(&opad, &inner_hash);
    
    &full_hmac[..24] == expected_sig
}

// ─── Entry point ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("\n");
    print("[USER] VeridianOS Phase 12 Security Monitor Verification\n");
    print("[USER] ====================================================\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 1 — Enclave Creation
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 1: Creating enclave...\n");
    let phys_addr = 0x8610_0000usize;
    let size = 0x4000usize; // 16KB
    let entry_pa = 0x8610_0000usize;

    // Enclave memory initially zeroed (VMO mapped at 0x4010_0000)
    let vaddr = 0x4010_0000 as *mut u8;
    unsafe {
        core::ptr::write_bytes(vaddr, 0, size);
    }

    let enclave_id = syscall(SYS_ENCLAVE_CREATE, phys_addr, size, entry_pa);
    if enclave_id < 0 {
        print_hex("[USER]   SYS_ENCLAVE_CREATE failed: ", enclave_id as usize);
        print("\n");
        fail("SYS_ENCLAVE_CREATE returned error");
    }
    print_hex("[USER]   Enclave created successfully, ID = ", enclave_id as usize);
    print("\n");
    print("[USER] TEST 1 PASSED.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 2 — Enclave Entry & Exit
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 2: Preparing and entering enclave...\n");

    // Write enclave exit code to virtual address 0x4010_0000
    // li a0, 0
    // li a7, 122 (SYS_ENCLAVE_EXIT)
    // ecall
    let payload = [
        0x13, 0x05, 0x00, 0x00, // li a0, 0
        0x93, 0x08, 0xA0, 0x07, // li a7, 122
        0x73, 0x00, 0x00, 0x00, // ecall
    ];
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), vaddr, payload.len());
    }

    print("[USER]   Entering enclave...\n");
    let enter_ret = syscall(SYS_ENCLAVE_ENTER, enclave_id as usize, 0, 0);
    if enter_ret != 0 {
        print_hex("[USER]   SYS_ENCLAVE_ENTER failed: ", enter_ret as usize);
        print("\n");
        fail("SYS_ENCLAVE_ENTER returned error");
    }
    print("[USER]   Returned from enclave cleanly.\n");
    print("[USER] TEST 2 PASSED.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 3 — Remote Attestation & HMAC Verification
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 3: Generating attestation report...\n");
    
    // Allocate 73-byte report buffer in user-space
    let mut report = [0u8; 73];
    let report_ptr = report.as_mut_ptr() as usize;

    let attest_ret = syscall(SYS_ENCLAVE_ATTEST, enclave_id as usize, report_ptr, 0);
    if attest_ret != 0 {
        print_hex("[USER]   SYS_ENCLAVE_ATTEST failed: ", attest_ret as usize);
        print("\n");
        fail("SYS_ENCLAVE_ATTEST returned error");
    }

    print("[USER]   Attestation report generated successfully.\n");
    print_hex("[USER]     Report enclave_id: ", report[0] as usize);
    print("\n");

    // Verify report fields
    let start_pa = u64::from_le_bytes(report[1..9].try_into().unwrap()) as usize;
    let size_val = u64::from_le_bytes(report[9..17].try_into().unwrap()) as usize;
    print_hex("[USER]     Report base PA: ", start_pa);
    print("\n");
    print_hex("[USER]     Report size: ", size_val);
    print("\n");

    if start_pa != phys_addr || size_val != size {
        fail("Report fields do not match expected enclave dimensions");
    }

    // Verify HMAC signature using the M-mode device key
    let device_key: [u8; 32] = [
        0x56, 0x65, 0x72, 0x69, 0x64, 0x69, 0x61, 0x6e,
        0x4f, 0x53, 0x4b, 0x65, 0x79, 0x30, 0x31, 0x30,
        0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE,
        0x13, 0x37, 0xC0, 0xDE, 0xFA, 0xCE, 0xFF, 0x00,
    ];
    let measurement: [u8; 32] = report[17..49].try_into().unwrap();
    let signature = &report[49..73];

    let verified = verify_hmac(&device_key, &measurement, signature);
    if !verified {
        fail("HMAC-SHA-256 signature verification failed!");
    }
    print("[USER]     HMAC-SHA-256 signature verified successfully.\n");
    print("[USER] TEST 3 PASSED.\n\n");

    print("[USER] ====================================================\n");
    print("[USER] Enclave Lifecycle & Attestation — ALL TESTS PASSED!\n");
    print("[USER] ====================================================\n\n");

    syscall(SYS_EXIT, 0, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    print("[PANIC] unexpected panic in enclave_test\n");
    syscall(SYS_EXIT, 1, 0, 0);
    loop {}
}
