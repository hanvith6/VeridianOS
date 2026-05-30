# VeridianOS — System Design Reference

> **Single source of truth** for VeridianOS architecture, implementation, research foundations, and developer onboarding.  
> This document covers everything from first boot through Phase 12 TEE isolation.

---

## Table of Contents

1. [Project Vision](#1-project-vision)
2. [Architecture Overview](#2-architecture-overview)
3. [Privilege Level Layout](#3-privilege-level-layout)
4. [Phase Implementation Reference](#4-phase-implementation-reference)
5. [Capability System](#5-capability-system)
6. [Memory Management](#6-memory-management)
7. [Thread Scheduler](#7-thread-scheduler)
8. [Neural Execution Subsystem (NES)](#8-neural-execution-subsystem-nes)
9. [Semantic Graph Filesystem (SGF)](#9-semantic-graph-filesystem-sgf)
10. [Agent Runtime](#10-agent-runtime)
11. [Self-Improving Policies](#11-self-improving-policies)
12. [Distributed Multi-Kernel Coherence](#12-distributed-multi-kernel-coherence)
13. [SMP and Exception Delivery](#13-smp-and-exception-delivery)
14. [Phase 12 — M-Mode TEE Security Monitor](#14-phase-12--m-mode-tee-security-monitor)
15. [Syscall Reference](#15-syscall-reference)
16. [Developer Onboarding](#16-developer-onboarding)
17. [Testing and QA Strategy](#17-testing-and-qa-strategy)
18. [Research Foundations](#18-research-foundations)
19. [Version Control and Branch Strategy](#19-version-control-and-branch-strategy)
20. [Future Roadmap](#20-future-roadmap)

---

## 1. Project Vision

VeridianOS is a clean-slate, open-source microkernel operating system written from scratch in **Rust** for the **RISC-V 64-bit** architecture. It addresses the limitations of operating system paradigms designed for sequential, single-CPU, von-Neumann machines — paradigms that predate the AI and distributed computing era by four decades.

The central hypothesis is that **AI agents, heterogeneous accelerators, and distributed capability sharing should be primary kernel abstractions**, not user-space afterthoughts bolted onto POSIX.

### Design Principles

| Principle | Rationale |
|-----------|-----------|
| **Capability-based security** | No ambient authority, no `root`. Every resource access requires a kernel-managed unforgeable token. Inspired by seL4 and Fuchsia/Zircon. |
| **Accelerator-first scheduling** | GPUs, NPUs, and specialized cores are primary scheduler targets, not I/O devices. Inspired by LithOS (SOSP '25). |
| **Semantic data model** | Data is a graph of typed entities and labeled relationships, not a flat hierarchy of byte sequences. |
| **Self-improving policies** | The scheduler observes real hardware latencies and updates its routing decisions using online exponential moving averages. |
| **Hardware TEE isolation** | AI agents running sensitive workloads can request M-mode-enforced memory isolation that even a compromised kernel cannot bypass. |
| **Minimal, safe TCB** | The Trusted Computing Base is a small S-mode kernel with `unsafe` Rust restricted to memory-mapped I/O and assembly. |

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              USER SPACE (U-mode)                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌───────────┐  │
│  │  AI Agent A  │  │  AI Agent B  │  │  VirtIO drv  │  │ enclave_  │  │
│  │  (ordinary)  │  │  (enclave)   │  │  (U-mode)    │  │ payload   │  │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └─────┬─────┘  │
│         │  ecall (a7=syscall#)               │                │        │
├─────────┼───────────────────────────────────┼────────────────┼─────────┤
│                        SUPERVISOR MODE (S-mode)                         │
│   ┌─────▼────────────────────────────────────────────────────▼──────┐  │
│   │                         VERIDIAN KERNEL                          │  │
│   │  capability/   memory/    process/    nes/      semantic_graph/  │  │
│   │  syscall/      trap.rs    sbi.rs      agent/    dist/    enclave/│  │
│   └─────────────────────────┬─────────────────────────────────────── ┘  │
│                             │  ecall (a7=EID, M-mode SBI)               │
├─────────────────────────────┼───────────────────────────────────────────┤
│                      MACHINE MODE (M-mode)                              │
│   ┌─────────────────────────▼─────────────────────────────────────────┐ │
│   │          OpenSBI + VeridianOS M-Mode TEE Monitor (monitor/)       │ │
│   │  pmp.rs    enclave.rs    attest.rs    sbi_handler.rs    main.rs   │ │
│   └───────────────────────────────────────────────────────────────────┘ │
├─────────────────────────────────────────────────────────────────────────┤
│                        RISC-V HARDWARE                                   │
│  PMP registers  │  CSRs (mtvec, stvec, satp, time)  │  CLINT  │  PLIC   │
└─────────────────────────────────────────────────────────────────────────┘
```

### Codebase Structure

```
VeridianOS/
├── kernel/src/
│   ├── main.rs            — Kernel entry, subsystem init sequence
│   ├── arch/riscv64/      — boot.S, trap.S, RISC-V-specific code
│   ├── capability/        — Handle, HandleTable, rights attenuation
│   ├── memory/            — Buddy page allocator, Sv39 page tables, VMO
│   ├── process/           — Process, Thread, ELF loader, stack setup
│   ├── syscall/           — Dispatcher (trap.rs), all syscall handlers, numbers.rs
│   ├── nes/               — Neural Execution Subsystem: queues, DAG, router
│   ├── semantic_graph/    — Semantic Graph Filesystem nodes and edges
│   ├── agent/             — AgentRecord, AgentChannel, agent lifecycle
│   ├── dist/              — DKCP rings, ClusterState, DCTP, Raft engine
│   ├── enclave/           — S-mode bridge to M-mode TEE monitor
│   ├── fs/                — VirtIO block, ustar TAR InitRAMFS
│   ├── sbi.rs             — SBI ecall wrappers (timer, console, HSM)
│   ├── trap.rs            — Trap delegation, exception handler routing
│   └── uart.rs            — 16550 UART MMIO driver
├── monitor/src/
│   ├── main.rs            — M-mode entry, secondary hart parking, SBI dispatch
│   ├── pmp.rs             — PMP NAPOT encoding, lock/unlock/grant
│   ├── enclave.rs         — EnclaveState machine, static 8-slot pool
│   ├── attest.rs          — SHA-256 (FIPS 180-4), HMAC-SHA-256, 73-byte report
│   └── sbi_handler.rs     — EID 0x08424B45 ("BKE") dispatcher
├── user_programs/
│   ├── hello/             — Basic ELF + syscall smoke test
│   ├── neural_test/       — NES queue submission and DAG execution
│   ├── semantic_test/     — SGF node/edge CRUD and traversal
│   ├── agent_test/        — Agent spawn, channel send/recv
│   ├── policy_test/       — Phase 10/11 syscalls 80–101
│   ├── smp_test/          — Multi-hart + user-space exception delivery
│   └── enclave_test/      — Phase 12: create, enter/exit, attestation
├── docs/                  — This file and supporting assets
├── Makefile               — Build targets: build, run, disk, build_monitor
└── rust-toolchain.toml    — Pins nightly + riscv64gc-unknown-none-elf target
```

---

## 3. Privilege Level Layout

RISC-V defines three privilege levels. VeridianOS uses all three:

| Mode | Name | Who Runs Here | Key CSRs |
|------|------|---------------|----------|
| M-mode | Machine | TEE Monitor (`monitor/` crate) | `mtvec`, `mepc`, `mstatus`, `pmpcfg0-3`, `pmpaddr0-15` |
| S-mode | Supervisor | VeridianOS Kernel (`kernel/` crate) | `stvec`, `sepc`, `sstatus`, `satp`, `sip`, `sie` |
| U-mode | User | All user processes, enclave payloads | No privileged CSR access |

### Trap Delegation

RISC-V M-mode traps everything by default. The monitor delegates standard traps to S-mode via `medeleg` and `mideleg`, except:
- `ecall` from S-mode with EID `0x08424B45` — intercepted by the monitor's SBI handler
- Machine-mode exceptions — handled in M-mode and never delegated

---

## 4. Phase Implementation Reference

All 13 phases (0 through 12) are complete. This table shows what each phase added and how to verify it.

| Phase | Name | Status | Verification | Key Files |
|-------|------|--------|--------------|-----------|
| 1 | Bootable RISC-V Microkernel | ✅ | UART output on `make run` | `boot.S`, `uart.rs`, `main.rs` |
| 2 | Capability System | ✅ | Integrated | `capability/mod.rs` |
| 3 | Page Allocator & Sv39 VM | ✅ | `hello` runs | `memory/buddy.rs`, `memory/page_table.rs` |
| 4 | Preemptive Thread Scheduler | ✅ | `hello` runs | `process/scheduler.rs` |
| 5 | VirtIO Block + InitRAMFS | ✅ | `hello` loaded from disk | `fs/virtio_blk.rs`, `fs/initramfs.rs` |
| 6 | ELF Loader + U-mode | ✅ | `hello` executes in U-mode | `process/mod.rs` (load_elf) |
| 7 | Neural Execution Subsystem | ✅ | `neural_test` | `nes/mod.rs`, `nes/queue.rs` |
| 8 | Semantic Graph Filesystem | ✅ | `semantic_test` | `semantic_graph/mod.rs` |
| 9 | Agent Runtime | ✅ | `agent_test` | `agent/mod.rs` |
| 10 | Self-Improving Policies | ✅ | `policy_test` | `nes/router.rs` |
| 11 | Distributed Coherence | ✅ | `policy_test` (syscalls 90–101) | `dist/` |
| 11.5 | SMP + Exception Delivery | ✅ | `smp_test` | `arch/riscv64/smp.rs`, `process/exception.rs` |
| 12 | M-Mode TEE Monitor | ✅ | `enclave_test` | `monitor/src/`, `kernel/src/enclave/` |

### Phase 1 — Bootable Microkernel

Entry point at `0x8020_0000` (after OpenSBI). Assembly stub in `boot.S` sets up per-hart stack and calls `_start_rust`. The Rust entry initializes UART, the memory allocator, and the capability system before spinning the scheduler loop.

Key decisions:
- **`#![no_std]` + `#![no_main]`** — no standard library, no OS runtime. All OS primitives built from scratch.
- **Linker script** (`link.ld`) places `.text.boot` at the physical entry address and `.bss` after `.data` for zero-initialization.
- **UART at `0x1000_0000`** — the QEMU `virt` machine's 16550-compatible UART address.

### Phase 2 — Capability System

Every kernel resource is represented as a `KernelObject`. Processes access objects via `Handle` tokens stored in per-process `HandleTable`. Handles carry a rights bitmask (`Rights`) that can only be attenuated (reduced), never amplified.

```rust
// kernel/src/capability/mod.rs
pub struct Handle {
    pub object: Arc<dyn KernelObject>,
    pub rights: Rights,
}
bitflags! {
    pub struct Rights: u32 {
        const READ    = 1 << 0;
        const WRITE   = 1 << 1;
        const EXECUTE = 1 << 2;
        const TRANSFER= 1 << 3;
    }
}
```

This eliminates ambient authority: there is no `root`, no global filesystem permission check, no UID comparison. If a process does not hold a handle to a resource, it cannot access it — period.

### Phase 3 — Physical Memory and Virtual Address Spaces

**Buddy Allocator** (`memory/buddy.rs`): Manages physical page frames using a power-of-two free list. Allocation and deallocation are O(log N).

**Sv39 Page Tables** (`memory/page_table.rs`): Three-level page tables (39-bit virtual address space) following the RISC-V Sv39 specification. Each user process gets its own root page table. The kernel maps its own image identically in all address spaces to simplify trap handling.

**Virtual Memory Objects (VMOs)**: Physical memory regions are wrapped as `VmoObject` kernel objects and shared across processes via capability handles. A process cannot access another's memory without a `READ`-capable handle to its VMO.

### Phase 4 — Thread Scheduler

Preemptive round-robin scheduling. The RISC-V timer (CLINT, via `SBI_SET_TIMER`) fires a supervisor timer interrupt every ~10ms. The trap handler saves the full register state (`TrapFrame`) and picks the next runnable thread.

Thread lifecycle: `Ready → Running → Blocked → Ready`. Blocking is explicit (e.g., waiting on a channel message); the scheduler never blocks a thread except by explicit syscall.

### Phase 5 — VirtIO Block and InitRAMFS

**VirtIO block driver** (`fs/virtio_blk.rs`): Communicates with the QEMU virtio-blk device through a virtqueue — shared-memory descriptor ring with `avail` and `used` rings. No interrupts in the current implementation; polling-mode I/O.

**InitRAMFS** (`fs/initramfs.rs`): On boot, the kernel reads `disk.img` (a POSIX ustar TAR archive, built by `make disk`) and extracts all embedded ELF binaries into memory. Each binary becomes available for spawning by name.

### Phase 6 — ELF Loader and User-Mode Execution

`process::load_elf` parses ELF64 headers, maps PT_LOAD segments into the process's Sv39 address space, allocates a 64 KiB user stack, and transfers control via `sret` to the ELF entry point in U-mode.

```
Physical memory layout for a loaded process:
  [ELF segments]   ← PT_LOAD segments mapped at virtual addresses
  [user stack]     ← 64 KiB at 0x7FFF_0000
  [syscall args]   ← argc/argv pushed on stack before entry
```

User processes issue syscalls via `ecall` with `a7 = syscall_number`, `a0-a5 = arguments`. The trap handler in S-mode dispatches to the appropriate handler.

---

## 5. Capability System

### Handle Table

Each `Process` owns a `HandleTable: BTreeMap<u32, Handle>`. The kernel allocates handle indices starting at 1; index 0 is always invalid (NULL sentinel). Handles are never directly accessible from user space — user code uses integer indices that the kernel resolves to actual object references on each syscall.

### Rights Attenuation

When a process transfers a capability to another (via `SYS_CHANNEL_SEND_HANDLE`), it may specify a subset of its own rights. The kernel enforces:
```
transferred_rights = held_rights & requested_rights
```
Amplification is impossible: you cannot grant a right you do not hold.

### Object Types

| Object Type | Description |
|-------------|-------------|
| `ThreadObject` | Schedulable thread with register state |
| `ProcessObject` | Address space + handle table + thread set |
| `VmoObject` | Physical memory region |
| `ChannelObject` | Bidirectional IPC message queue |
| `AgentObject` | AI agent with mailbox and lifecycle state |
| `EnclaveObject` | (Phase 12) TEE enclave slot reference |

---

## 6. Memory Management

### Physical Allocator

The buddy allocator partitions the physical memory range `[0x8040_0000, 0x9000_0000)` into 4 KiB pages. Free lists for orders 0 through 10 (4 KiB through 4 MiB blocks). The kernel heap (`linked_list_allocator`) sits atop this, managed by a global allocator.

### Sv39 Address Space

```
Virtual Address Space (39-bit):
  0x0000_0000_0000 – 0x0000_003F_FFFF  →  User space (256 GiB)
  0xFFFF_FFC0_0000 – 0xFFFF_FFFF_FFFF  →  Kernel (direct-mapped, 1 GiB)
```

`satp` CSR holds the root PPN and ASID. Context switches update `satp` and flush the TLB with `sfence.vma`.

### Memory Safety

All kernel memory management uses Rust's ownership system. The `unsafe` surface is restricted to:
- Raw pointer reads/writes for MMIO registers
- Assembly for CSR access and context switch
- Atomic operations on shared scheduler state

---

## 7. Thread Scheduler

### Context Switch

`TrapFrame` stores 31 general-purpose registers + `sepc` + `sstatus`. On timer interrupt:
1. Save current thread's `TrapFrame` to its `Thread.context` field
2. Pick next `Ready` thread from the run queue
3. Restore its `TrapFrame`
4. Update `satp` if switching processes (TLB flush)
5. `sret` to resume

### Scheduler State Machine

```
                 spawn()
    ─────────────────────────────► Ready
                                     │
                              timer  │  schedule()
                            interrupt│
                                     ▼
                                  Running
                                  │     │
                    syscall_block()│     │ thread_exit()
                                  ▼     ▼
                                Blocked  Dead
                                  │
                        unblock() │
                                  └──────────► Ready
```

---

## 8. Neural Execution Subsystem (NES)

### Motivation

Traditional kernels dispatch all computation to CPUs. AI workloads run across CPUs, GPUs, and NPUs. NES makes the kernel aware of heterogeneous hardware queues and dispatches task graph nodes to the fastest available device.

### Device Queues

NES maintains three `DeviceQueue` instances:
- `CPU_QUEUE` — ordinary RISC-V cores
- `GPU_QUEUE` — placeholder for OpenCL/CUDA-equivalent accelerator
- `NPU_QUEUE` — neural processing unit (inference accelerator)

Each queue holds a fixed-size circular buffer of `NesNode` (task graph node) entries.

### Task Graph (DAG) Execution

User programs submit `NesNode` items via `SYS_NES_SUBMIT`. Each node has:
- `device_hint: DeviceType` — preferred hardware
- `deps: [u64; 4]` — IDs of prerequisite nodes (0 = no dep)
- `payload: [u64; 8]` — opaque data for the device driver

The scheduler dispatches a node only when all its deps are `Completed`. This implements Directed Acyclic Graph execution natively in the kernel.

### Syscalls (Phase 7)

| Number | Name | Arguments |
|--------|------|-----------|
| 60 | `SYS_NES_SUBMIT` | `node_ptr`, `node_size` → `node_id` |
| 61 | `SYS_NES_WAIT` | `node_id` → 0 when complete |
| 62 | `SYS_NES_QUERY` | `node_id` → status code |

---

## 9. Semantic Graph Filesystem (SGF)

### Motivation

Hierarchical filesystems (directories, filenames) are an artifact of slow rotating disks. VeridianOS replaces them with a graph database where data is structured by **meaning and relationship**, not path.

### Data Model

```
Node { id: u64, node_type: NodeType, data: [u8; 128] }
Edge { from: u64, to: u64, relation: EdgeRelation }
```

`NodeType` variants: `Binary`, `DataBlob`, `AgentState`, `ServiceEndpoint`, `ConfigRecord`

`EdgeRelation` variants: `DependsOn`, `ExecutesWith`, `SecuredBy`, `Stores`, `Replicates`

### Graph Operations (Syscalls 70–79)

| Number | Name | Description |
|--------|------|-------------|
| 70 | `SYS_SGF_NODE_CREATE` | Insert a new typed node |
| 71 | `SYS_SGF_NODE_GET` | Retrieve node data by ID |
| 72 | `SYS_SGF_EDGE_CREATE` | Connect two nodes with a relation |
| 73 | `SYS_SGF_EDGE_QUERY` | Find neighbors by relation type |
| 74 | `SYS_SGF_NODE_DELETE` | Remove a node and its edges |

### Access Control

Every SGF node is a `KernelObject`. Processes access nodes through capability handles, inheriting the same rights model as all other kernel objects.

---

## 10. Agent Runtime

### Agent as First-Class Kernel Object

An `AgentRecord` captures:
```rust
pub struct AgentRecord {
    pub id: AgentId,
    pub state: AgentState,          // Created, Running, Suspended, Dead
    pub mailbox: VecDeque<Message>,
    pub capability_budget: u32,
    pub enclave_id: Option<u8>,     // Phase 12: Some(id) = TEE-isolated
}
```

### Agent Lifecycle (Syscalls 80–89)

| Number | Name | Description |
|--------|------|-------------|
| 80 | `SYS_AGENT_SPAWN` | Create a new AgentRecord, return agent_id |
| 81 | `SYS_AGENT_SEND` | Post a `Message` to an agent's mailbox |
| 82 | `SYS_AGENT_RECV` | Dequeue next message (blocks if empty) |
| 83 | `SYS_AGENT_STATUS` | Query current `AgentState` |
| 84 | `SYS_AGENT_KILL` | Terminate an agent |

### Channels

Agents communicate via `ChannelObject` (bidirectional FIFO). `SYS_CHANNEL_SEND` and `SYS_CHANNEL_RECV` transfer `Message` payloads. Capabilities (handles) can be sent through channels, enabling dynamic delegation of resources between agents.

---

## 11. Self-Improving Policies

### Epsilon-Greedy Device Router

The NES router tracks per-device execution latency using **Exponential Moving Averages (EMA)**:

```
EMA_new = α × latest_sample + (1-α) × EMA_old     (α = 0.2)
```

Latency samples are taken from the RISC-V `rdtime` CSR before and after node execution.

**Epsilon-greedy policy** (ε = 0.1):
- With probability 1-ε: route to the device with lowest EMA latency (exploit)
- With probability ε: route to a random device (explore)

Over time the policy converges to the empirically fastest device for each workload type without any static configuration.

### Phase 10 Syscalls (90–101)

| Number | Name | Description |
|--------|------|-------------|
| 90 | `SYS_DIST_DOMAIN_JOIN` | Register with the cluster |
| 91 | `SYS_DIST_DOMAIN_LIST` | List known peer domains |
| 92 | `SYS_DIST_DOMAIN_STATUS` | Query domain liveness |
| 93 | `SYS_DIST_NES_DISPATCH` | Dispatch a NES node to a remote domain |
| 94 | `SYS_DIST_NES_WAIT` | Wait for remote NES node completion |
| 95 | `SYS_DIST_NES_ABORT` | Cancel in-flight remote dispatch |
| 96 | `SYS_DIST_CAP_EXPORT` | Export a local capability as a 128-bit UID |
| 97 | `SYS_DIST_CAP_IMPORT` | Install a remote capability into local handle table |
| 98 | `SYS_DIST_CAP_REVOKE` | Broadcast capability revocation |
| 99 | `SYS_DIST_SGF_REPLICATE` | Enable graph replication for a node |
| 100 | `SYS_DIST_SGF_QUERY` | Query replicated graph state |
| 101 | `SYS_DIST_RAFT_STATUS` | Query Raft consensus state |

---

## 12. Distributed Multi-Kernel Coherence

### DKCP Transport (Lock-Free SPSC Ring)

`DkcpRing`: 256-slot circular buffer, 64 bytes per slot (one cache line). Lock-free enqueue and dequeue using atomic `write_volatile`/`read_volatile` with sequential consistency. No mutex, no kernel lock on the hot path.

Message types: `Hello`, `Heartbeat`, `CapExport`, `CapImport`, `CapRevokeNotify`, `GraphNodeDispatch`, `GraphNodeResult`, `AppendEntries`, `AppendEntriesReply`, `RequestVote`, `RequestVoteReply`.

### Cluster Membership

`ClusterState` tracks up to 8 `KernelDomainId` peers. Each domain has a liveness epoch counter. After 5 missed heartbeat ticks, a domain transitions to `Dead`. `Hello` and `Heartbeat` messages refresh liveness on arrival.

### DCTP — Distributed Capability Transfer

Cap export derives a 128-bit UID from `rdtime` + `domain_id` + monotonic counter. UIDs are globally unique within the cluster. On import, the kernel installs a shadow `Handle` into the remote process's handle table. Revocation propagates via `CapRevokeNotify` broadcast — all domains with a shadow copy of the revoked handle invalidate it atomically.

### Raft Consensus

Full Raft state machine: `Follower → Candidate → Leader`. On a single QEMU instance, hart 0 wins quorum immediately (it is the only voter). `append_entry` commits `SemanticGraphMutation` records to a 128-slot static log and replicates via `AppendEntries` messages on the DKCP ring.

---

## 13. SMP and Exception Delivery

### Secondary Hart Bringup

During early boot, hart 0 starts harts 1–3 via `SBI_HART_START` (HSM extension). Each secondary hart:
1. Initializes its own supervisor-mode trap vector
2. Enables supervisor timer interrupts
3. Enters an independent scheduling loop

There is no global kernel lock. Harts pick threads from a shared `AtomicPtr<Thread>` ready queue using compare-and-swap.

### User-Space Exception Delivery

Processes register a fault handler via `SYS_REGISTER_EXCEPTION_HANDLER(entry_va, stack_va)`. On a synchronous exception (page fault, illegal instruction, misaligned access), instead of terminating the process:
1. The kernel saves the full `TrapFrame` to the user stack at `stack_va`
2. Sets `sepc` to `entry_va`
3. `sret`s into the handler

The handler can inspect the saved frame, attempt recovery, and resume via a resume syscall. This is the foundation for user-space signal-like semantics.

---

## 14. Phase 12 — M-Mode TEE Security Monitor

### Motivation

AI agents processing sensitive data (private keys, personal user data, proprietary model weights) need isolation that even a compromised S-mode kernel cannot break. RISC-V Physical Memory Protection (PMP) registers are accessible only from M-mode — making an M-mode monitor the hardware-enforced root of trust.

### System Architecture

```
U-mode:   enclave_test (SYS_ENCLAVE_CREATE → SYS_ENCLAVE_ATTEST)
              │  ecall a7=120-123
S-mode:   kernel/src/enclave/mod.rs  (validates args, forwards SBI)
              │  ecall a7=0x08424B45, a6=FID
M-mode:   monitor/src/sbi_handler.rs → pmp.rs + enclave.rs + attest.rs
              │  csrw pmpaddr0..7, pmpcfg0/2
Hardware: PMP registers (S-mode cannot read or write these)
```

### Monitor Binary (`monitor/` crate)

A separate `no_std` binary targeting `riscv64gc-unknown-none-elf`. Loaded by QEMU as the M-mode firmware, before OpenSBI. M-mode entry at `_start` (assembly in `global_asm!`):

```asm
_start:
    la sp, _stack_top
    li t0, 8192            # 8 KiB per hart
    mul t0, a0, t0
    sub sp, sp, t0
    tail monitor_main
```

`monitor_main` (hart 0):
1. Calls `pmp::lock_monitor_self()` — PMP entry 15, NAPOT, Lock bit set. Monitor image is now immutable.
2. Configures `mtvec` to point to the trap handler
3. Sets `medeleg`/`mideleg` to delegate standard exceptions to S-mode
4. Sets `mstatus.MPP = S-mode`, `mepc = kernel_entry`
5. `mret` — drops to S-mode and starts the kernel

### PMP Configuration (`pmp.rs`)

RISC-V PMP protects physical memory ranges from lower-privilege modes. NAPOT (Naturally Aligned Power Of Two) is the most efficient mode:

```
NAPOT encoding: pmpaddr = (base >> 2) | ((size / 8) - 1)
Example: 64 KiB at 0x8800_0000 → pmpaddr = 0x2200_1FFF
```

PMP entry assignment:
- Entries 0–7: enclave slots (dynamically assigned as enclaves are created)
- Entry 15: monitor self-protection (locked at boot, immutable)

API:
- `lock_region(slot, phys_start, size)` — deny S-mode/U-mode access (no R/W/X, NAPOT)
- `grant_region(slot, phys_start, size)` — allow full access (used while loading enclave binary)
- `unlock_region(slot)` — disable entry (A=OFF) on enclave exit
- `lock_monitor_self()` — called once at boot, sets L=1 on entry 15

### Enclave Lifecycle (`enclave.rs`)

Static pool of 8 `Enclave` slots (no heap allocation in monitor):

```
EnclaveState::Empty   → allocation pool entry is free
EnclaveState::Created → SBI_ENCLAVE_CREATE: PMP granted, measurement computed
EnclaveState::Running → SBI_ENCLAVE_ENTER: PMP locked (S-mode access revoked)
EnclaveState::Exited  → SBI_ENCLAVE_EXIT:  PMP unlocked, slot returned to pool
```

Transition on `ENCLAVE_CREATE`:
1. Find a free slot in the static pool
2. Call `pmp::grant_region(slot, phys_start, size)` — kernel can load the binary
3. Compute SHA-256 measurement over `[phys_start, phys_start+size)` bytes
4. Store measurement in `enclave.measurement`
5. Return `enclave_id` (0–7) to S-mode

Transition on `ENCLAVE_ENTER`:
1. Call `pmp::lock_region(slot, ...)` — removes S-mode access
2. Save current S-mode CPU context (so kernel can be resumed after exit)
3. Set `mepc = enclave.entry_pa`, `mstatus.MPP = U-mode`
4. `mret` — CPU drops to U-mode and begins executing enclave code

### Attestation (`attest.rs`)

The attestation report proves to a remote verifier what code is running inside an enclave.

**SHA-256 measurement**: computed over the entire enclave memory region at creation time. Implemented from FIPS 180-4 with no external crates (no_std constraint).

**HMAC-SHA-256**: signs the measurement with a device key. The 32-byte device key is a compile-time constant in the scaffold; production deployment replaces it with a key read from OTP/eFuse.

**Report layout (73 bytes)**:

```
Offset  Len  Field
──────  ───  ─────────────────────────────────────
0       1    enclave_id (u8)
1       8    phys_start (u64, little-endian)
9       8    size (u64, little-endian)
17      32   SHA-256 measurement
49      24   HMAC-SHA-256 tag (first 24 bytes of full 32-byte HMAC)
```

The report is written to a user-space buffer via `SYS_ENCLAVE_ATTEST`. A remote verifier with the device public key can independently compute the HMAC and compare.

### SBI Extension (EID `0x08424B45`)

ASCII encoding of "BKE" (VeridianOS Breakout Keystone Extension):

| FID | Function | Arguments | Returns |
|-----|----------|-----------|---------|
| 0 | `ENCLAVE_CREATE` | `a0=phys_addr`, `a1=size`, `a2=entry_pa` | `a1=enclave_id` |
| 1 | `ENCLAVE_ENTER` | `a0=enclave_id` | — (returns after enclave exits) |
| 2 | `ENCLAVE_EXIT` | `a0=enclave_id` | — (resumes kernel) |
| 3 | `ENCLAVE_ATTEST` | `a0=enclave_id`, `a1=report_phys` | — (fills 73-byte report) |

SBI error codes: `SBI_SUCCESS=0`, `SBI_ERR_FAILED=-1`, `SBI_ERR_INVALID_PARAM=-3`, `SBI_ERR_DENIED=-4`, `SBI_ERR_ALREADY_AVAILABLE=-6`.

### Kernel Bridge (`kernel/src/enclave/mod.rs`)

Validates user-supplied arguments before forwarding to the monitor:
- `phys_addr` must be page-aligned
- `size` must be a power of two and ≥ 8 bytes
- `report_ptr` must be a valid user-space virtual address
- `enclave_id` must be 0–7

SBI errors from the monitor propagate directly to user space as negative `isize` return values.

### Syscall Interface (Phase 12, numbers 120–123)

| Number | Name | Arguments | Returns |
|--------|------|-----------|---------|
| 120 | `SYS_ENCLAVE_CREATE` | `a0=phys_addr`, `a1=size`, `a2=entry_pa` | enclave_id (0–7) or negative error |
| 121 | `SYS_ENCLAVE_ENTER` | `a0=enclave_id` | 0 on success (after enclave exits) |
| 122 | `SYS_ENCLAVE_EXIT` | `a0=enclave_id` | (issued from inside enclave) |
| 123 | `SYS_ENCLAVE_ATTEST` | `a0=enclave_id`, `a1=report_buf_vaddr` | 0 on success |

### AgentRecord Integration

`AgentRecord.enclave_id: Option<u8>` — when `Some(id)`, the agent's execution context is inside a PMP-isolated enclave. The kernel cannot inspect or modify the agent's working memory; only the agent itself and the M-mode monitor have access.

### Security Properties

**Guaranteed by hardware**:
- Once `lock_region` is called, no S-mode or U-mode instruction can read, write, or execute the enclave region — a PMP violation raises a hardware fault.
- The monitor image itself is locked at entry (PMP entry 15, L=1) — no runtime modification possible without a system reset.
- SHA-256 measurement is computed before S-mode can modify the enclave binary (creation happens before locking).

**Not guaranteed (known limitations)**:
- Side-channel attacks (cache timing, Spectre-PHT) — PMP provides spatial isolation only, not temporal.
- Physical memory attacks (cold boot) — requires encrypted RAM (AMD SME equivalent).
- Device key is a compile-time constant — production requires OTP-fused per-device keys.
- Multi-hart enclave access requires TLB shootdowns across harts.

---

## 15. Syscall Reference

Complete list of all VeridianOS syscalls:

| Number | Name | Phase | Description |
|--------|------|-------|-------------|
| 1 | `SYS_WRITE` | 1 | Write bytes to UART (debug I/O) |
| 2 | `SYS_EXIT` | 1 | Terminate the calling process |
| 10 | `SYS_THREAD_YIELD` | 4 | Voluntarily yield the CPU |
| 20 | `SYS_CHANNEL_CREATE` | 5 | Create a bidirectional IPC channel |
| 21 | `SYS_CHANNEL_SEND` | 5 | Send message on a channel |
| 22 | `SYS_CHANNEL_RECV` | 5 | Receive message from a channel |
| 30 | `SYS_VMO_CREATE` | 3 | Allocate a Virtual Memory Object |
| 31 | `SYS_VMO_MAP` | 3 | Map a VMO into the address space |
| 32 | `SYS_VMO_UNMAP` | 3 | Unmap a VMO |
| 40 | `SYS_PROCESS_SPAWN` | 6 | Spawn a new process from an ELF name |
| 41 | `SYS_PROCESS_EXIT` | 6 | Exit the current process |
| 50 | `SYS_HANDLE_CLOSE` | 2 | Release a capability handle |
| 51 | `SYS_HANDLE_DUP` | 2 | Duplicate a handle with attenuated rights |
| 60 | `SYS_NES_SUBMIT` | 7 | Submit a neural execution node |
| 61 | `SYS_NES_WAIT` | 7 | Wait for node completion |
| 62 | `SYS_NES_QUERY` | 7 | Query node execution status |
| 70 | `SYS_SGF_NODE_CREATE` | 8 | Create a semantic graph node |
| 71 | `SYS_SGF_NODE_GET` | 8 | Retrieve a node by ID |
| 72 | `SYS_SGF_EDGE_CREATE` | 8 | Create a graph edge |
| 73 | `SYS_SGF_EDGE_QUERY` | 8 | Query edges by relation type |
| 74 | `SYS_SGF_NODE_DELETE` | 8 | Delete a node |
| 80 | `SYS_AGENT_SPAWN` | 9 | Spawn a kernel AI agent |
| 81 | `SYS_AGENT_SEND` | 9 | Send message to an agent |
| 82 | `SYS_AGENT_RECV` | 9 | Receive message from agent mailbox |
| 83 | `SYS_AGENT_STATUS` | 9 | Query agent state |
| 84 | `SYS_AGENT_KILL` | 9 | Terminate an agent |
| 90 | `SYS_DIST_DOMAIN_JOIN` | 11 | Join distributed cluster |
| 91 | `SYS_DIST_DOMAIN_LIST` | 11 | List peer domains |
| 92 | `SYS_DIST_DOMAIN_STATUS` | 11 | Query domain liveness |
| 93 | `SYS_DIST_NES_DISPATCH` | 11 | Dispatch NES node to remote domain |
| 94 | `SYS_DIST_NES_WAIT` | 11 | Wait for remote NES result |
| 95 | `SYS_DIST_NES_ABORT` | 11 | Cancel remote NES dispatch |
| 96 | `SYS_DIST_CAP_EXPORT` | 11 | Export capability as 128-bit UID |
| 97 | `SYS_DIST_CAP_IMPORT` | 11 | Import remote capability |
| 98 | `SYS_DIST_CAP_REVOKE` | 11 | Revoke distributed capability |
| 99 | `SYS_DIST_SGF_REPLICATE` | 11 | Enable SGF node replication |
| 100 | `SYS_DIST_SGF_QUERY` | 11 | Query replicated SGF state |
| 101 | `SYS_DIST_RAFT_STATUS` | 11 | Query Raft consensus state |
| 110 | `SYS_REGISTER_EXCEPTION_HANDLER` | 11.5 | Register user-space exception handler |
| 111 | `SYS_EXCEPTION_RESUME` | 11.5 | Resume execution after handling exception |
| 120 | `SYS_ENCLAVE_CREATE` | 12 | Create TEE enclave via M-mode monitor |
| 121 | `SYS_ENCLAVE_ENTER` | 12 | Enter TEE enclave (CPU drops to U-mode in PMP region) |
| 122 | `SYS_ENCLAVE_EXIT` | 12 | Exit enclave, return to kernel |
| 123 | `SYS_ENCLAVE_ATTEST` | 12 | Get hardware attestation report |

---

## 16. Developer Onboarding

### Prerequisites

| Tool | Minimum Version | Purpose | Install |
|------|----------------|---------|---------|
| Rust (rustup) | nightly | `no_std` kernel, inline asm | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| QEMU | 7.0+ | RISC-V 64-bit emulator | `brew install qemu` (macOS) / `apt install qemu-system-misc` |
| GNU Make | 3.81+ | Build orchestration | Pre-installed |
| GDB (optional) | Any | Kernel debugging via QEMU stub | `brew install riscv-software-src/riscv/riscv-gnu-toolchain` |

The project ships `rust-toolchain.toml` — `rustup` auto-installs the correct nightly and the `riscv64gc-unknown-none-elf` target. No manual toolchain setup required.

### Build and Run

```bash
git clone https://github.com/hanvith6/VeridianOS.git
cd VeridianOS

make run          # Build kernel + monitor + disk, boot in QEMU
# Ctrl+A then X to exit QEMU

make build        # Build without running
make disk         # Rebuild disk.img only (after user_program changes)
```

### QEMU Boot Sequence

```
QEMU -bios monitor.bin -kernel kernel.bin -drive disk.img
         │                      │
         │  M-mode monitor      │  S-mode kernel
         │  (Phase 12)          │  (Phases 1-11.5)
         ▼                      ▼
  lock monitor PMP          init UART
  delegate traps            init memory
  mret to S-mode ──────►    init capabilities
                            init scheduler
                            load InitRAMFS
                            spawn hello
                            spawn enclave_test
                            ...
```

### How to Add a New Syscall

1. Add the constant to `kernel/src/syscall/numbers.rs`:
   ```rust
   pub const SYS_MY_FEATURE: usize = 130;
   ```

2. Write the handler in the appropriate module (e.g., `kernel/src/my_module/mod.rs`):
   ```rust
   pub fn sys_my_feature(arg0: usize, arg1: usize) -> isize {
       // validate args, do the work, return 0 on success or negative error
       0
   }
   ```

3. Wire it in `kernel/src/syscall/mod.rs` (the dispatch match):
   ```rust
   SYS_MY_FEATURE => sys_my_feature(a0, a1),
   ```

4. Write a user-space test in `user_programs/` using the raw `ecall` ABI.

5. Add the binary to the `disk` target in `Makefile`.

### Common Gotchas

| Problem | Cause | Fix |
|---------|-------|-----|
| `trap: illegal instruction` at boot | Wrong entry address | Check `link.ld` places `.text.boot` at `0x8020_0000` |
| Page fault on first user instruction | ELF stack not mapped | Verify `load_elf` allocates and maps the user stack VMO |
| SBI call returns `-3` | `SBI_ERR_INVALID_PARAM` | Check NAPOT alignment: `phys_start` must be `size`-aligned, `size` must be power-of-two |
| Monitor warning `unused_doc_comments` | `///` on `global_asm!` | Use `//` comments on macro invocations |
| Build fails: `can't find crate for 'std'` | Wrong target | Ensure `.cargo/config.toml` sets `target = "riscv64gc-unknown-none-elf"` |

---

## 17. Testing and QA Strategy

### Why Standard `cargo test` Does Not Apply

The kernel and monitor target `riscv64gc-unknown-none-elf` — a bare-metal target with no OS, no `std`, and no test runtime. Tests must run as QEMU user-space programs.

### Test Matrix

| Test Binary | Phase Coverage | What It Verifies |
|-------------|---------------|------------------|
| `hello` | 1–6 | Boot, UART, memory, ELF load, U-mode execution |
| `neural_test` | 7 | NES queue submission, node execution, DAG dependency resolution |
| `semantic_test` | 8 | SGF node create/get/delete, edge create/query |
| `agent_test` | 9 | Agent spawn, message send/recv, agent kill |
| `policy_test` | 10–11 | EMA latency updates, epsilon-greedy routing, distributed syscalls 90–101 |
| `smp_test` | 11.5 | Secondary hart activation, exception handler registration and recovery |
| `enclave_test` | 12 | Enclave create, enter/exit, SHA-256 + HMAC attestation report verification |

### How to Run a Specific Test

The kernel spawns all user programs listed in `disk.img` sequentially. To run only one program for debugging, modify `kernel/src/main.rs` to spawn only the target binary name.

### Phase 12 Integration Test Detail

`enclave_test` runs three assertions:

**TEST 1 — Enclave Creation**
- Calls `SYS_ENCLAVE_CREATE(phys=0x8610_0000, size=0x4000, entry=0x8610_0000)`
- Expects positive `enclave_id` (0–7) return
- Failure: any negative SBI error code

**TEST 2 — Enclave Entry and Exit**
- Writes a 12-byte RISC-V exit payload to the enclave region:
  ```
  li a0, 0          # exit code = 0
  li a7, 122        # SYS_ENCLAVE_EXIT
  ecall
  ```
- Calls `SYS_ENCLAVE_ENTER(enclave_id)` — expects clean return (0)
- Verifies syscall returns after enclave issues `SYS_ENCLAVE_EXIT`

**TEST 3 — Attestation Report**
- Calls `SYS_ENCLAVE_ATTEST(enclave_id, &report_buf)`
- Verifies `report[1..9]` (phys_start) == `0x8610_0000`
- Verifies `report[9..17]` (size) == `0x4000`
- Recomputes HMAC-SHA-256 of `report[17..49]` (measurement) with the known device key
- Compares computed HMAC to `report[49..73]`
- Failure: field mismatch or HMAC verification failure

### Build Verification

```bash
cargo build 2>&1 | grep -E "^error"   # Must produce no output
```

This is the primary CI check — all 13 phases' code must compile cleanly for `riscv64gc-unknown-none-elf`.

---

## 18. Research Foundations

### seL4 — Formal Verification of an OS Kernel (SOSP '09)
Mathematically proved functional correctness and security properties (integrity, confidentiality, isolation) of the L4 microkernel. Provides the core mathematical model for VeridianOS's `Handle` and `HandleTable` abstractions — every resource access is mediated by an unforgeable capability token, exactly as seL4 proved correct.

### LithOS — An OS for Efficient ML on GPUs (SOSP '25)
Proposes direct spatial scheduling of ML execution graphs on GPU hardware cores. Shows that bypassing standard driver latency bottlenecks yields orders-of-magnitude lower latency. Guides Phase 7 NES design: the kernel scheduler dispatches DAG nodes directly to device queues without user-space runtime overhead.

### AIOS — LLM Agent Operating System (COLM '25)
Treats Large Language Models and AI agents as primary system processes, designing scheduler abstractions for concurrent agent loops. Informs Phase 9 agent runtime: the OS manages agent lifecycles, allocates mailbox memory, and schedules execution without application-level runtime.

### Decima — Scheduling Algorithms via RL (SIGCOMM '19)
Uses Graph Neural Networks and Reinforcement Learning to schedule DAG task graphs in parallel computing clusters. Validates representing AI workloads as DAGs and scheduling them dynamically — directly validates VeridianOS Phase 7/10 design.

### Asterinas — Rust-Based Framekernel OS (USENIX ATC '25)
Details the "Framekernel" architecture that restricts `unsafe` code to a minimal TCB. Confirms that production-grade OS kernels can keep unsafe Rust below 5% of the codebase. Justifies VeridianOS's clean-slate Rust approach.

### Keystone Enclave (USENIX Security '20)
Open-source RISC-V TEE using PMP and an M-mode Security Monitor. Key insight: PMP CSRs are M-mode-exclusive — the kernel can never bypass them. Direct architecture inspiration for Phase 12. EID `0x08424B45`, 8-slot enclave pool, `lock_monitor_self()` pattern are all adapted from Keystone.

### RISC-V Privileged Specification v1.12
RISC-V Foundation specification for M/S/U privilege levels, PMP (§3.6), trap delegation (§3.3), and the SBI interface. Primary technical reference for all boot, trap, and PMP implementation decisions.

### Reinforcement Learning: An Introduction (Sutton & Barto, MIT Press 2018)
Chapter 2 (Multi-Armed Bandits) provides the mathematical basis for the epsilon-greedy device router in Phase 10. EMA update rule follows Chapter 2.4 (tracking a nonstationary problem).

---

## 19. Version Control and Branch Strategy

### Branch Model

```
main         ── stable, tagged releases (v0.1.0 through v0.12.0)
develop      ── active development branch; all PRs target develop
feature/*    ── short-lived feature branches (e.g., feature/phase12-pmp)
```

**Rule**: never push directly to `main`. Work on `develop`. Tag milestones as `v0.X.0` when a phase is complete and all test binaries pass.

### Commit Message Convention

```
<subsystem>: <description>

<optional body>
```

Examples:
```
kernel: fix ELF stack-segment collision with ASLR
monitor: implement NAPOT lock_region for PMP entry isolation
enclave: wire SYS_ENCLAVE_ATTEST to M-mode SBI call
docs: add Phase 12 design to DESIGN.md
```

Subsystem tokens: `kernel`, `monitor`, `enclave`, `nes`, `sgf`, `agent`, `dist`, `smp`, `docs`, `build`, `test`.

### Tagging

| Tag | Status | Description |
|-----|--------|-------------|
| v0.1.0 | ✅ | Phase 1: bootable kernel |
| v0.2.0 | ✅ | Phase 2: capability system |
| ... | ✅ | ... |
| v0.11.0 | ✅ | Phase 11: distributed coherence |
| v0.12.0-dev | 🔄 | Phase 12 in development |

---

## 20. Future Roadmap

### Phase 13 — VirtIO-Net and Real Distributed Testing

Replace the DKCP loopback simulation with a real `virtio-net` driver. Run two QEMU instances connected via a TAP bridge and execute the full distributed test suite (DCTP cap transfer, Raft log replication, remote NES dispatch) across two independent kernel images.

**Key work**: implement VirtIO network queue negotiation, IP/Ethernet framing for DKCP messages, TAP device setup in `Makefile`.

### Phase 14 — CHERI Pointer Safety

Evaluate CHERI-RISC-V capability instructions for hardware-enforced spatial memory safety. CHERI extends every pointer with bounds and permissions in fat-pointer hardware registers — a compromised driver cannot forge an out-of-bounds pointer. Key decision: whether to require a CHERI-enabled CPU variant or implement a software-emulated CHERI ABI.

### Phase 15 — Asymmetric Attestation (Ed25519)

Replace HMAC-SHA-256 in the Phase 12 attestation report with Ed25519 signatures. This allows remote verifiers to use only the public key (distributed via PKI), without needing the device private key. Requires implementing Ed25519 in `no_std` (using `curve25519-dalek` or a hand-rolled implementation).

### Phase 16 — Quantum-Classical Scheduling Interface

Define kernel object types for QPU execution queues. `QpuObject` wraps a quantum circuit execution ticket; `SYS_QPU_SUBMIT` and `SYS_QPU_WAIT` expose QPU scheduling through the same capability-based interface as NES. Informed by QOS (arXiv '25) and RISC-V's emerging quantum extension proposals.

### Longer Term (2030+)

- **CXL memory disaggregation**: VMO handles as CXL memory pool references; Phase 11 DKCP as the transport layer for memory coherence across physical nodes.
- **Neuromorphic device integration**: Route spike-train event streams from Intel Loihi 2 / SpiNNaker 2 devices as kernel interrupt sources into NES queues.
- **Formal verification**: Apply Lean 4 or Isabelle/HOL to prove the capability system invariants (no amplification, no confused deputy) — following seL4's proof methodology.
- **Energy-aware scheduling**: Extend the Phase 10 EMA router with power consumption signatures from RISC-V platform power management registers; defer batch workloads to low-carbon grid windows.

---

*Document reflects implementation state as of Phase 12 (complete). See `ROADMAP.md` for one-line status table and `README.md` for quick-start instructions.*
