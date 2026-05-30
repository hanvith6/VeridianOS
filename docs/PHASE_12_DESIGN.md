# VeridianOS Phase 12 — Hardware-Attested AI Agent Enclaves

> **Status:** Implemented  
> **Depends On:** Phase 11 (Distributed Multi-Kernel Coherence), Phase 9 (Agent Runtime)  
> **Duration:** 4–5 weeks  
> **Complexity Summary:** High overall — M-mode firmware, PMP configuration, cryptographic attestation, and cross-privilege SBI bridge.

---

## 1. Overview and Goals

### 1.1 What Phase 12 Delivers

Phase 12 adds a **Trusted Execution Environment (TEE)** to VeridianOS through a custom M-mode Security Monitor that sits below the kernel in the RISC-V privilege hierarchy. The monitor enforces physical memory isolation at the hardware level using the Physical Memory Protection (PMP) unit — a CPU mechanism that cannot be circumvented by software running at lower privilege levels, including the kernel itself.

The concrete deliverables are:

- A standalone `veridian-monitor` Rust crate that runs as M-mode firmware, loaded by QEMU as `-bios` before the kernel.
- A PMP configuration engine (`pmp.rs`) implementing NAPOT encoding, region lock/unlock, and self-protection for the monitor image.
- A full enclave lifecycle (`enclave.rs`): create, enter, exit, and attest, with a static pool of 8 concurrent enclaves.
- A cryptographic attestation subsystem (`attest.rs`) implementing SHA-256 (FIPS 180-4) and HMAC-SHA-256 (RFC 2104) with no heap and no external dependencies.
- An SBI extension dispatcher (`sbi_handler.rs`) for EID `0x08424B45` ("BKE"), providing the kernel with four SBI calls to manage enclaves.
- A kernel-side bridge (`kernel/src/enclave/mod.rs`) that translates S-mode syscalls `120–123` into SBI ecalls to the monitor.
- Three QEMU integration tests in `user_programs/enclave_test/` that verify creation, enter/exit, and HMAC attestation end-to-end.
- An `enclave_id: Option<u8>` field on `AgentRecord` in the Phase 9 agent runtime, binding hardware isolation to kernel-tracked AI agents.

### 1.2 Why TEE Isolation Matters for AI Agents

VeridianOS Phase 9 introduced a kernel-tracked agent runtime where agents carry intent metadata and IPC channels. Phase 10 gave those agents adaptive scheduling. Phase 11 enabled them to span multiple kernel domains. A persistent gap remained: a privileged process or exploited kernel could read the memory of any agent — model weights, private context windows, user data processed during inference.

TEE isolation addresses this through the hardware trust boundary. Once the monitor locks an enclave region with PMP, no software at S-mode or U-mode can access it — not the kernel, not other user processes, not DMA without IOMMU support. The isolation is not a software convention; it is enforced by the CPU's memory protection unit in a privilege ring the kernel cannot reach.

For AI agent workloads specifically, this means:

- Model weights loaded into an enclave cannot be exfiltrated even if the kernel is compromised.
- Attestation reports allow a remote verifier to confirm that a specific agent binary (identified by its SHA-256 measurement) is running on a legitimate VeridianOS device before sending it sensitive data.
- The Phase 11 capability system can bind a distributed capability to an attested enclave identity, enabling cross-domain trust chains grounded in hardware.

---

## 2. Architecture

### 2.1 Privilege Stack Diagram

```
  ┌────────────────────────────────────────────────────────────────────┐
  │  U-mode (User Space)                                               │
  │                                                                    │
  │   ┌──────────────────────┐     ┌────────────────────────────────┐ │
  │   │  Regular User Process │     │  Enclave Payload (U-mode)      │ │
  │   │  (SYS_ENCLAVE_*)     │     │  PMP-isolated, no S-mode touch │ │
  │   └──────────┬───────────┘     └───────────────┬────────────────┘ │
  │              │ ecall (a7=120..123)               │ ecall (exit/attest)│
  └──────────────┼───────────────────────────────────┼────────────────┘
                 │                                   │
  ┌──────────────▼───────────────────────────────────▼────────────────┐
  │  S-mode (VeridianOS Kernel)   kernel/src/enclave/mod.rs           │
  │                                                                    │
  │   Validates pointer/alignment → issues SBI ecall a7=0x08424B45   │
  │   AgentRecord.enclave_id ties agent identity to enclave slot      │
  └──────────────────────────────┬─────────────────────────────────────┘
                                 │ ecall → M-mode trap (mcause=9)
  ┌──────────────────────────────▼─────────────────────────────────────┐
  │  M-mode (VeridianOS Monitor)  monitor/src/                        │
  │                                                                    │
  │   sbi_handler.rs   — EID/FID dispatch                             │
  │   enclave.rs       — lifecycle, static pool of 8 slots             │
  │   pmp.rs           — NAPOT encoding, CSR writes, sfence.vma        │
  │   attest.rs        — SHA-256, HMAC-SHA-256, 73-byte report         │
  └──────────────────────────────┬─────────────────────────────────────┘
                                 │ pmpaddr/pmpcfg CSR writes
  ┌──────────────────────────────▼─────────────────────────────────────┐
  │  RISC-V Hardware                                                   │
  │                                                                    │
  │   PMP Unit — 16 entries, checked on every memory access           │
  │   Entry 0..7  : enclave slots (dynamically assigned)              │
  │   Entry 8..14 : reserved                                          │
  │   Entry 15    : monitor self-protection (L bit set, permanent)    │
  └────────────────────────────────────────────────────────────────────┘
```

### 2.2 Boot Sequence

The monitor is loaded by QEMU as M-mode firmware and executes before the kernel:

```
QEMU loads monitor at 0x8000_0000 (M-mode firmware entry)
  │
  ├─ hart 0: monitor_main(hart_id=0, dtb_ptr)
  │    1. Set mtvec → m_trap_vector
  │    2. lock_monitor_self() — PMP entry 15, L bit, deny S/U
  │    3. init_pool() — zero all 8 enclave slots
  │    4. Configure medeleg: delegate all except ecalls (cause 9, 11)
  │    5. Configure mideleg: delegate SSIP/STIP/SEIP to S-mode
  │    6. mepc = 0x8020_0000 (kernel entry), mstatus.MPP = S-mode
  │    7. mret → kernel boot
  │
  └─ hart 1..3: park in WFI loop until SBI HSM hart_start wakes them
```

### 2.3 SBI Trap Path (Enclave Call)

```
Kernel issues: ecall (a7=0x08424B45, a6=FID, a0=arg0, a1=arg1, a2=arg2)
  │
  ▼ mcause = 9 (ecall from S-mode)
m_trap_vector()
  ├─ reads mcause, identifies S-mode ecall
  ├─ captures a0..a2, a6, a7 from registers
  ├─ calls sbi_handler::dispatch(eid=a7, fid=a6, a0, a1, a2)
  │    └─ dispatch_enclave(fid, a0, a1, a2)
  │         ├─ FID 0: handle_enclave_create → enclave::enclave_create
  │         ├─ FID 1: handle_enclave_enter  → enclave::enclave_enter
  │         ├─ FID 2: handle_enclave_exit   → enclave::enclave_exit
  │         └─ FID 3: handle_enclave_attest → enclave::enclave_attest
  ├─ writes ret.error → a0, ret.value → a1
  ├─ advances mepc += 4 (except successful enter/exit — they manipulate mepc directly)
  └─ mret → returns to S-mode (or U-mode for enclave_enter)
```

---

## 3. Component Breakdown

### 3.1 Monitor Crate (`monitor/src/`)

The `veridian-monitor` crate is a `#![no_std] #![no_main]` RISC-V binary targeting M-mode. It has no heap allocator, no external cryptography crates, and no OS dependencies. All state is in statically declared arrays.

| File | Responsibility |
|---|---|
| `main.rs` | M-mode entry point, hart parking, TrapFrame definition, m_trap_vector |
| `pmp.rs` | NAPOT encoding, PMP CSR read/write helpers, lock/unlock/grant/self-protect |
| `enclave.rs` | EnclaveState enum, Enclave struct, ENCLAVE_POOL, lifecycle functions |
| `attest.rs` | SHA-256 (FIPS 180-4), HMAC-SHA-256 (RFC 2104), sign_measurement |
| `sbi_handler.rs` | SbiRet type, EID/FID dispatch, inline timer/putchar/HSM/IPI handlers |

**TrapFrame layout** — The M-mode trap frame saves all 32 RISC-V integer registers plus `mepc`. The layout is `#[repr(C)]` and must match the assembly save/restore sequence in the trap entry stub:

```rust
#[repr(C)]
pub struct TrapFrame {
    pub ra: usize,  pub sp: usize,  pub gp: usize,  pub tp: usize,
    pub t0: usize,  pub t1: usize,  pub t2: usize,  pub s0: usize,
    pub s1: usize,
    pub a0: usize,  // SBI error return / arg0
    pub a1: usize,  // SBI value return / arg1
    pub a2: usize,  pub a3: usize,  pub a4: usize,  pub a5: usize,
    pub a6: usize,  // SBI FID
    pub a7: usize,  // SBI EID
    // s2..s11, t3..t6
    pub mepc: usize,
}
```

The frame lives on the **M-mode stack**, which is protected by PMP entry 15. No lower privilege level can inspect it.

### 3.2 Kernel Enclave Bridge (`kernel/src/enclave/mod.rs`)

The bridge translates syscalls from U-mode into SBI ecalls to the monitor. It is the only kernel code that crosses the privilege boundary into M-mode for enclave operations. The trust model is explicit:

```
User Process (U-mode)
    │  SYS_ENCLAVE_* syscall (ecall, a7=120..123)
Kernel (S-mode)  ← this module
    │  SBI ecall (ecall, a7=0x08424B45)
M-mode Monitor
    │  PMP configuration + attestation
RISC-V Hardware
```

The kernel validates user-supplied arguments (pointer range, alignment, power-of-two size) before issuing the SBI call. Physical address translation for the report buffer uses the current process's page table — the kernel converts the user virtual address to a physical address that the monitor can write to directly.

```rust
// Argument validation before issuing SBI call
pub fn sys_enclave_create(phys_addr: usize, size: usize, entry_pa: usize) -> isize {
    if size == 0 || !size.is_power_of_two() { return -22; }
    if phys_addr & (size - 1) != 0 { return -22; }
    if entry_pa < phys_addr || entry_pa >= phys_addr.saturating_add(size) { return -22; }
    // ... SBI call proceeds
}
```

### 3.3 Syscall ABI (`kernel/src/syscall/numbers.rs`)

Four syscalls are allocated in the Phase 12 range:

```
SYS_ENCLAVE_CREATE  = 120
SYS_ENCLAVE_ENTER   = 121
SYS_ENCLAVE_EXIT    = 122
SYS_ENCLAVE_ATTEST  = 123
```

These are registered in `kernel/src/syscall/mod.rs` and dispatched to `kernel/src/enclave/mod.rs`. Error codes from the monitor (negative `SBI_ERR_*` values) propagate directly to the calling user process.

---

## 4. PMP Configuration Deep Dive

### 4.1 RISC-V PMP Overview

Physical Memory Protection is a RISC-V hardware mechanism exclusive to M-mode. Every memory access at S-mode or U-mode is checked against up to 16 PMP entries (QEMU virt provides 16). If an access matches an entry and the entry denies the access type, the CPU raises a PMP fault. M-mode is exempt from PMP checks by default, giving the monitor unrestricted access to configure the protection.

Each PMP entry consists of:
- `pmpaddr<n>` CSR: the encoded address of the protected region
- A byte within `pmpcfg<n/4>` CSR: 8-bit config with R, W, X permission bits, A (address mode) field, and L (Lock) bit

### 4.2 NAPOT Encoding Math

VeridianOS uses **NAPOT** (Naturally Aligned Power-Of-Two) mode exclusively. It is the most efficient mode for enclave regions because it requires only one PMP entry per region.

For a region of size `2^k` bytes starting at `base` (where `base` must be `2^k`-aligned):

```
pmpaddr = (base >> 2) | ((size / 8) - 1)
```

The implementation in `pmp.rs`:

```rust
#[inline]
fn napot_encode(base: usize, size: usize) -> usize {
    // pmpaddr = (base >> 2) | ((size/8) - 1)
    (base >> 2) | ((size / 8) - 1)
}
```

**Worked example** — 16 KiB enclave at `0x8610_0000`:

```
size = 16 * 1024 = 0x4000
k    = 14
pmpaddr = (0x8610_0000 >> 2) | ((0x4000 / 8) - 1)
        = 0x2184_0000 | 0x1FF
        = 0x2184_01FF
```

The hardware decodes this back to the 16 KiB aligned region `[0x8610_0000, 0x8614_0000)`.

### 4.3 PMP Slot Assignment

```
Entry 0..=7   Enclave slots 0..=7 (one PMP entry per enclave)
Entry 8..=14  Reserved for future use (kernel heap guard, I/O MMIO regions)
Entry 15      Monitor self-protection (Lock bit set at boot, permanent)
```

The one-to-one mapping between enclave slot index and PMP entry index means finding the PMP slot for an enclave is an O(1) lookup: `enclave.pmp_slot == enclave.id`.

### 4.4 PMP Configuration Bit Fields

```rust
const PMP_R:      u8 = 1 << 0;   // Readable by S/U-mode
const PMP_W:      u8 = 1 << 1;   // Writable by S/U-mode
const PMP_X:      u8 = 1 << 2;   // Executable by S/U-mode
const PMP_A_NAPOT: u8 = 0b11 << 3; // Address mode: NAPOT
const PMP_L:      u8 = 1 << 7;   // Lock: prevents further modification
```

### 4.5 Region Lifecycle

```
enclave_create():
  grant_region(slot, base, size)
  → pmpcfg: PMP_R | PMP_W | PMP_A_NAPOT  (no X, no L)
  → kernel can copy binary into region; cannot execute it yet
  → SHA-256 measurement computed at this point

enclave_enter():
  lock_region(slot, base, size)
  → pmpcfg: PMP_A_NAPOT only  (R=0 W=0 X=0, no L)
  → S-mode denied all access; enclave executes in U-mode via
    separate U-mode page table mappings set up by the kernel

enclave_exit():
  unlock_region(slot)
  → pmpcfg: 0 (A=OFF, entry disabled)
  → pmpaddr: 0
  → sfence.vma: ensures TLB/PMP cache consistency
  → kernel may now reclaim or zero the physical memory
```

### 4.6 Monitor Self-Protection

Called exactly once at `monitor_main` entry, before dropping to S-mode:

```rust
pub unsafe fn lock_monitor_self() {
    const MONITOR_BASE: usize = 0x8000_0000;
    const MONITOR_SIZE: usize = 256 * 1024; // 256 KiB

    let addr = napot_encode(MONITOR_BASE, MONITOR_SIZE);
    write_pmpaddr(MONITOR_SELF_ENTRY, addr);   // entry 15
    let cfg = PMP_A_NAPOT | PMP_L;             // no R/W/X, NAPOT, Lock
    write_pmpcfg_byte(MONITOR_SELF_ENTRY, cfg);
    core::arch::asm!("sfence.vma");
}
```

The `PMP_L` (Lock) bit is critical. It makes entry 15 **immutable until the next system reset** — even M-mode code cannot change it. This provides defense-in-depth: if a vulnerability in the monitor's SBI handlers were exploited to gain M-mode code execution, the attacker still cannot remove or modify the monitor's self-protection entry to expose the device key or enclave metadata.

The monitor image occupies `[0x8000_0000, 0x8004_0000)` — 256 KiB. This region contains the monitor binary, BSS (including `ENCLAVE_POOL`), and the M-mode stack. The kernel and all user processes are loaded above `0x8020_0000` and cannot access this range.

### 4.7 RV64 CSR Packing

On RV64, PMP configuration registers pack eight 8-bit entries per 64-bit CSR. Only even-numbered `pmpcfg` CSRs exist on RV64 (`pmpcfg0` = entries 0–7, `pmpcfg2` = entries 8–15; `pmpcfg1` and `pmpcfg3` are illegal):

```rust
unsafe fn write_pmpcfg_byte(entry: usize, cfg: u8) {
    let byte_shift = (entry % 8) * 8;
    let mask: usize = !(0xFF << byte_shift);
    match entry / 8 {
        0 => { let old = csr_read!("pmpcfg0");
               csr_write!("pmpcfg0", (old & mask) | ((cfg as usize) << byte_shift)); }
        _ => { let old = csr_read!("pmpcfg2");
               csr_write!("pmpcfg2", (old & mask) | ((cfg as usize) << byte_shift)); }
    }
}
```

PMP address CSRs (`pmpaddr0`–`pmpaddr15`) cannot be selected by a runtime register value — RISC-V assembly requires compile-time CSR addresses. The `write_pmpaddr` function uses a `match` over entry index to emit the correct `csrw pmpaddr<n>` instruction for each slot.

---

## 5. SBI Extension Specification

### 5.1 Extension Identity

| Field | Value |
|---|---|
| EID | `0x08424B45` |
| Name | VeridianOS Enclave Extension ("BKE") |
| Encoding | ASCII `BEL` `B` `K` `E` packed big-endian |
| SBI Spec Version | 2.0 |

The EID follows the SBI specification convention for vendor extensions (EIDs with the high byte set to a non-zero value). The kernel constant mirrors the monitor constant:

```rust
// Both monitor/src/sbi_handler.rs and kernel/src/enclave/mod.rs
pub const VERIDIAN_ENCLAVE_EID: usize = 0x08424B45;
```

### 5.2 SBI ABI Calling Convention

All SBI calls follow the SBI v2.0 register convention:

```
a7  = EID (Extension ID)
a6  = FID (Function ID within extension)
a0  = arg0
a1  = arg1
a2  = arg2
─── return ───
a0  = error code (0 = SBI_SUCCESS, negative = error)
a1  = return value (meaningful only when a0 == 0)
```

### 5.3 Function Table

| FID | Function | Arguments | Return (a1) | Description |
|---|---|---|---|---|
| 0 | `enclave_create` | a0=phys_addr, a1=size, a2=entry_pa | enclave_id (u8) | Allocate slot, compute measurement, grant S-mode R/W for loading |
| 1 | `enclave_enter` | a0=enclave_id | — | Seal PMP, drop to U-mode at entry_pa; returns after enclave exits |
| 2 | `enclave_exit` | a0=enclave_id | — | Unlock PMP, restore saved S-mode context |
| 3 | `enclave_attest` | a0=enclave_id, a1=report_phys | — | Write 73-byte attestation report to physical address |

### 5.4 Error Codes

| Code | Name | Meaning |
|---|---|---|
| 0 | `SBI_SUCCESS` | Operation completed successfully |
| -1 | `SBI_ERR_FAILED` | Generic failure (e.g. no free slots) |
| -2 | `SBI_ERR_NOT_SUPPORTED` | Unknown FID |
| -3 | `SBI_ERR_INVALID_PARAM` | Misaligned address, bad enclave_id, entry out of range |
| -4 | `SBI_ERR_DENIED` | State machine violation (e.g. entering an already-running enclave) |
| -5 | `SBI_ERR_INVALID_ADDRESS` | NULL report_phys, or non-NAPOT-aligned base |
| -6 | `SBI_ERR_ALREADY_AVAILABLE` | Hart already started (HSM extension) |

### 5.5 Standard SBI Pass-Through

The monitor also handles the following standard SBI extensions inline (no OpenSBI dependency):

| EID | Extension | Handler |
|---|---|---|
| `0x54494D45` | Timer (sbi_set_timer) | Writes CLINT `mtimecmp`, sets `STIP` in `mip` |
| `0x01` | Legacy putchar | Writes to NS16550 UART at `0x1000_0000` |
| `0x48534D` | HSM (hart_start) | Sets `HART_STATES[hartid]`, triggers `MSIP` to wake secondary hart |
| `0x735049` | IPI (sbi_send_ipi) | Writes CLINT `msip` registers for target harts |

All other EIDs return `SBI_ERR_NOT_SUPPORTED`.

---

## 6. Enclave Lifecycle State Machine

### 6.1 States

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum EnclaveState {
    Empty   = 0,  // Slot unoccupied
    Created = 1,  // Memory allocated, measured; PMP grants S-mode R/W
    Running = 2,  // PMP sealed; CPU executing in U-mode inside enclave
    Exited  = 3,  // Enclave completed; slot pending reclamation
}
```

### 6.2 State Transition Diagram

```
                                enclave_create()
  ┌────────────────────────────────────────────────────┐
  │                                                    ▼
Empty ◄─── slot_allocated=false ─── Exited ──── Created
                                       ▲            │
                                   enclave_exit()   │ enclave_enter()
                                       │            ▼
                                    Running ◄───────┘
```

**Transition table:**

| From | To | Trigger | Guard |
|---|---|---|---|
| Empty | Created | `enclave_create(phys_addr, size, entry_pa)` | Free slot exists, size is power-of-two, base is NAPOT-aligned, entry is in-range |
| Created | Running | `enclave_enter(enclave_id)` | State == Created |
| Running | Exited | `enclave_exit(enclave_id)` | State == Running |
| Exited | Empty | Next `enclave_create()` claim | `allocated == false` |

Attempting any transition that violates the guard returns `SBI_ERR_DENIED`. The kernel cannot bypass this check because state is stored in M-mode BSS, which is PMP-protected by entry 15.

### 6.3 Per-Slot Metadata

```rust
#[repr(C)]
pub struct Enclave {
    pub id:             u8,          // 0..7, matches PMP slot index
    pub phys_start:     usize,       // Physical base address (NAPOT-aligned)
    pub size:           usize,       // Power-of-two byte count
    pub entry_pa:       usize,       // First instruction physical address
    pub state:          EnclaveState,
    pub measurement:    [u8; 32],    // SHA-256 of region at creation
    pub allocated:      bool,
    pub pmp_slot:       usize,       // Always == id
    pub saved_mepc:     usize,       // Kernel return address saved at enter
    pub saved_mstatus:  usize,       // Kernel mstatus saved at enter
}
```

The `saved_mepc` stores `kernel_mepc + 4` — the address of the instruction after the `SBI_ENCLAVE_ENTER` ecall. When the enclave exits, the monitor writes this value back to the `mepc` CSR before `mret`, so the kernel resumes at precisely the right point.

### 6.4 Context Switch Detail

On `enclave_enter`, the monitor:

1. Saves `mepc` (kernel return) and `mstatus` (kernel privilege state) into the slot.
2. Calls `pmp::lock_region` to deny S-mode all access.
3. Sets `mstatus.MPP = 00` (U-mode) — `mret` will drop to U-mode.
4. Clears `mstatus.MPIE` — enclave starts with interrupts disabled.
5. Writes `entry_pa` into `mepc`.
6. Returns to the trap handler, which executes `mret` — CPU enters U-mode at `entry_pa`.

On `enclave_exit`, the monitor:

1. Calls `pmp::unlock_region` — S-mode access restored.
2. Writes `saved_mepc` back to `mepc` CSR.
3. Writes `saved_mstatus` back to `mstatus` CSR (MPP = S-mode restored).
4. Marks slot `Exited`, `allocated = false`.
5. Returns; trap handler executes `mret` — CPU returns to S-mode at the saved return address.

---

## 7. Attestation Report Format

### 7.1 Report Layout (73 bytes)

```
Offset  Size  Type        Field
──────  ────  ──────────  ─────────────────────────────────────────
     0     1  u8          enclave_id
     1     8  u64 (LE)    phys_start — physical base address
     9     8  u64 (LE)    size — region size in bytes
    17    32  [u8; 32]    measurement — SHA-256 of [phys_start, phys_start+size)
    49    24  [u8; 24]    signature — HMAC-SHA-256 truncated to 192 bits
──────
Total: 73 bytes
```

All multi-byte integers are little-endian, matching RISC-V native byte order.

### 7.2 Measurement Scope

The SHA-256 measurement is computed over the **entire physical memory region** `[phys_start, phys_start + size)` at the moment `enclave_create` is called — after the kernel has copied the enclave binary into the region but before the enclave has been entered. The measurement is final at creation time and stored in the `Enclave` struct.

If the kernel modifies the enclave region after calling `enclave_create` (and before `enclave_enter` — when S-mode still has R/W access), the measurement in the attestation report will not match the code that actually ran. A remote verifier comparing the report measurement against a known-good binary hash will detect the discrepancy. This is by design: the measurement binds the attestation to a specific binary state, not to the region in general.

### 7.3 SHA-256 Implementation

The monitor implements SHA-256 per FIPS 180-4 with no heap allocation. A fixed 64-byte stack buffer is used for block processing:

```rust
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = H;          // FIPS 180-4 initial hash values
    let mut buf   = [0u8; 64]; // block buffer (stack-allocated)
    let mut buf_len = 0usize;
    let mut total_bits: u64 = 0;
    // ... streaming block processing, then padding with 0x80 and bit count
}
```

The initial hash values `H` and round constants `K` are the standard FIPS 180-4 constants (fractional parts of square roots and cube roots of the first primes). The implementation handles multi-block inputs of arbitrary length.

### 7.4 HMAC-SHA-256 Design

The signature uses HMAC-SHA-256 per RFC 2104:

```
HMAC(K, M) = SHA-256((K' ⊕ opad) || SHA-256((K' ⊕ ipad) || M))
```

where `K'` is the device key padded or hashed to 64 bytes, `ipad = 0x36` repeated, and `opad = 0x5C` repeated.

The full 32-byte HMAC is truncated to 24 bytes for the report:

```rust
pub fn sign_measurement(measurement: &[u8; 32]) -> [u8; 24] {
    let full_hmac = hmac_sha256(&DEVICE_KEY, measurement);
    let mut tag = [0u8; 24];
    tag.copy_from_slice(&full_hmac[..24]);
    tag
}
```

Truncation to 192 bits is safe per NIST SP 800-107 §5.2, which permits truncation to at least `L/2` bits where `L` is the hash output length (256 bits). The 24-byte signature fits in the fixed 73-byte report while maintaining 96 bits of collision resistance — sufficient for the non-repudiation purpose of attestation.

### 7.5 Device Key

In the Phase 12 scaffold, the device key is a compile-time constant:

```rust
const DEVICE_KEY: [u8; 32] = [
    0x56, 0x65, 0x72, 0x69, 0x64, 0x69, 0x61, 0x6e, // "Veridian"
    0x4f, 0x53, 0x4b, 0x65, 0x79, 0x30, 0x31, 0x30, // "OSKey010"
    0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE,
    0x13, 0x37, 0xC0, 0xDE, 0xFA, 0xCE, 0xFF, 0x00,
];
```

This key is never exposed to S-mode or U-mode — it lives in M-mode BSS, protected by PMP entry 15. For production deployment, replace this with a key read from OTP/eFuse memory (see Section 13).

### 7.6 Remote Verification Protocol

A remote verifier receives the 73-byte report and performs:

1. Extract `measurement` bytes from offsets 17–48.
2. Recompute `HMAC-SHA-256(device_key, measurement)` using the device's public key (distributed out-of-band via a device certificate).
3. Compare the first 24 bytes of the computed HMAC against the report `signature` field (offsets 49–72).
4. If the HMAC matches, the measurement was produced by a legitimate VeridianOS device in M-mode.
5. Compare `measurement` against a reference SHA-256 of the expected enclave binary. If they match, the enclave is running genuine, unmodified code.

---

## 8. Security Properties

### 8.1 Hardware Guarantees

The following properties are enforced by RISC-V hardware and cannot be subverted in software:

**PMP is M-mode-only.** Only code running in Machine mode can write `pmpaddr` and `pmpcfg` CSRs. The kernel (S-mode) and all user processes (U-mode) have no instruction that can modify PMP configuration. A kernel exploit cannot unlock enclave memory by modifying PMP registers.

**PMP violation raises a fault.** Any S-mode or U-mode access to a PMP-denied region raises a load/store/instruction access fault. The memory bus never delivers the requested data. There is no software-visible way to read a PMP-locked region without going through M-mode.

**Lock bit is reset-permanent.** PMP entries with the `L` bit set cannot be modified by any software — including M-mode — until the next cold or warm reset. Entry 15 (monitor self-protection) is locked at boot. An attacker who gains M-mode code execution cannot remove this protection without rebooting the platform.

**Enclave pool inaccessibility.** `ENCLAVE_POOL` lives in the monitor's BSS segment at `[0x8000_0000, 0x8004_0000)`. PMP entry 15 denies S-mode and U-mode read, write, and execute access to this range. The kernel cannot enumerate enclave slots, forge enclave IDs, or modify saved context fields.

### 8.2 What Phase 12 Does Not Protect Against

**Software side-channel attacks.** PMP enforces physical memory isolation but does not prevent timing side channels (cache timing, branch predictor state), shared microarchitectural state (speculative execution), or software-visible covert channels. An attacker in S-mode can infer information about enclave computation through shared cache sets, DRAM timing, or timer-based measurements. Defenses require cache partitioning, constant-time implementations, and microarchitectural mitigations not implemented in Phase 12.

**Physical attacks.** PMP does not protect against an attacker with physical access who can probe memory buses, perform cold-boot attacks, or tamper with the hardware directly. Trusted hardware security requires tamper-evident packaging and secure boot chains beyond the scope of a software-only Phase 12 implementation.

**IOMMU / DMA.** PMP applies to CPU-initiated memory accesses. A DMA-capable device (network card, storage controller) with access to the physical address range of an enclave can read or write enclave memory regardless of PMP configuration. Preventing DMA attacks requires an IOMMU that applies equivalent access controls to DMA operations — not implemented in the QEMU virt platform.

**Multi-hart enclave atomicity.** The current implementation assumes single-hart operation for the enclave lifecycle. The `ENCLAVE_POOL` static is not protected by a spinlock between harts. For SMP deployments, concurrent `enclave_create` calls from different harts could race. Phase 12.5 will add a hart-indexed lock or atomic compare-and-swap for slot allocation.

**Enclave memory re-use without zeroing.** After `enclave_exit`, the monitor unlocks the physical region and marks the slot free. The kernel is responsible for zeroing the memory before reuse. The monitor does not zero on exit because it would need to re-acquire write access (calling `grant_region`), write zeros, and then release — adding complexity without a strong security benefit given that the kernel controls what occupies the memory next.

### 8.3 Trust Boundary Summary

| Boundary | Protected By | Bypass Possible |
|---|---|---|
| Enclave memory vs. S-mode | PMP hardware | No (requires physical attack or reset) |
| Monitor image vs. S-mode | PMP entry 15 (L bit) | No (requires reset) |
| Device key vs. S-mode | PMP entry 15 | No |
| Attestation report content | M-mode write via HMAC key | No (key in PMP-protected BSS) |
| Enclave code vs. DMA | — | Yes (no IOMMU) |
| Enclave data vs. cache timing | — | Yes (no cache partitioning) |

---

## 9. Syscall Interface

### 9.1 Overview

The four Phase 12 syscalls are the user-facing interface. They are in the range `[120, 123]`, above Phase 11's distributed syscalls `[90, 101]` and the exception handler syscalls `[110, 111]`. User programs call them via the standard RISC-V `ecall` instruction with the syscall number in `a7`.

### 9.2 SYS_ENCLAVE_CREATE (120)

```
a7 = 120
a0 = phys_addr    — physical base address of the enclave region
a1 = size         — region size in bytes (power of two, >= 8)
a2 = entry_pa     — physical entry point address

Returns (a0):
  >= 0  enclave_id (u8, 0..=7) on success
  -22   -EINVAL: size not power-of-two, phys_addr misaligned, entry out of range
  -1    SBI_ERR_FAILED: no free enclave slots
  -5    SBI_ERR_INVALID_ADDRESS: phys_addr not NAPOT-aligned
```

The kernel allocates the physical region (via `SYS_MAP` or a trusted page allocator) before calling this syscall. The monitor does not allocate physical memory; it accepts a region the kernel designates and locks it.

### 9.3 SYS_ENCLAVE_ENTER (121)

```
a7 = 121
a0 = enclave_id   — ID returned by SYS_ENCLAVE_CREATE

Returns (a0):
  0   success — returned after enclave calls SYS_ENCLAVE_EXIT
  -22 -EINVAL: enclave_id > 255
  -4  SBI_ERR_DENIED: enclave not in Created state
```

This syscall is **non-returning in the conventional sense**: it returns to the caller only after the enclave payload calls `SYS_ENCLAVE_EXIT`. The calling thread blocks at the kernel level while the CPU executes inside the enclave. From the caller's perspective, `SYS_ENCLAVE_ENTER` and `SYS_ENCLAVE_EXIT` behave as a matched pair — enter returns 0 when exit succeeds.

### 9.4 SYS_ENCLAVE_EXIT (122)

```
a7 = 122
a0 = enclave_id   — the running enclave's own ID

Returns (a0):
  0   success — CPU returns to S-mode at the instruction after SYS_ENCLAVE_ENTER
  -22 -EINVAL: enclave_id > 255
  -4  SBI_ERR_DENIED: enclave not in Running state
```

This syscall is issued from **inside the enclave** (U-mode), not from the kernel. The enclave payload calls it when it has finished its computation. The monitor unlocks the PMP entry, restores saved S-mode context, and `mret`s to the kernel at the saved return address.

### 9.5 SYS_ENCLAVE_ATTEST (123)

```
a7 = 123
a0 = enclave_id       — enclave to attest
a1 = report_buf_ptr   — user-space pointer to a 73-byte output buffer

Returns (a0):
  0   success — report written at report_buf_ptr
  -22 -EINVAL: enclave_id > 255 or report_buf_ptr == NULL
  -14 -EFAULT: report_buf_ptr not accessible/writable by the calling process
  -4  SBI_ERR_DENIED: enclave slot not allocated
```

The kernel translates `report_buf_ptr` from the user virtual address space to a physical address before passing it to the monitor. The monitor writes the 73-byte report directly to the physical address using `core::ptr::copy_nonoverlapping`. The kernel cannot intercept or modify the report content between the monitor write and the user process read.

### 9.6 Error Propagation

SBI error codes (negative `isize` values) from the monitor are propagated directly to user space without transformation. The kernel does not remap them to POSIX codes, with two exceptions: argument validation failures before the SBI call are returned as `-EINVAL (-22)`, and page table translation failures for the report buffer are returned as `-EFAULT (-14)`.

---

## 10. AgentRecord Integration

### 10.1 The enclave_id Field

Phase 9 introduced `AgentRecord` — the kernel-tracked metadata for each AI agent. Phase 12 adds one field:

```rust
pub struct AgentRecord {
    pub id:          AgentId,
    pub parent_id:   AgentId,
    pub state:       AgentState,
    pub intent:      [u8; MAX_INTENT_LEN],
    pub pid:         usize,
    pub valid:       bool,
    /// Phase 12: enclave_id is `Some(id)` when this agent runs inside a
    /// hardware-isolated TEE enclave managed by the M-mode monitor.
    /// `None` means the agent runs in ordinary S-mode kernel-managed memory.
    pub enclave_id:  Option<u8>,
}
```

The `Option<u8>` value `None` means the agent runs in ordinary kernel-managed memory with standard process isolation but no hardware TEE guarantees. `Some(id)` means the agent's computation runs inside a PMP-locked enclave managed by the monitor, and the enclave `id` can be passed to `SYS_ENCLAVE_ATTEST` to obtain a hardware-signed measurement report.

### 10.2 How an Agent Gets Hardware Isolation

The intended workflow for an AI agent requiring TEE isolation:

1. The agent runtime (S-mode) allocates a physically contiguous, NAPOT-aligned region for the agent binary.
2. The agent spawner calls `SYS_ENCLAVE_CREATE(phys_addr, size, entry_pa)` and stores the returned `enclave_id` in `AgentRecord.enclave_id`.
3. Before entering the enclave, the spawner may call `SYS_ENCLAVE_ATTEST` to obtain a measurement report and verify the binary matches the expected hash.
4. The spawner calls `SYS_ENCLAVE_ENTER(enclave_id)`. The CPU drops to U-mode inside the locked region.
5. When the agent computation completes, the enclave payload calls `SYS_ENCLAVE_EXIT`. Control returns to the spawner.
6. On agent termination, the spawner zeroes and frees the physical memory. `AgentRecord.enclave_id` is reset to `None`.

### 10.3 Integration with Phase 11 Distributed Capabilities

An agent with a non-None `enclave_id` can use Phase 11's `SYS_CAP_EXPORT` to share capabilities with remote kernel domains. The receiving domain can call `SYS_ENCLAVE_ATTEST` on the enclave's report before accepting the capability, establishing a chain of trust: the capability was granted by a specific binary (measured by SHA-256) running on legitimate hardware (verified by HMAC with device key). This is the foundation of hardware-attested cross-domain agent trust, to be formalized in Phase 13.

---

## 11. Test Suite

### 11.1 Integration Test Program (`user_programs/enclave_test/`)

The test program is a `no_std` U-mode binary that runs as a user-space process and exercises the full enclave lifecycle. It is included in the RAMFS disk image and spawned by the kernel's init sequence.

The program re-implements SHA-256 and HMAC-SHA-256 from scratch (matching the monitor's implementation) so it can independently verify the attestation signature without trusting any shared library.

### 11.2 Test 1 — Enclave Creation

```
[USER] TEST 1: Creating enclave...
  phys_addr = 0x8610_0000
  size      = 0x4000 (16 KiB)
  entry_pa  = 0x8610_0000
  Zero the enclave VMO at virtual 0x4010_0000
  SYS_ENCLAVE_CREATE → enclave_id (should be 0..7)
  Assert: enclave_id >= 0
[USER] TEST 1 PASSED.
```

This test confirms that the monitor accepts a validly-aligned region, computes the measurement, and returns a slot ID. It verifies the argument validation path (power-of-two size, NAPOT alignment, entry within region) is working end-to-end through the kernel bridge.

### 11.3 Test 2 — Enclave Entry and Exit

```
[USER] TEST 2: Preparing and entering enclave...
  Write 3-instruction payload at virtual 0x4010_0000:
    li a0, 0          // 0x13, 0x05, 0x00, 0x00
    li a7, 122        // 0x93, 0x08, 0xA0, 0x07  (SYS_ENCLAVE_EXIT)
    ecall             // 0x73, 0x00, 0x00, 0x00
  SYS_ENCLAVE_ENTER(enclave_id)
  Assert: returns 0 (clean exit from enclave)
[USER] Returned from enclave cleanly.
[USER] TEST 2 PASSED.
```

The enclave payload is three RISC-V instructions that immediately call `SYS_ENCLAVE_EXIT`. This verifies the full enter/exit round-trip: PMP seal on enter, PMP unlock on exit, kernel context restoration, and `mret` back to S-mode.

### 11.4 Test 3 — Remote Attestation and HMAC Verification

```
[USER] TEST 3: Generating attestation report...
  SYS_ENCLAVE_ATTEST(enclave_id, report_buf_ptr) → 73-byte report
  Assert: report[0] == enclave_id
  Assert: report[1..9] == phys_addr (LE u64)
  Assert: report[9..17] == size (LE u64)
  Recompute HMAC-SHA-256(DEVICE_KEY, report[17..49]) in user space
  Compare first 24 bytes against report[49..73]
  Assert: HMAC matches
[USER]   HMAC-SHA-256 signature verified successfully.
[USER] TEST 3 PASSED.
```

This test is the most security-significant: it cryptographically verifies that the attestation report was produced by the monitor using the expected device key. A forge or corruption of any field (measurement, phys_start, size) would produce a different HMAC. The test program recomputes the HMAC independently, confirming end-to-end correctness of the attestation pipeline.

### 11.5 Expected Console Output

```
[USER] VeridianOS Phase 12 Security Monitor Verification
[USER] ====================================================

[USER] TEST 1: Creating enclave...
[ENCLAVE] Created enclave id=0 phys=0x86100000 size=0x4000
[USER]   Enclave created successfully, ID = 0x0
[USER] TEST 1 PASSED.

[USER] TEST 2: Preparing and entering enclave...
[USER]   Entering enclave...
[ENCLAVE] Enclave 0 entered (returned from monitor)
[USER]   Returned from enclave cleanly.
[USER] TEST 2 PASSED.

[USER] TEST 3: Generating attestation report...
[ENCLAVE] Attestation report generated for enclave 0
[USER]   Attestation report generated successfully.
[USER]     Report enclave_id: 0x0
[USER]     Report base PA: 0x86100000
[USER]     Report size: 0x4000
[USER]     HMAC-SHA-256 signature verified successfully.
[USER] TEST 3 PASSED.

[USER] ====================================================
[USER] Enclave Lifecycle & Attestation — ALL TESTS PASSED!
[USER] ====================================================
```

---

## 12. QEMU Build and Boot

### 12.1 Build Commands

```bash
# Build the M-mode monitor binary
make build_monitor
# Output: target/riscv64gc-unknown-none-elf/release/veridian-monitor

# Build the kernel
make build_kernel
# Output: target/riscv64gc-unknown-none-elf/release/veridian-kernel

# Build the enclave_test user program and disk image
make disk
# Output: disk.img (ustar TAR containing enclave_test and other programs)

# Build everything
make build
```

### 12.2 Cargo Workspace Configuration

The monitor is a separate crate in the workspace:

```
Cargo.toml (workspace)
  members = [
    "kernel",           # veridian-kernel (S-mode)
    "monitor",          # veridian-monitor (M-mode)
    "user_programs/...",
  ]
```

The monitor crate has its own `Cargo.toml` with `name = "veridian-monitor"`, a custom linker script placing the binary at `0x8000_0000`, and `[profile.release]` settings that eliminate the standard library (`panic = "abort"`, `opt-level = "s"` for minimal binary size).

### 12.3 QEMU Invocation

The monitor binary is loaded as the BIOS firmware, before OpenSBI and before the kernel:

```bash
qemu-system-riscv64 \
  -machine virt \
  -nographic \
  -serial mon:stdio \
  -bios target/riscv64gc-unknown-none-elf/release/veridian-monitor \
  -kernel target/riscv64gc-unknown-none-elf/release/veridian-kernel \
  -smp 4 \
  -drive id=hd0,file=disk.img,format=raw,if=none \
  -device virtio-blk-device,drive=hd0 \
  -device virtio-net-device \
  -netdev user,id=net0
```

**Load addresses (QEMU virt machine):**
- Monitor (BIOS): `0x8000_0000` (loaded by QEMU as `-bios`, executed first in M-mode)
- Kernel: `0x8020_0000` (loaded by QEMU as `-kernel`; entered via `mret` from monitor)

The `-bios` flag tells QEMU to load the provided binary as the Machine-mode firmware, replacing the default OpenSBI. The monitor provides a minimal SBI implementation sufficient for the kernel to boot (timer, putchar, HSM, IPI).

### 12.4 Memory Map

```
Physical Address    Content
────────────────    ───────────────────────────────────────────
0x0000_0000         CLINT (Core-Local Interruptor)
0x0200_0000           msip registers  (0x0200_0000 + 4 * hartid)
0x0200_4000           mtimecmp[0]     (timer compare for hart 0)
0x0200_BFF8           mtime           (global machine timer)
0x0C00_0000         PLIC (Platform-Level Interrupt Controller)
0x1000_0000         UART0 (NS16550 compatible, 8 bytes)
0x8000_0000         Monitor image (256 KiB, PMP entry 15 locked)
0x8004_0000         (free — between monitor and kernel)
0x8020_0000         Kernel image (~2 MiB)
0x8610_0000         Enclave region used in enclave_test (16 KiB example)
```

---

## 13. Production Deployment Notes

### 13.1 OTP Device Key Replacement

The Phase 12 scaffold uses a hardcoded `DEVICE_KEY` in M-mode BSS. Before any production or pre-production deployment, replace the compile-time constant with a runtime read from the platform's One-Time Programmable fuse array or equivalent secure storage:

```rust
// Production replacement for DEVICE_KEY initialization
fn read_device_key() -> [u8; 32] {
    // Read from platform-specific OTP/eFuse CSR or MMIO
    // Example: RISC-V platforms may expose this through a vendor CSR
    // or through a memory-mapped security controller at a known address.
    // The key must be readable only from M-mode (enforce via PMP if MMIO).
    unsafe {
        // Platform-specific: read 32 bytes from OTP MMIO at BASE_ADDR
        let otp_base = 0xFFFF_0000 as *const u8;  // platform-specific
        let mut key = [0u8; 32];
        for i in 0..32 {
            key[i] = core::ptr::read_volatile(otp_base.add(i));
        }
        key
    }
}
```

The OTP read must occur before `init_pool()` in `monitor_main`, and the key must be stored only in M-mode-accessible memory (the monitor BSS, protected by PMP entry 15).

### 13.2 Asymmetric Attestation

The current HMAC-SHA-256 scheme is a **symmetric** construction: the verifier must know the device key to verify the HMAC. This requires the verifier to possess secret material, which is impractical at scale.

Production deployment should replace HMAC with an **asymmetric signature** (Ed25519 preferred for code-size efficiency on RISC-V, or ECDSA P-256 for FIPS compliance):

```
Signing:   signature = Ed25519_sign(DEVICE_PRIVATE_KEY, measurement)
Verifying: Ed25519_verify(DEVICE_PUBLIC_KEY, measurement, signature)
```

The device private key stays in OTP. The device public key is distributed via a device certificate signed by a platform CA. Verifiers only need the public key (or the CA certificate) — no secret material crosses the verification boundary.

The attestation report layout would grow to accommodate the 64-byte Ed25519 signature, and the report format version field would need to be added to distinguish HMAC-SHA-256 (v1) from Ed25519 (v2).

### 13.3 HSM Integration

For high-security deployments where even M-mode code should not have direct access to signing keys, the attestation operation should be delegated to a Hardware Security Module (HSM) or Platform Security Processor (PSP) via a side-channel (MMIO or a dedicated interconnect). The monitor would forward the measurement to the HSM and receive back a signed token, never holding the private key in its address space.

### 13.4 Secure Boot Chain

For the attestation to be meaningful, the monitor binary itself must be verified before it runs. A complete secure boot chain would be:

```
ROM Boot Loader
  → verifies monitor signature against fused public key hash
  → loads monitor at 0x8000_0000
Monitor (M-mode)
  → verifies kernel hash (measured boot, stored in monitor extension)
  → loads kernel at 0x8020_0000
Kernel (S-mode)
  → verifies enclave binary before calling SYS_ENCLAVE_CREATE
```

Phase 12 does not implement measured boot for the kernel. The monitor assumes the kernel at `0x8020_0000` is trusted. Adding kernel measurement would involve storing a reference hash in OTP and comparing it during `monitor_main` before the `mret` to S-mode.

---

## 14. Limitations and Future Work

### 14.1 Phase 12 Limitations

**Static 8-slot pool.** The `ENCLAVE_POOL` array has exactly 8 entries. This is sufficient for the QEMU simulation but limits concurrent enclave count in production. A larger pool requires either increasing the static array (consuming more M-mode BSS) or implementing a dynamic allocator in M-mode (significant added complexity).

**HMAC not asymmetric.** As described in Section 13.2, the HMAC construction requires the verifier to hold the device key. This is appropriate for a Phase 12 scaffold but unsuitable for production remote attestation.

**No multi-hart enclave support.** The current implementation assumes a single CPU hart executes the enclave. Multi-hart (SMP) enclaves — where multiple harts share access to the same enclave region — require synchronization on the `ENCLAVE_POOL` and coordinated PMP configuration across harts. This is deferred to Phase 12.5.

**No kernel memory accounting.** The kernel does not currently enforce that the physical address passed to `SYS_ENCLAVE_CREATE` belongs to memory that was legitimately allocated by the calling process. A malicious process could pass an arbitrary physical address (including kernel code or another process's memory) and potentially use the measurement to leak information about that memory.

**Single PMP entry per enclave.** The current scheme uses one PMP entry to deny S-mode all access to the enclave region. The enclave runs in U-mode using kernel-configured page table mappings. A more robust scheme would use two PMP entries per enclave: one deny-all for S-mode, one explicit allow for U-mode with only the permissions needed (R/X, not W, for code regions). This limits damage if the enclave is itself compromised.

**No side-channel mitigations.** As detailed in Section 8.2, timing attacks through shared CPU microarchitectural state are not addressed.

### 14.2 Phase 12.5 Planned Work

- **SMP safety**: Add atomic compare-and-swap or a per-hart mutex to `ENCLAVE_POOL` slot allocation.
- **Physical address validation**: Cross-reference `phys_addr` against the kernel's physical memory allocator before passing to the monitor.
- **Enclave-to-enclave IPC**: Allow two enclaves on the same platform to exchange messages through a monitor-mediated shared memory region, verified by both measurements.
- **Measurement-bound capabilities**: Extend the Phase 11 capability system so a capability can carry an enclave measurement constraint — the capability is only exercisable by code whose measurement matches the constraint.

### 14.3 Phase 13 Planned Work

- **Asymmetric attestation**: Replace HMAC-SHA-256 with Ed25519 signatures using an OTP-provisioned key.
- **Measured kernel boot**: Extend `monitor_main` to hash the kernel image and store the measurement, creating a boot-time chain of trust.
- **Cross-domain attestation**: Integrate the attestation report format with Phase 11's DCTP so remote kernel domains can verify the enclave identity of a capability before accepting it.
- **Formal security properties**: Define the security invariants of the PMP+enclave model as a set of predicates and verify them against the implementation, following the seL4 methodology.

---

## 15. Academic References

**[1] Lee, D., et al. (2020). "Keystone: An Open Framework for Architecting TEEs."** *EuroSys '20.*  
Keystone is the primary academic reference for Phase 12. The VeridianOS monitor architecture follows Keystone's security monitor design: an M-mode firmware that manages PMP regions for enclaves, exposes an SBI-based API to the kernel, and produces attestation reports. Key differences: VeridianOS uses a static enclave pool (no heap), truncated HMAC rather than Keystone's full signing chain, and a tighter integration with the Phase 9 agent runtime.

**[2] RISC-V International. (2021). "RISC-V Privileged Architecture Specification v1.12."**  
§3.6 (Physical Memory Protection) is the definitive reference for PMP CSR layout, NAPOT encoding, the Lock bit semantics, and the `sfence.vma` requirement after PMP changes. The `napot_encode` formula in `pmp.rs` is derived directly from §3.6.1.

**[3] RISC-V International. (2022). "RISC-V Supervisor Binary Interface Specification v2.0."**  
Defines the SBI ecall ABI (EID/FID in a7/a6, return values in a0/a1), standard extension IDs, and error codes. Phase 12 follows the SBI v2.0 convention for the custom vendor extension EID `0x08424B45`.

**[4] National Institute of Standards and Technology. (2015). "FIPS PUB 180-4: Secure Hash Standard."** *NIST.*  
The SHA-256 implementation in `attest.rs` follows FIPS 180-4 §6.2 exactly: initial hash values from §5.3.3, round constants from §4.2.2, message schedule from §6.2.2, and padding from §5.1.1. The implementation has no external dependencies and is verified by the enclave_test program recomputing the HMAC independently.

**[5] Krawczyk, H., Bellare, M., & Canetti, R. (1997). "HMAC: Keyed-Hashing for Message Authentication."** *RFC 2104.*  
The HMAC construction in `attest.rs` follows RFC 2104 §2 verbatim: ipad `0x36`, opad `0x5C`, key normalization, inner and outer hash. The truncation of the HMAC output to 24 bytes is justified by NIST SP 800-107 §5.2 (truncation to at least `L/2` bits is safe).

**[6] Costan, V., & Devadas, S. (2016). "Intel SGX Explained."** *IACR ePrint 2016/086.*  
Comprehensive treatment of the security properties, measurement model, and attestation protocol of Intel SGX. Phase 12's measurement-at-creation design (§7.2) and the remote attestation workflow (§7.6) are directly analogous to SGX's enclave measurement and remote attestation, adapted for RISC-V PMP rather than Intel's memory encryption engine.

**[7] Sabt, M., Achemlal, M., & Bouabdallah, A. (2015). "Trusted Execution Environment: What It Is, and What It Is Not."** *IEEE TrustCom '15.*  
Provides the taxonomy of TEE properties used in Section 8: confidentiality (data not readable outside TEE), integrity (code not modifiable without invalidating attestation), and isolation (enforcement by hardware, not software). Phase 12 satisfies confidentiality and isolation via PMP, and integrity via SHA-256 measurement, while explicitly acknowledging the side-channel and physical attack limitations documented in §8.2.

**[8] Waterman, A., & Asanović, K. (Eds.). (2019). "The RISC-V Instruction Set Manual, Volume I: Unprivileged ISA."** *RISC-V International.*  
§2.1 (RISC-V ecall instruction), §22 (Zicsr — CSR instructions). The `csrr`/`csrw` macros in `main.rs` and `pmp.rs` use the Zicsr extension instructions documented here.
