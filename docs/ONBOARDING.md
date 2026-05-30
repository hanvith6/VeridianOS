# 🧑‍💻 VeridianOS Developer Onboarding Guide

> **Welcome to VeridianOS** — an AI-native, capability-based microkernel written in Rust for RISC-V 64-bit.  
> This guide will take you from zero to making your first kernel contribution.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Environment Setup](#2-environment-setup)
3. [Building & Running](#3-building--running)
4. [Architecture Overview](#4-architecture-overview)
5. [Codebase Tour](#5-codebase-tour)
6. [How to Add a New Syscall](#6-how-to-add-a-new-syscall)
7. [How to Add a New Kernel Module](#7-how-to-add-a-new-kernel-module)
8. [Testing & Debugging Workflow](#8-testing--debugging-workflow)
9. [Common Gotchas & Fixes](#9-common-gotchas--fixes)
10. [Contribution Checklist](#10-contribution-checklist)

---

## 1. Prerequisites

### Required Tools

| Tool | Minimum Version | Purpose | Install Command |
|------|----------------|---------|----------------|
| **Rust** (via `rustup`) | Nightly | `no_std`, inline asm, custom targets | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **QEMU** | 7.0+ | RISC-V 64-bit emulator (`qemu-system-riscv64`) | `brew install qemu` (macOS) / `apt install qemu-system-misc` (Linux) |
| **GNU Make** | 3.81+ | Build orchestration | Preinstalled on macOS/Linux |
| **GDB** (optional) | Any | Kernel debugging via remote stub | `brew install riscv-software-src/riscv/riscv-gnu-toolchain` (macOS) |

### Rust Toolchain Details

VeridianOS requires **Rust Nightly** for the following unstable features:
- `#![no_std]` + `#![no_main]` — bare-metal binary without standard library
- `core::arch::global_asm!` — inline assembly for boot stub and trap vector
- `#[unsafe(no_mangle)]` — stable ABI for kernel entry point
- `spin` crate locks — used for kernel-wide mutual exclusion
- `linked_list_allocator = "0.10.6"` — heap allocator used by the kernel for dynamic allocation

The project ships a `rust-toolchain.toml` that pins the exact channel:

```toml
[toolchain]
channel = "nightly"
targets = ["riscv64gc-unknown-none-elf"]
components = ["rust-src", "llvm-tools"]
```

`rustup` reads this file automatically; **you don't need to manually install anything**.

### Platform Notes

| Host OS | Status | Notes |
|---------|--------|-------|
| macOS (Apple Silicon / Intel) | ✅ Fully supported | Use `brew` for QEMU |
| Ubuntu/Debian Linux | ✅ Fully supported | Use `apt` for QEMU |
| Windows (WSL2) | ⚠️ Works with caveats | Use WSL2 + Ubuntu; native Windows not tested |

---

## 2. Environment Setup

### Step 1 — Install Rust & the RISC-V Target

```bash
# Install rustup (skip if already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# rustup will automatically read rust-toolchain.toml when you enter the project
# But you can also manually add nightly + the target:
rustup toolchain install nightly
rustup target add riscv64gc-unknown-none-elf --toolchain nightly
rustup component add rust-src llvm-tools --toolchain nightly
```

### Step 2 — Install QEMU

```bash
# macOS
brew install qemu

# Ubuntu / Debian
sudo apt update && sudo apt install -y qemu-system-misc qemu-utils

# Fedora / RHEL
sudo dnf install -y qemu-system-riscv
```

Verify QEMU supports RISC-V:
```bash
qemu-system-riscv64 --version
# Expected output: QEMU emulator version 7.x.x or higher
```

### Step 3 — Clone & Verify

```bash
git clone https://github.com/hanvith6/VeridianOS.git
cd VeridianOS

# Verify the toolchain resolves correctly
rustup show
# Should display: riscv64gc-unknown-none-elf (installed)
```

### Step 4 — Install Optional GDB (for kernel debugging)

```bash
# macOS — RISC-V cross-GDB
brew tap riscv-software-src/riscv
brew install riscv-gnu-toolchain
# Provides: riscv64-unknown-elf-gdb

# Ubuntu
sudo apt install -y gcc-riscv64-linux-gnu gdb-multiarch
# Use: gdb-multiarch target/riscv64gc-unknown-none-elf/release/veridian-kernel
```

---

## 3. Building & Running

### Quick Start

```bash
# Build everything (kernel + user programs + disk image) and boot
make run
```

### Build Targets Reference

| Command | What It Does |
|---------|-------------|
| `make build` | Full build: user programs + disk image + kernel |
| `make run` | Build everything, then launch in QEMU |
| `make disk` | Build user programs and create `disk.img` (ustar TAR) |
| `make debug` | Build and launch QEMU with GDB server (`-s -S` flags) |
| `make clean` | Remove all build artifacts and `disk.img` |
| `make clippy` | Run `cargo clippy` with RISC-V target |
| `make fmt` | Check code formatting with `cargo fmt --check` |

### What Happens During `make run`

```
make run
  └─→ make disk
  │     ├─→ cargo build -p hello --release
  │     ├─→ cargo build -p neural_test --release
  │     └─→ tar cf disk.img --format=ustar hello neural_test
  └─→ make build_kernel
  │     ├─→ cargo build -p user_program --release  (legacy fallback)
  │     └─→ cargo build -p veridian-kernel --release
  └─→ cargo run -p veridian-kernel --release
        └─→ qemu-system-riscv64 -machine virt -bios default
              -kernel veridian-kernel
              -drive disk.img (VirtIO block device)
```

### Expected Boot Output

```
================================================================
 __      __        _     _ _             ____   _____
 \ \    / /       (_)   | (_)           / __ \ / ____|
  \ \  / /__ _ __ _  __| |_  __ _ _ __ | |  | | (___
   \ \/ / _ \ '__| |/ _` | |/ _` | '_ \| |  | |\___ \
    \  /  __/ |  | | (_| | | (_| | | | | |__| |____) |
     \/ \___|_|  |_|\__,_|_|\__,_|_| |_|\____/|_____/
================================================================
               VeridianOS Version 0.1.0-alpha
  Concept: AI-Native, Capability-Based Architecture (RISC-V 64)
================================================================

[BOOT] Booting CPU Hart ID: 0
[BOOT] Device Tree Blob located at physical address: 0xXXXXXXXX
[BOOT] Initializing memory management...
[BOOT] Memory management active (Sv39 Paging enabled).
...
[VIRTIO] Block device ready. Capacity: N sectors
[RAMFS] Loaded 2 file(s) from disk image.
[RAMFS] Found 'neural_test' (XXXXX bytes). Spawning process...
...
[SUCCESS] VeridianOS Phase 6 fully verified!
```

To exit QEMU: press **`Ctrl+A`** then **`X`**.

---

## 4. Architecture Overview

### Privilege Level Separation

```
┌─────────────────────────────────────────────────────────────────┐
│  M-MODE (Machine)  — OpenSBI Firmware + TEE Security Monitor    │
│  Handles: hardware init, SBI ecall forwarding, M-mode traps,    │
│           PMP-based enclave isolation (Phase 12 monitor/ crate) │
├─────────────────────────────────────────────────────────────────┤
│  S-MODE (Supervisor)  — VeridianOS Microkernel (4 harts, SMP)   │
│  Handles: paging, capabilities, scheduling, syscalls, IPC,      │
│           distributed DKCP/Raft, NES, heap (linked_list_alloc)  │
├─────────────────────────────────────────────────────────────────┤
│  U-MODE (User)  — Isolated Processes                            │
│  Handles: user apps (hello, neural_test, smp_test), user-space  │
│           exception handlers (registered via syscall 110)       │
└─────────────────────────────────────────────────────────────────┘
```

### Kernel Subsystem Map

```
kmain() in main.rs
│
├── uart::WRITER.init()          ← NS16550 UART at 0x10000000
├── trap::init()                 ← Write stvec CSR → trap_vector
├── memory::init()
│   ├── page_alloc: BuddyAllocator (4KB–4MB blocks, 128MB RAM)
│   ├── page_table: Sv39 3-level page table, satp activated
│   └── heap: linked_list_allocator (dynamic kernel heap)
│
├── smp::init()                  ← Wake harts 1–3 via SBI HSM sbi_hart_start
│
├── capability::                 ← Handles, Rights, HandleTable
│   ├── Handle { object_type, object_ptr, rights }
│   ├── HandleTable [64 slots per process]
│   └── channel::Channel [8-msg ring buffer, cap transfer]
│
├── process::
│   ├── Process { pid, state, page_table, handle_table }
│   ├── spawn()  ← ELF loader + U-mode entry
│   └── thread:: ← Per-hart round-robin scheduler (current_idx[4])
│       └── exception delivery: page faults → registered U-mode handler
│
├── syscall::                    ← ecall dispatcher (syscalls 1–123)
│   ├── numbers.rs (SYS_WRITE=1 … SYS_ENCLAVE_ATTEST=123)
│   └── mod.rs    (syscall_handler match table)
│
├── nes::                        ← Neural Execution Subsystem
│   ├── graph.rs      (TaskGraph, TaskNode, GraphPool[16])
│   ├── queue.rs      (HeterogeneousQueue ring[128] per device)
│   ├── validator.rs  (DFS cycle detection)
│   ├── simulator.rs  (CPU/GPU/NPU worker threads + EMA policy)
│   └── nes_dist.rs   (remote node dispatch, TicketPool[16])
│
├── dkcp::                       ← Distributed Multi-Kernel Layer
│   ├── ring.rs   (lock-free SPSC ring, 256 × 64 B)
│   ├── dctp.rs   (export / import / revoke, 128-bit UIDs)
│   └── raft.rs   (Leader/Follower/Candidate, AppendEntries, elections)
│
├── enclave_bridge.rs            ← S-mode → M-mode SBI bridge (EID 0x08424B45)
│
├── virtio::                     ← VirtIO device drivers
│   ├── blk.rs  (block device, 3-descriptor chain)
│   └── net.rs  (virtio-net for real inter-VM networking)
│
└── fs::ramfs                    ← ustar TAR InitRAMFS parser
```

### Syscall Table

| Range | Subsystem | Representative Syscalls |
|-------|-----------|------------------------|
| 1–2 | Core I/O & lifecycle | `SYS_WRITE` (1), `SYS_EXIT` (2) |
| 3 | Scheduler | `SYS_YIELD` (3) |
| 10–11 | Virtual Memory | `SYS_VMO_CREATE` (10), `SYS_VMO_MAP` (11) |
| 50–53 | NES graph | `SYS_GRAPH_CREATE` (50) … `SYS_GRAPH_WAIT` (53) |
| 60–63 | Semantic graph FS | `SYS_NODE_CREATE` (60) … `SYS_GRAPH_QUERY` (63) |
| 70–74 | Agent runtime | `SYS_AGENT_SPAWN` (70) … `SYS_AGENT_STATUS` (74) |
| 80 | Policy engine | `SYS_POLICY_CONFIGURE` (80) |
| 90–101 | Distributed DKCP / Raft / NES-dist | `SYS_DCAP_EXPORT` (90) … `SYS_DKCP_DISCONNECT` (101) |
| 110–111 | User-space exception delivery | `SYS_REGISTER_EXCEPTION_HANDLER` (110), `SYS_EXCEPTION_RETURN` (111) |
| 120–123 | M-mode TEE enclaves | `SYS_ENCLAVE_CREATE` (120) … `SYS_ENCLAVE_ATTEST` (123) |

### Key Design Principles

1. **No Ambient Authority** — Processes can only access resources they hold capability handles for. No UID, no `root`, no `/proc`.
2. **Unforgeable Handles** — Handle IDs are process-local table indices; a process cannot fabricate one.
3. **Rights Monotonicity** — Capability rights can only be reduced (`A ∩ mask`), never amplified.
4. **Microkernel Minimalism** — Only memory, scheduling, IPC, and capability enforcement run in S-mode. Everything else belongs in U-mode services.

### System Call Flow

```
User ecall instruction
      │
      ▼ (hardware trap to S-mode)
trap_vector (trap.S)        ← saves 32 GPRs + CSRs onto kernel stack
      │
      ▼
trap_handler() (trap.rs)    ← reads scause, dispatches
      │  scause = 8 (U-mode ecall)
      ▼
syscall_handler(id=a7, a0..a4)
      │
      ▼ (match on syscall number)
sys_write / sys_exit / sys_graph_create / ...
      │
      ▼
result written to tf.regs[10] (a0)
sepc += 4  (skip past ecall)
      │
      ▼
trap_vector restores all registers
sret → resume U-mode
```

---

## 5. Codebase Tour

### Directory Structure

```
VeridianOS/
├── Cargo.toml              ← Workspace: [kernel, monitor, hello, neural_test, user_program, smp_test]
├── Makefile                ← Build targets (build/run/disk/debug/clean)
├── rust-toolchain.toml     ← Pins nightly + riscv64gc-unknown-none-elf
├── disk.img                ← Generated ustar TAR (rebuilt by `make disk`)
├── plan.md                 ← Master development plan (17 sections)
│
├── monitor/                ← M-mode TEE Security Monitor (Phase 12)
│   ├── Cargo.toml          ← Separate crate; compiles to a distinct M-mode binary
│   └── src/
│       ├── lib.rs          ← M-mode entry point
│       ├── pmp.rs          ← PMP-based enclave isolation (16 entries)
│       ├── enclave.rs      ← EnclaveRecord pool + lifecycle state machine
│       ├── attest.rs       ← SHA-256 + HMAC-SHA-256 attestation (no_std, hand-rolled)
│       └── sbi_ext.rs      ← SBI extension EID 0x08424B45: enclave_create/enter/exit/attest
│
├── kernel/src/
│   ├── main.rs             ← kmain() — boot sequence orchestrator
│   ├── uart.rs             ← NS16550 UART at MMIO 0x10000000
│   ├── sbi.rs              ← SBI firmware calls (putchar, set_timer)
│   ├── trap.rs             ← Trap handler + U-mode transition
│   ├── panic.rs            ← #[panic_handler] for no_std
│   │
│   ├── arch/riscv64/
│   │   ├── boot.S          ← _start: stack init → call kmain
│   │   └── trap.S          ← trap_vector: save/restore all 32 GPRs
│   │
│   ├── memory/
│   │   ├── page_alloc.rs   ← Binary buddy allocator (order 0–10)
│   │   └── page_table.rs   ← Sv39 3-level page tables, satp
│   │
│   ├── capability/
│   │   ├── mod.rs          ← Handle, HandleTable, ObjectType
│   │   ├── rights.rs       ← Rights bitflags (READ/WRITE/EXECUTE/...)
│   │   └── channel.rs      ← Channel IPC, Message, cap transfer
│   │
│   ├── process/
│   │   ├── mod.rs          ← Process struct, spawn()
│   │   ├── thread.rs       ← Thread, ThreadContext, scheduler
│   │   └── elf.rs          ← ELF64 parser, PT_LOAD mapper
│   │
│   ├── syscall/
│   │   ├── numbers.rs      ← SYS_WRITE=1, SYS_EXIT=2, etc.
│   │   └── mod.rs          ← syscall_handler() dispatch table
│   │
│   ├── nes/
│   │   ├── mod.rs          ← NES init, GRAPH_POOL, DEVICE_QUEUES
│   │   ├── types.rs        ← OpType, DeviceType, TensorDescriptor
│   │   ├── graph.rs        ← TaskGraph, TaskNode (DAG structure)
│   │   ├── queue.rs        ← HeterogeneousQueue ring buffer
│   │   ├── validator.rs    ← DFS-based cycle detection
│   │   ├── simulator.rs    ← CPU/GPU/NPU software workers
│   │   └── syscalls.rs     ← sys_graph_create/add_node/submit/wait
│   │
│   ├── virtio/
│   │   ├── mod.rs          ← MMIO helpers, VirtIO constants
│   │   └── blk.rs          ← VirtIO block driver (3-desc chain)
│   │
│   └── fs/
│       ├── mod.rs          ← RamFs public API
│       └── ramfs.rs        ← ustar TAR parser, static RAM buffer
│
├── user_programs/
│   ├── hello/src/main.rs          ← Simple U-mode hello world
│   ├── neural_test/src/main.rs    ← NES DAG integration test
│   └── smp_test/src/main.rs       ← SMP verification: confirms all four harts schedule concurrently
│
└── docs/
    ├── ONBOARDING.md       ← This file
    ├── ARCHITECTURE.md     ← Architecture reference
    ├── NEURAL_SCHEDULER_DESIGN.md
    ├── ACADEMIC_REFERENCES.md
    └── FUTURE_COMPUTING_TRENDS.md
```

### Key Files to Read First

| Priority | File | Why |
|----------|------|-----|
| 1 | `kernel/src/main.rs` | Boot sequence; understand init order |
| 2 | `kernel/src/syscall/mod.rs` | Syscall dispatch — the kernel's public API |
| 3 | `kernel/src/capability/mod.rs` | Core security primitive |
| 4 | `kernel/src/nes/graph.rs` | NES DAG engine — the novel component |
| 5 | `user_programs/neural_test/src/main.rs` | How user space calls the kernel |

---

## 6. How to Add a New Syscall

This is a step-by-step walkthrough for adding `SYS_DEBUG_DUMP` (syscall number `10`) that dumps the current process's handle table to the UART.

### Step 1 — Register the Syscall Number

Edit `kernel/src/syscall/numbers.rs`:

```rust
/// Syscall: Dump the current process's handle table to UART (debug use only).
/// Registers:
/// - `a7` = SYS_DEBUG_DUMP (10)
pub const SYS_DEBUG_DUMP: usize = 10;
```

> [!IMPORTANT]
> Syscall numbers must be **unique and stable**. Never reuse a number for a different syscall.
> Allocation guide: 5–49 basic syscalls; 50–89 NES; 90–101 distributed DKCP/Raft;
> 110–111 user-space exception delivery; 120–123 M-mode TEE enclaves; 130+ reserved for future subsystems.

### Step 2 — Implement the Handler Function

Add the implementation in `kernel/src/syscall/mod.rs` (or a new file if complex):

```rust
/// SYS_DEBUG_DUMP — Dump the current process handle table to UART.
/// Returns: number of valid handles printed, or -EPERM if no process.
fn sys_debug_dump() -> isize {
    let process_guard = CURRENT_PROCESS.lock();
    let process = match process_guard.as_ref() {
        Some(p) => p,
        None => return -3, // -EPERM: no active process
    };

    let mut count = 0isize;
    println!("[DEBUG] Handle table dump for PID {}:", process.pid);
    for (idx, slot) in process.handle_table.slots.iter().enumerate() {
        if let Some(handle) = slot {
            println!("  [{:02}] type={:?} ptr=0x{:X} rights={:?}",
                idx, handle.object_type, handle.object_ptr, handle.rights);
            count += 1;
        }
    }
    println!("[DEBUG] {} active handles.", count);
    count
}
```

### Step 3 — Add to the Dispatch Table

In `kernel/src/syscall/mod.rs`, find the `match id { ... }` block and add your case:

```rust
pub fn syscall_handler(id: usize, arg0: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) -> isize {
    match id {
        numbers::SYS_WRITE           => sys_write(arg0, arg1),
        numbers::SYS_EXIT            => sys_exit(arg0),
        numbers::SYS_HANDLE_CLOSE    => sys_handle_close(arg0),
        numbers::SYS_HANDLE_DUPLICATE => sys_handle_duplicate(arg0, arg1),
        numbers::SYS_GRAPH_CREATE    => sys_graph_create(),
        numbers::SYS_GRAPH_ADD_NODE  => sys_graph_add_node(arg0, arg1, arg2, arg3, arg4),
        numbers::SYS_GRAPH_SUBMIT    => sys_graph_submit(arg0, arg1),
        numbers::SYS_GRAPH_WAIT      => sys_graph_wait(arg0, arg1),
        // ← ADD YOUR NEW SYSCALL HERE:
        numbers::SYS_DEBUG_DUMP      => sys_debug_dump(),
        _                            => -1, // ENOSYS
    }
}
```

### Step 4 — Expose the Syscall Number to User Space

User programs need to know the syscall number. Add it to the user program's own constants (each user binary is its own crate):

```rust
// In user_programs/my_program/src/main.rs (or a shared lib crate)
const SYS_DEBUG_DUMP: usize = 10;

fn syscall_debug_dump() -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") SYS_DEBUG_DUMP,
            lateout("a0") ret,
        );
    }
    ret
}
```

### Step 5 — Write a Test

Add a call in `user_programs/neural_test/src/main.rs` or the hello binary:

```rust
// Test the new syscall
let handles = syscall_debug_dump();
assert!(handles >= 0, "SYS_DEBUG_DUMP failed: {}", handles);
```

### Step 6 — Build and Verify

```bash
make run 2>&1 | grep -E "(DEBUG|ERROR)"
# Should show: [DEBUG] Handle table dump for PID 2: ...
```

### Syscall Design Checklist

- [ ] Unique syscall number (check `numbers.rs` for conflicts)
- [ ] All arguments validated (null pointers, kernel address ranges, alignment)
- [ ] Handle existence and type checked before dereference
- [ ] Rights checked before any operation (`handle.rights.contains(Rights::WRITE)`)
- [ ] Error codes follow the standard table (see `plan.md §14`)
- [ ] Return value fits in `isize` (negative = error, non-negative = success)
- [ ] Documented with the register convention in `numbers.rs`

---

## 7. How to Add a New Kernel Module

### Step 1 — Create the Module Directory

```bash
mkdir kernel/src/my_module
touch kernel/src/my_module/mod.rs
```

### Step 2 — Write the Module

```rust
// kernel/src/my_module/mod.rs
//! My Module — Brief description of purpose.
//!
//! This module provides XYZ functionality for VeridianOS.

use spin::Mutex;
use crate::println;

/// Global state for this module (if needed).
static MY_STATE: Mutex<Option<MyStruct>> = Mutex::new(None);

pub struct MyStruct {
    pub field: usize,
}

/// Initialize the module. Called from kmain() during boot.
pub fn init() {
    let s = MyStruct { field: 42 };
    *MY_STATE.lock() = Some(s);
    println!("[MY_MODULE] Initialized.");
}

/// Public API function.
pub fn do_something() -> usize {
    MY_STATE.lock().as_ref().map(|s| s.field).unwrap_or(0)
}
```

### Step 3 — Register in main.rs

```rust
// kernel/src/main.rs — add the module declaration:
pub mod my_module;

// Inside kmain(), call init() at the appropriate point:
println!("[BOOT] Initializing my_module...");
my_module::init();
println!("[BOOT] my_module ready.");
```

### Step 4 — Add Module to Cargo.toml (if it needs external crates)

The kernel's `Cargo.toml` already includes `spin` for locks. If your module needs a new dependency:

```toml
# kernel/Cargo.toml
[dependencies]
spin = { version = "0.9", features = ["mutex"] }
# Add your new dep here — must be no_std compatible!
```

> [!CAUTION]
> Only add crates that are `no_std` compatible. Standard library crates (`std`, `tokio`, etc.)
> will fail to compile for the bare-metal `riscv64gc-unknown-none-elf` target.

### Step 5 — Add Unit Tests (if any logic can be tested on host)

Kernel code that doesn't do MMIO can be tested with `cargo test --target x86_64-unknown-linux-gnu`:

```rust
// kernel/src/my_module/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_logic() {
        // Pure logic only — no MMIO, no CSR reads
        assert_eq!(2 + 2, 4);
    }
}
```

### Module Init Order Guidelines

The boot sequence in `kmain()` must follow this dependency order:

```
1.  uart::init()             ← Must be first (needed for println!)
2.  trap::init()             ← Must be before interrupts are enabled
3.  memory::init()           ← Must be before any dynamic allocation (buddy + heap)
4.  smp::init()              ← Wake secondary harts 1–3 via SBI HSM
5.  capability system        ← Depends on memory
6.  process/thread           ← Depends on memory + capability; per-hart scheduler
7.  nes::init()              ← Depends on memory + capability
8.  dkcp::init()             ← Depends on memory; starts Raft state machine
9.  enclave_bridge::init()   ← Registers SBI extension EID with M-mode monitor
10. virtio::blk::init()      ← Depends on memory (DMA buffers)
11. virtio::net::init()      ← Depends on memory; optional for multi-VM networking
12. fs::RamFs::load()        ← Depends on virtio::blk
13. process::spawn()         ← Depends on fs + memory + capability
14. thread::schedule()       ← Must be last (yields control to threads on all harts)
```

---

## 8. Testing & Debugging Workflow

### Workflow Overview

```
Edit code
    │
    ▼
make build          ← Catches compile errors early
    │
    ▼
make run            ← Boot in QEMU, observe UART output
    │
    ▼
Analyze output      ← Look for [ERROR], panics, wrong values
    │
    ▼
make debug          ← Attach GDB for step-through debugging
    │
    ▼
make clippy         ← Fix all Clippy warnings before committing
    │
    ▼
make fmt            ← Ensure formatting passes
```

### Using GDB for Kernel Debugging

**Terminal 1** — Launch QEMU with GDB stub:
```bash
make debug
# QEMU starts, halts at the first instruction, waits for GDB on port 1234
```

**Terminal 2** — Connect GDB:
```bash
riscv64-unknown-elf-gdb target/riscv64gc-unknown-none-elf/release/veridian-kernel
(gdb) target remote :1234
(gdb) break kmain
(gdb) continue
(gdb) layout src           # Show source code
(gdb) info registers       # Dump all registers
(gdb) x/20i $pc            # Disassemble 20 instructions at PC
(gdb) p/x *((usize*)0x80200000)   # Read physical memory
```

> [!TIP]
> GDB with the RISC-V ELF has full DWARF debug info in debug builds.
> For release builds, add `debug = true` to `[profile.release]` in `Cargo.toml` temporarily.

### Useful GDB Commands for Kernel Work

| GDB Command | Purpose |
|-------------|---------|
| `info registers` | Show all 32 RISC-V GPRs + PC |
| `p $satp` | Show current page table root (Sv39 format) |
| `p $scause` | Show trap cause register |
| `p $sepc` | Show exception program counter |
| `p $sstatus` | Show supervisor status register |
| `x/4gx 0x80200000` | Examine 4 quad-words of physical memory |
| `break trap_handler` | Break on every trap/exception |
| `break sys_graph_submit` | Break on a specific syscall |
| `watch *(usize*)0x90000000` | Watchpoint on NES doorbell register |

### Reading QEMU Serial Output

All kernel `println!` output goes to the UART (serial port). In QEMU with `-nographic`, it appears directly in your terminal. To save it to a file:

```bash
qemu-system-riscv64 \
    -machine virt -nographic \
    -smp 4 \
    -bios default \
    -drive id=hd0,file=disk.img,format=raw,if=none \
    -device virtio-blk-device,drive=hd0 \
    -kernel target/riscv64gc-unknown-none-elf/release/veridian-kernel \
    2>&1 | tee kernel_boot.log
```

### Debugging a Kernel Panic

When the kernel panics, you'll see:

```
[PANIC] kernel/src/nes/graph.rs:42: assertion `left == right` failed
  left: 5
  right: 3
```

The panic handler (`kernel/src/panic.rs`) prints the file, line, and message, then enters a `loop { wfi }` to halt safely.

To get a stack backtrace, attach GDB after the panic:
```
(gdb) break rust_begin_unwind
(gdb) continue
(gdb) bt   # Print backtrace
```

### Checking Memory Safety

```bash
# Check for potential issues with Clippy
make clippy

# Look for unsafe blocks that might need review:
grep -rn "unsafe" kernel/src/ --include="*.rs" | grep -v "//.*unsafe"
```

---

## 9. Common Gotchas & Fixes

### ❌ "Illegal Instruction" Panic After U-Mode Entry

**Symptom:** After transitioning to U-mode via `sret`, the kernel immediately receives an `Illegal Instruction` exception (scause = 2).

**Common Causes & Fixes:**

| Cause | Diagnosis | Fix |
|-------|-----------|-----|
| ELF loaded but entry point is wrong | Print `sepc` value in trap handler; check against ELF header | Verify ELF e_entry matches the address where `_start` is linked |
| User stack not mapped | Entry jumps to valid code but immediately accesses unmapped stack | Verify `0x40002000` is mapped with `USER | RW` flags |
| `sret` target is in kernel space | `sepc` points to `0x80xxxxxx` | ELF binary must be linked to user-space addresses (e.g., `0x400000`) |
| Permissions wrong on ELF .text | Page mapped with RW but not X | Check page flags: `.text` needs `READ | EXECUTE | USER` |

**Debug Steps:**
```bash
# 1. In trap_handler(), print scause and sepc on every trap
# Add temporarily:
println!("[TRAP] scause=0x{:X} sepc=0x{:X}", scause, sepc);

# 2. Check the ELF entry point
riscv64-unknown-elf-readelf -h target/riscv64gc-unknown-none-elf/release/neural_test
# Look for: Entry point address: 0x...

# 3. Verify the linker script puts code at user-space address
cat user_programs/neural_test/linker.ld  # Should show ORIGIN = 0x400000 or similar
```

### ❌ `make disk` Fails — "tar: command not found"

```bash
# macOS fix (BSD tar is fine, but check PATH)
which tar   # Should be /usr/bin/tar

# On some systems, install GNU tar
brew install gnu-tar
# Then update Makefile: use `gtar` instead of `tar`
```

### ❌ QEMU Not Found

```bash
which qemu-system-riscv64
# If empty:
brew install qemu           # macOS
sudo apt install qemu-system-misc  # Ubuntu
```

### ❌ Linking Error: "undefined reference to `_free_mem_start`"

This linker symbol is defined in `kernel/src/linker.ld`. If you've modified the linker script:

```bash
# Verify the symbol is exported:
grep "_free_mem_start" kernel/src/linker.ld
# Must appear as: _free_mem_start = .;
```

### ❌ VirtIO Block Device Not Found

```
[VIRTIO] Block device not available: No VirtIO block device found
[VIRTIO] Falling back to legacy include_bytes! ELF loader.
```

**Cause:** `disk.img` is missing or QEMU wasn't passed the `-device virtio-blk-device` flag.

```bash
# Rebuild disk image
make disk

# Verify it was created
ls -lh disk.img
tar tf disk.img    # Should list: hello, neural_test
```

### ❌ Rust Nightly Build Fails

```bash
# Update the pinned nightly version
rustup update nightly

# If a specific feature is unstable, check the rust-toolchain.toml date
# You may need to pin to a specific nightly date:
# channel = "nightly-2024-12-01"
```

### ❌ Page Fault / Access Violation in Kernel

If the kernel itself takes a store/load fault, the most common causes are:
1. **Dereferencing a raw pointer before paging is enabled** — `memory::init()` must be called first
2. **Static mut without a lock** — Always use `spin::Mutex` for global mutable state
3. **Stack overflow** — Kernel threads have 16KB stacks; deep recursion will overflow silently

```bash
# Check stack usage:
# In GDB, after a fault:
(gdb) p $sp   # Current stack pointer
# Compare with the thread's stack_bottom field
```

### ❌ `GRAPH_POOL is full` — NES Syscall Returns -ENOMEM

The NES maintains a static pool of 16 `TaskGraph` objects. If all 16 are in use (including leaked ones), allocation fails.

**Fix:** Always close graph handles when done:

```rust
// User space — after waiting for completion:
syscall_graph_wait(graph_handle, usize::MAX);
syscall_handle_close(graph_handle);  // ← Don't forget this!
```

---

## 10. Contribution Checklist

Before opening a pull request, verify:

```
Documentation
  [ ] New functions/types have doc comments (/// ...)
  [ ] Syscalls documented in syscall/numbers.rs
  [ ] plan.md updated if architecture changes

Code Quality
  [ ] `make clippy` passes with zero warnings
  [ ] `make fmt` passes (no formatting changes)
  [ ] No new `#[allow(dead_code)]` or `#[allow(unused)]` without justification
  [ ] No `unwrap()` in kernel code — use `expect("reason")` or handle the error

Safety
  [ ] All `unsafe` blocks have a // SAFETY: comment explaining why it's safe
  [ ] New syscalls validate all user-space pointers:
      - Non-null
      - Below 0x80000000 (kernel space boundary)
      - Properly aligned for the type
  [ ] New syscalls check handle type AND rights before use

Testing
  [ ] `make run` boots successfully to "[SUCCESS] VeridianOS Phase N fully verified!"
  [ ] New syscall is exercised by at least one user-space call
  [ ] No regressions in existing UART output
```

---

<br>

<div align="center">

**VeridianOS** — *The AI-Native Operating System*

*Built with 🦀 Rust • Targeting ⚡ RISC-V • Secured by 🔐 Capabilities*

*Questions? Open an issue or check `plan.md` for the full technical reference.*

</div>
