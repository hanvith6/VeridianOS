//! Attestation primitives for the VeridianOS M-mode monitor.
//!
//! Provides SHA-256 and a simplified HMAC-SHA-256 signing scheme used to
//! produce hardware attestation reports for enclaves.
//!
//! ## Design Decisions
//!
//! - **No external crates**: The monitor is a `no_std` binary with no heap.
//!   We implement SHA-256 directly from FIPS 180-4.
//!
//! - **Device key**: In a production TEE the device key is fused into OTP
//!   (One-Time Programmable) memory and only readable from M-mode. Here we
//!   use a compile-time constant for the scaffold — a real deployment would
//!   read it from a RISC-V platform-specific CSR or MMIO register in the
//!   secure enclave controller.
//!
//! - **HMAC-SHA-256**: We use HMAC rather than a bare SHA-256 signature so
//!   that the device key is never exposed in the hash input. A remote
//!   verifier must know the device public key (distributed out-of-band,
//!   e.g., via a PKI) to verify the HMAC tag.
//!
//! ## Security Note
//!
//! This implementation is a scaffold. Before production use:
//!   1. Replace `DEVICE_KEY` with a key read from OTP/eFuse at runtime.
//!   2. Replace HMAC with an asymmetric signature (Ed25519 or ECDSA P-256)
//!      so remote verifiers do not need the private key.
//!   3. Consider using a Hardware Security Module (HSM) or Platform Security
//!      Processor for signing operations.

// -----------------------------------------------------------------------
// Device key (scaffold — replace with OTP read in production)
// -----------------------------------------------------------------------

/// 256-bit device identity key.
///
/// SECURITY: In production this MUST be read from OTP memory or a
/// hardware security element, not hardcoded. This value is used solely
/// to demonstrate the HMAC construction during Phase 12 scaffolding.
const DEVICE_KEY: [u8; 32] = [
    0x56, 0x65, 0x72, 0x69, 0x64, 0x69, 0x61, 0x6e, // "Veridian"
    0x4f, 0x53, 0x4b, 0x65, 0x79, 0x30, 0x31, 0x30, // "OSKey010"
    0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, // random-looking suffix
    0x13, 0x37, 0xC0, 0xDE, 0xFA, 0xCE, 0xFF, 0x00,
];

// -----------------------------------------------------------------------
// SHA-256 — FIPS 180-4 implementation (no_std, no heap)
// -----------------------------------------------------------------------

/// SHA-256 initial hash values (first 32 bits of fractional parts of square
/// roots of the first 8 primes).
const H: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 round constants (first 32 bits of fractional parts of cube roots
/// of the first 64 primes).
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

#[inline(always)]
fn rotr32(x: u32, n: u32) -> u32 { x.rotate_right(n) }

#[inline(always)]
fn ch(x: u32, y: u32, z: u32) -> u32 { (x & y) ^ (!x & z) }

#[inline(always)]
fn maj(x: u32, y: u32, z: u32) -> u32 { (x & y) ^ (x & z) ^ (y & z) }

#[inline(always)]
fn sigma0(x: u32) -> u32 { rotr32(x, 2)  ^ rotr32(x, 13) ^ rotr32(x, 22) }

#[inline(always)]
fn sigma1(x: u32) -> u32 { rotr32(x, 6)  ^ rotr32(x, 11) ^ rotr32(x, 25) }

#[inline(always)]
fn gamma0(x: u32) -> u32 { rotr32(x, 7)  ^ rotr32(x, 18) ^ (x >> 3) }

#[inline(always)]
fn gamma1(x: u32) -> u32 { rotr32(x, 17) ^ rotr32(x, 19) ^ (x >> 10) }

/// Process a single 512-bit (64-byte) block and update the hash state.
fn sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];

    // Prepare message schedule
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        w[i] = gamma1(w[i - 2])
            .wrapping_add(w[i - 7])
            .wrapping_add(gamma0(w[i - 15]))
            .wrapping_add(w[i - 16]);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;

    for i in 0..64 {
        let t1 = h
            .wrapping_add(sigma1(e))
            .wrapping_add(ch(e, f, g))
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let t2 = sigma0(a).wrapping_add(maj(a, b, c));

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

/// Compute SHA-256 over an arbitrary byte slice.
///
/// Returns the 32-byte digest. No heap allocation; uses a fixed 64-byte
/// stack buffer for block processing.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = H;
    let mut buf   = [0u8; 64];
    let mut buf_len = 0usize;
    let mut total_bits: u64 = 0;

    for &byte in data {
        buf[buf_len] = byte;
        buf_len += 1;
        total_bits = total_bits.wrapping_add(8);

        if buf_len == 64 {
            sha256_block(&mut state, &buf);
            buf_len = 0;
        }
    }

    // Padding: append 0x80 bit, then zeros, then 64-bit big-endian bit count.
    buf[buf_len] = 0x80;
    buf_len += 1;

    if buf_len > 56 {
        // Not enough room for length field — process this block, start a new one.
        while buf_len < 64 { buf[buf_len] = 0; buf_len += 1; }
        sha256_block(&mut state, &buf);
        buf_len = 0;
    }

    // Zero-pad up to the 8-byte length field at offset 56.
    while buf_len < 56 { buf[buf_len] = 0; buf_len += 1; }

    // Append bit count in big-endian.
    let bits_be = total_bits.to_be_bytes();
    buf[56..64].copy_from_slice(&bits_be);
    sha256_block(&mut state, &buf);

    // Pack state into output digest (big-endian).
    let mut digest = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

// -----------------------------------------------------------------------
// HMAC-SHA-256
// -----------------------------------------------------------------------

const BLOCK_SIZE: usize = 64; // SHA-256 block size in bytes

/// Compute HMAC-SHA-256(key, message).
///
/// Follows RFC 2104. The key is padded or hashed to 64 bytes as required.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // Derive block-sized key.
    let mut k = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = sha256(key);
        k[..32].copy_from_slice(&hashed);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    // ipad = 0x36 repeated, opad = 0x5C repeated.
    let mut ipad = [0u8; BLOCK_SIZE];
    let mut opad = [0u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5C;
    }

    // inner = SHA-256(ipad || message)
    // We cannot heap-allocate, so we process the two parts incrementally
    // using a manual two-part SHA-256.
    let inner_hash = sha256_two_parts(&ipad, message);

    // outer = SHA-256(opad || inner_hash)
    sha256_two_parts(&opad, &inner_hash)
}

/// Compute SHA-256(part1 || part2) without heap allocation.
///
/// Processes `part1` first (it must be a multiple of the block size, which
/// is guaranteed for ipad/opad = 64 bytes), then continues with `part2`.
fn sha256_two_parts(part1: &[u8], part2: &[u8]) -> [u8; 32] {
    let mut state = H;
    let mut buf   = [0u8; 64];
    let mut buf_len = 0usize;
    let mut total_bits: u64 = 0;

    // Feed part1
    for &byte in part1 {
        buf[buf_len] = byte;
        buf_len += 1;
        total_bits = total_bits.wrapping_add(8);
        if buf_len == 64 { sha256_block(&mut state, &buf); buf_len = 0; }
    }

    // Feed part2
    for &byte in part2 {
        buf[buf_len] = byte;
        buf_len += 1;
        total_bits = total_bits.wrapping_add(8);
        if buf_len == 64 { sha256_block(&mut state, &buf); buf_len = 0; }
    }

    // Finalize with padding.
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

// -----------------------------------------------------------------------
// Public attestation API
// -----------------------------------------------------------------------

/// Sign an enclave measurement with the device key.
///
/// Returns a 24-byte truncated HMAC-SHA-256 tag (first 24 bytes of the
/// full 32-byte HMAC). Truncation to 192 bits is safe per NIST SP 800-107.
///
/// In production replace with Ed25519 sign(DEVICE_PRIVATE_KEY, measurement).
pub fn sign_measurement(measurement: &[u8; 32]) -> [u8; 24] {
    let full_hmac = hmac_sha256(&DEVICE_KEY, measurement);
    let mut tag = [0u8; 24];
    tag.copy_from_slice(&full_hmac[..24]);
    tag
}
