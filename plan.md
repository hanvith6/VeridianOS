# VeridianOS — Master Development Plan

> **AI-Native, Capability-Based Operating System in Rust for RISC-V 64-bit**
> Dual-licensed: MIT + Apache 2.0

---

## Executive Summary

VeridianOS is a clean-slate operating system designed for the next 30 years of computing. It departs
from POSIX/Unix assumptions (made in the 1970s for single-CPU, no-AI, no-GPU systems) and instead
treats heterogeneous AI workloads, capability-based access control, and semantic data storage as
first-class primitives baked into the microkernel itself.

```
╔══════════════════════════════════════════════════════════════════════╗
║                        VeridianOS System Stack                       ║
╠══════════════════════════════════════════════════════════════════════╣
║  USER SPACE (U-mode, vaddr 0x40000000+)                              ║
║  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌──────────────────┐  ║
║  │  AI Agent  │ │  AI Agent  │ │  User App  │ │  OS Services     │  ║
║  │ Process A  │ │ Process B  │ │ (POSIX shim│ │ (VirtIO driver,  │  ║
║  │            │ │            │ │ in future) │ │  NetStack, etc.) │  ║
║  └─────┬──────┘ └─────┬──────┘ └─────┬──────┘ └────────┬─────────┘  ║
║        └──────────────┴──────────────┴─────────────────┘            ║
║                               │ ecall (syscall)                      ║
╠═══════════════════════════════╪══════════════════════════════════════╣
║  SUPERVISOR (S-mode, vaddr 0x80200000+)                              ║
║  ┌────────────────────────────▼────────────────────────────────┐     ║
║  │                    VERIDIAN KERNEL                          │     ║
║  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │     ║
║  │  │  Capability  │  │   Neural     │  │  Semantic Graph  │  │     ║
║  │  │  Manager     │  │  Scheduler   │  │  Storage (Ph. 8) │  │     ║
║  │  │  (Handles,   │  │  (NES: GPU/  │  │  (Objects +      │  │     ║
║  │  │   Rights,    │  │   NPU/CPU    │  │   Relationships  │  │     ║
║  │  │   HandleTbl) │  │   Queues)    │  │   graph store)   │  │     ║
║  │  └──────────────┘  └──────────────┘  └──────────────────┘  │     ║
║  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │     ║
║  │  │  Memory Mgr  │  │  Thread/Sched│  │  Trap Handler    │  │     ║
║  │  │  (Buddy alloc│  │  (Round-robin│  │  (trap.S + Rust) │  │     ║
║  │  │   Sv39 VM)   │  │   preemptive)│  │  ecall dispatch  │  │     ║
║  │  └──────────────┘  └──────────────┘  └──────────────────┘  │     ║
║  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │     ║
║  │  │  VirtIO Blk  │  │  ELF Loader  │  │  Page Allocator  │  │     ║
║  │  │  Driver +    │  │  + InitRAMFS │  │  (Binary Buddy)  │  │     ║
║  │  │  InitRAMFS   │  │  (ustar TAR) │  │  + Sv39 Tables   │  │     ║
║  │  └──────────────┘  └──────────────┘  └──────────────────┘  │     ║
║  └─────────────────────────────────────────────────────────────┘     ║
╠══════════════════════════════════════════════════════════════════════╣
║  MACHINE (M-mode) — OpenSBI firmware                                 ║
║  Hardware: QEMU virt (RISC-V 64-bit, 128MB RAM, VirtIO MMIO)        ║
╚══════════════════════════════════════════════════════════════════════╝
```

---

## Development Phases Overview

| # | Phase | Status | Key Deliverable |
|---|-------|--------|-----------------|
| 1 | Bootable RISC-V Microkernel | ✅ Done | boot.S, UART, panic handler, linker script |
| 2 | Capability System Foundation | ✅ Done | Handle, HandleTable, Rights bitflags |
| 3 | Page Allocator & Sv39 VM | ✅ Done | Binary Buddy allocator, page tables |
| 4 | Preemptive Thread Scheduler | ✅ Done | Round-robin, timer interrupt, context switch |
| 5 | VirtIO Block Driver & InitRAMFS | ✅ Done | VirtIO MMIO, ustar TAR loader |
| 6 | ELF Loader & User Mode Transition | ✅ Done | PT_LOAD segments, Sv39 user mappings, ecall |
| 7 | Neural Execution Subsystem (NES) | ✅ Done | TaskGraph DAG, GPU/NPU simulated queues |
| 8 | Semantic Knowledge Graph FS | ✅ Done | Replace files with entity-relationship graph |
| 9 | Agent Runtime | ✅ Done | Kernel-native AI agent scheduling |
| 10 | Self-Improving Kernel Policies | 📋 Planned | ML-driven scheduling parameter tuning |
| 11 | Production Hardening | 📋 Planned | Formal verification, security audit, drivers |

---

## Phase 1 — Bootable RISC-V Microkernel

### Overview
The foundation: a bare-metal Rust kernel that boots on QEMU's 64-bit RISC-V `virt` machine via
OpenSBI, initializes a console, and enters a safe supervisor idle loop.

### Boot Sequence
```
Power On / QEMU Start
       │
       ▼
 OpenSBI (M-mode)         ← QEMU -bios default loads OpenSBI
       │ Jumps to 0x80200000 in S-mode
       ▼
 boot.S _start            ← .text.entry (must be first in linker)
  ├─ Only hart 0 proceeds (bnez a0, _park_hart)
  ├─ la sp, _stack_top    ← 64KB boot stack from linker.ld
  ├─ Zero BSS (_bss_start .. _bss_end)
  └─ call kmain           ← jump to Rust
       │
       ▼
 kmain() in main.rs
  ├─ uart::init()
  ├─ Print boot banner (ASCII art)
  ├─ memory::init()
  ├─ capability system setup
  └─ Idle loop (wfi)
```

### Key Files
| File | Purpose |
|------|---------|
| `kernel/src/arch/riscv64/boot.S` | Assembly entry, stack setup, BSS zero, switch_context |
| `kernel/src/arch/riscv64/linker.ld` | Memory layout: text→rodata→data→bss→stack(64KB)→heap(16MB) |
| `kernel/src/uart.rs` | UART 16550 driver at MMIO 0x10000000, `println!` macro |
| `kernel/src/panic.rs` | Panic handler: print message + halt |
| `kernel/src/main.rs` | `kmain()` entry point |

### Physical Memory Map (QEMU virt)
```
0x0000_0000 – 0x0FFF_FFFF   ROM / Reserved
0x1000_0000                 UART 16550 MMIO base
0x1000_1000 – 0x1000_7FFF   VirtIO MMIO slots (8 slots × 0x1000)
0x2000_0000 – 0x2000_FFFF   CLINT (Core Local Interrupt)
0x0C00_0000 – 0x0FFF_FFFF   PLIC (Platform-Level Interrupt Controller)
0x8000_0000 – 0x8001_FFFF   OpenSBI (128KB)
0x8020_0000 – ?             Kernel image (.text, .rodata, .data, .bss)
         ?  – ?+64KB        Kernel boot stack (_stack_bottom.._stack_top)
         ?  – ?+16MB        Kernel heap (_heap_start.._heap_end)
         ?  – 0x8800_0000   Free pages for buddy allocator
```

---

## Phase 2 — Capability System Foundation

### Overview
Implements VeridianOS's core security model: every resource access requires an unforgeable
**Handle** (capability token). No ambient authority, no root user.

### Capability Model
```
Process Handle Table
┌────┬──────────────┬──────────────────────────────┐
│ ID │ ObjectType   │ Rights                        │
├────┼──────────────┼──────────────────────────────┤
│  0 │ UartDevice   │ READ | WRITE                  │
│  1 │ MemoryRegion │ READ | WRITE | MAP            │
│  2 │ Channel      │ READ | WRITE | TRANSFER       │
│  3 │ TaskGraph    │ READ | SUBMIT | WAIT          │
└────┴──────────────┴──────────────────────────────┘
        │
        │ sys_handle_close(3)       → removes entry, revokes access
        │ sys_handle_duplicate(2,0) → copies with subset rights only
        └──────────────────────────────────────────────────────────►
```

### Rights Bitflags
```rust
bitflags! {
    pub struct Rights: u32 {
        const READ      = 0b0000_0001;
        const WRITE     = 0b0000_0010;
        const EXECUTE   = 0b0000_0100;
        const DUPLICATE = 0b0000_1000;
        const TRANSFER  = 0b0001_0000;
    }
}
```

### Key Invariants
- Rights can only be **attenuated** (reduced), never escalated during duplication
- A handle is only valid within its owning process's HandleTable
- The kernel never exposes raw pointers to userspace

### Key Files
| File | Purpose |
|------|---------|
| `kernel/src/capability/mod.rs` | Handle, HandleTable, Rights, ObjectType definitions |
| `kernel/src/syscall/mod.rs` | sys_handle_close, sys_handle_duplicate implementations |

---

## Phase 3 — Page Allocator & Sv39 Virtual Memory

### Overview
Physical page management via a Binary Buddy Allocator and virtual address translation via
RISC-V Sv39 three-level page tables.

### Binary Buddy Allocator
- Orders 0–10: 4KB (1 page) to 4MB (1024 pages)
- Free lists: `free_lists: [Option<*mut PageNode>; 11]`
- PageNode written directly into the free block (no heap needed)
- Allocation: find block at order, split if needed, return lower half
- Deallocation: merge with buddy if free (XOR addresses to find buddy)

### Sv39 Virtual Memory
```
Virtual Address (39-bit):
 38      30  29      21  20      12  11        0
┌──────────┬──────────┬──────────┬────────────┐
│  VPN[2]  │  VPN[1]  │  VPN[0]  │   Offset   │
│  9 bits  │  9 bits  │  9 bits  │  12 bits   │
└──────────┴──────────┴──────────┴────────────┘
     │           │           │
     ▼           ▼           ▼
 L2 Table    L1 Table    L0 Table → Physical Page
 (512 PTEs)  (512 PTEs)  (512 PTEs)
```

### Page Table Flags
```
PTE Bits: V(0) R(1) W(2) X(3) U(4) G(5) A(6) D(7)
V = Valid, R = Read, W = Write, X = Execute
U = User accessible (set for U-mode pages)
A = Accessed, D = Dirty (set by hardware on access/write)
```

### Key Files
| File | Purpose |
|------|---------|
| `kernel/src/memory/page_alloc.rs` | Binary Buddy allocator implementation |
| `kernel/src/memory/page_table.rs` | Sv39 PageTable, map_page(), unmap_page() |
| `kernel/src/memory/mod.rs` | Memory subsystem init, KERNEL_PAGE_TABLE global |

---

## Phase 4 — Preemptive Thread Scheduler

### Overview
Round-robin preemptive multitasking using RISC-V Supervisor Timer Interrupts (STIE).

### Thread Structure
```rust
#[repr(C)]
#[repr(align(16))]
pub struct Thread {
    pub tid:     usize,        // Thread ID
    pub state:   ThreadState,  // Ready | Running | Blocked | Exited
    pub context: ThreadContext, // ra, sp, s0-s11 (callee-saved)
    pub stack:   Stack,        // 16KB kernel stack [u8; 16384]
    pub satp:    usize,        // Address space (Sv39 SATP value)
}
```

### Context Switch (switch_context in boot.S)
```
Thread A                    Thread B
 running                     waiting
    │                           │
    │  Timer Interrupt fires    │
    │  (STIE, scause=0x80..05)  │
    ▼                           │
trap_handler() calls            │
thread::schedule()              │
    │                           │
    ▼                           │
switch_context(&A.ctx, &B.ctx)  │
  ├─ sd ra,sp,s0-s11 → A.ctx   │
  ├─ ld ra,sp,s0-s11 ← B.ctx   │
  └─ csrw satp, B.satp          │
     sfence.vma                 │
     ret (jumps to B's ra)      │
                                ▼
                           Thread B
                            running
```

### Scheduler State Machine
```
         spawn()            timer interrupt
  None ──────────► Ready ◄──────────────── Running
                      │                       │
                      │ schedule() picks       │
                      └──────────────────────►│
                                              │
                   block_current_thread()     │
  Blocked ◄─────────────────────────────────-┘
     │
     │ wakeup_thread(tid)
     └──────────────────────► Ready
```

### Key Files
| File | Purpose |
|------|---------|
| `kernel/src/process/thread.rs` | Thread, ThreadContext, SchedulerState, spawn/schedule/block/wakeup |
| `kernel/src/arch/riscv64/boot.S` | switch_context assembly |
| `kernel/src/trap.rs` | Timer interrupt → thread::schedule() dispatch |

---

## Phase 5 — VirtIO Block Driver & InitRAMFS

### Overview
Loads a ustar TAR disk image via VirtIO MMIO block device and parses it as an in-memory
filesystem (InitRAMFS).

### VirtIO Block I/O Flow
```
Kernel                              VirtIO Device (QEMU)
  │                                       │
  │  1. Build descriptor chain:           │
  │     Desc[0]: BlkReq{type=READ, lba}  │
  │     Desc[1]: 512-byte data buffer    │
  │     Desc[2]: 1-byte status           │
  │                                       │
  │  2. Write to AvailRing               │
  │     avail.ring[avail.idx] = desc[0]  │
  │     avail.idx++                       │
  │                                       │
  │  3. MMIO doorbell: QUEUE_NOTIFY=0 ──────────────► Device processes
  │                                                    descriptor chain
  │  4. Poll UsedRing:                   │
  │     while used.idx != target: spin  ◄────────────── Device writes
  │                                       status byte
  │  5. Check status byte (0=OK)         │
  │                                       │
  └───────────────────────────────────────┘
```

### InitRAMFS (ustar TAR)
- disk.img is a ustar TAR archive embedded in kernel memory
- Parser reads 512-byte headers, extracts filename + size
- Files mapped into memory without dynamic allocation
- Currently holds: `hello` (user hello world), `neural_test` (Phase 7 test)

### Key Files
| File | Purpose |
|------|---------|
| `kernel/src/virtio/mod.rs` | VirtIO MMIO discovery, VirtqDesc/Avail/Used ring structs |
| `kernel/src/virtio/blk.rs` | Block device driver, read_sector(), read_sectors() |
| `kernel/src/fs/initramfs.rs` | ustar TAR parser, file lookup by name |

---

## Phase 6 — ELF Loader & User Mode Transition

### Overview
Parses ELF64 executables, maps PT_LOAD segments into a new user address space, and executes
them in U-mode (User Mode) with system call support.

### ELF Loading Flow
```
ELF binary bytes (from InitRAMFS)
         │
         ▼
  1. Verify ELF magic (0x7F 'E' 'L' 'F')
  2. Check ELF64, RISC-V, executable
  3. Read entry_point from e_entry
         │
         ▼
  For each PT_LOAD segment:
  ┌─────────────────────────────────────────────┐
  │  a. Allocate physical pages (buddy alloc)   │
  │  b. Copy segment data from ELF bytes        │
  │  c. Map vaddr → paddr in user page table    │
  │     with flags: R | W | X | U              │
  │  d. Zero memsz - filesz (BSS within seg)   │
  └─────────────────────────────────────────────┘
         │
         ▼
  Allocate user stack (4 pages = 16KB)
  Map at user vaddr 0x40002000 → 0x40006000
  stack_top = 0x40006000 - 16 (ABI alignment)
         │
         ▼
  spawn_user_thread(entry, stack_top, user_satp)
         │
         ▼
  Scheduler runs user_mode_trampoline:
  ├─ release_lock() (force-unlock scheduler spinlock)
  ├─ sfence.vma (TLB flush for new address space)
  ├─ read sp (kernel stack for sscratch)
  └─ enter_user_mode(entry, user_sp, kernel_sp)
       ├─ csrc sstatus, SPP  (drop to U-mode on sret)
       ├─ csrs sstatus, SPIE (enable interrupts in U-mode)
       ├─ csrw sepc, entry_point
       ├─ csrw sscratch, kernel_sp  ← CRITICAL: kernel stack top for trap
       ├─ mv sp, user_sp
       └─ sret → CPU enters U-mode, jumps to entry_point
```

### System Call Dispatch (ecall flow)
```
User code: ecall (a7=syscall_id, a0=arg0, a1=arg1 ...)
                │
                ▼
        trap_vector (trap.S)
        ├─ csrrw sp, sscratch, sp   [sp ↔ sscratch: kernel_sp ↔ user_sp]
        ├─ addi sp, sp, -272        [allocate TrapFrame]
        ├─ save all 32 regs
        ├─ save sstatus, sepc
        └─ call trap_handler(tf_ptr)
                │
                ▼
        trap_handler (trap.rs)
        ├─ csrr scause              [read cause]
        ├─ if scause == 8:          [U-mode ecall]
        │   ├─ id = tf.regs[17]    [a7]
        │   ├─ args = tf.regs[10-14] [a0-a4]
        │   ├─ ret = syscall_handler(id, args...)
        │   ├─ tf.regs[10] = ret   [write return value to a0]
        │   └─ tf.sepc += 4        [advance past ecall instruction]
        └─ if interrupt (bit 63): → thread::schedule()
                │
                ▼
        (restore from TrapFrame)
        ├─ restore sstatus, sepc
        ├─ if returning to U-mode: csrw sscratch, sp+272
        ├─ restore all 32 regs
        └─ sret → return to user
```

---

## System Call Reference

### Quick Reference Table

| Syscall | ID | Arguments | Return | Description |
|---------|----|-----------|--------|-------------|
| `SYS_WRITE` | 1 | a0=ptr, a1=len | bytes written or -1 | Print UTF-8 string to UART |
| `SYS_EXIT` | 2 | a0=status | (noreturn) | Terminate current thread |
| `SYS_HANDLE_CLOSE` | 3 | a0=handle_id | 0 or error | Revoke a capability handle |
| `SYS_HANDLE_DUPLICATE` | 4 | a0=src, a1=rights_mask | new_id or error | Copy handle with ≤ original rights |
| `SYS_GRAPH_CREATE` | 50 | — | graph_handle or -1 | Create a new NES TaskGraph |
| `SYS_GRAPH_ADD_NODE` | 51 | a0=graph_h, a1=op_type, a2=cfg_ptr, a3=dep_count, a4=dep_arr | node_id or -1 | Add node to TaskGraph |
| `SYS_GRAPH_SUBMIT` | 52 | a0=graph_h, a1=queue_h | 0 or error | Validate & submit graph for execution |
| `SYS_GRAPH_WAIT` | 53 | a0=graph_h, a1=timeout_us | 0=done, 1=timeout, -1=err | Block until graph completes |

### Error Codes
| Code | Name | Meaning |
|------|------|---------|
| -1 | ENOSYS / EFAULT | Unknown syscall or bad address |
| -2 | EBADF | Invalid handle ID |
| -3 | EPERM | No active process / permission denied |
| -12 | ENOMEM | Handle table full |
| -13 | EACCES | Rights not held |

---

## Phase 7 — Neural Execution Subsystem (NES)

### Overview
The kernel-native heterogeneous task scheduler. AI computation graphs (DAGs of tensor operators)
are submitted to the kernel, which dispatches nodes to GPU/NPU/CPU hardware queues.

### NES Architecture
```
User Process
  sys_graph_create() → graph_handle
  sys_graph_add_node(graph, GEMM, cfg, deps) → node_id_0
  sys_graph_add_node(graph, RELU, cfg, [node_id_0]) → node_id_1
  sys_graph_add_node(graph, VECTOR_ADD, cfg, [node_id_1]) → node_id_2
  sys_graph_submit(graph, queue_handle)
  sys_graph_wait(graph, timeout_us)

                        │
            ┌───────────▼───────────┐
            │   NES Graph Validator  │
            │  (DAG cycle check,    │
            │   VMO ownership,      │
            │   bounds checking)    │
            └───────────┬───────────┘
                        │
            ┌───────────▼───────────┐
            │  Topological Scheduler │
            │  (in-degree decrement, │
            │   dependency tracking) │
            └──┬────────┬────────┬───┘
               │        │        │
               ▼        ▼        ▼
          ┌────────┐ ┌──────┐ ┌──────┐
          │  NPU   │ │  GPU │ │  CPU │
          │ Queue  │ │Queue │ │Queue │
          │(GEMM,  │ │(Vec  │ │(Relu,│
          │ Conv)  │ │ Ops) │ │ Elem)│
          └────────┘ └──────┘ └──────┘
               │        │        │
               └────────┴────────┘
                        │
            ┌───────────▼───────────┐
            │  S-Mode Worker Threads │
            │  (simulate GPU/NPU     │
            │   on QEMU virt)        │
            └───────────────────────┘
```

### NES Key Data Structures
| Type | File | Description |
|------|------|-------------|
| `TaskGraph` | `nes/graph.rs` | DAG of up to 32 nodes, adjacency lists, completion state |
| `TaskNode` | `nes/graph.rs` | op_type, in_degree, successor list, NodeState |
| `HeterogeneousQueue` | `nes/queue.rs` | MMIO-backed ring buffer per device type |
| `OpType` | `nes/types.rs` | GEMM, ReLU, VectorAdd, Conv2D, Attention |
| `DeviceType` | `nes/types.rs` | CPU, GPU, NPU |

### Current Status
- ✅ Types, graph, queue, validator, simulator modules created
- ✅ Syscalls 50-53 implemented
- ✅ Resolved `sscratch` setup mismatch during U-mode transition, successfully running U-mode preemptive execution
- ✅ Math verification suite (`neural_test`) executes successfully on boot with 131.0 result

---

## Phase 8 — Semantic Knowledge Graph Filesystem (Completed)

### Current Status
- ✅ Types, static-array graph store, and query predicate matching logic implemented
- ✅ Syscalls 60-63 (`sys_node_create`, `sys_edge_add`, `sys_node_write`, `sys_graph_query`) registered and fully implemented
- ✅ Successfully mapped VMO dynamically assigned memory regions starting at `0x5000_0000` to processes for blob read/write operations
- ✅ Verification program `semantic_test` parses properties, creates nodes and directed edges, performs queries and edge verification, returning status 0

### Design Vision
Replace Unix-style hierarchical byte-file storage with a **semantic graph database** built into
the kernel. Data is stored as entities (Objects) and relationships (labeled directed edges).

```
Traditional FS:        VeridianOS Semantic Graph:
/home/                 Object{Invoice_2024_Q4}
  user/                   ─[is_invoice_for]→ Object{Company_Acme}
    documents/             ─[authored_by]→   Object{Person_Alice}
      invoice.pdf          ─[contains]→      Object{Blob_PDF_data}

ls /home/user/docs/     query("invoices from Acme in 2024")
  invoice.pdf             → [Invoice_2024_Q4, Invoice_2024_Q3]
```

### Kernel Objects for Phase 8
| Object | Syscall to create | Rights |
|--------|------------------|--------|
| GraphNode | sys_node_create | READ, WRITE, LINK, DELETE |
| GraphEdge | sys_edge_add | READ, DELETE |
| GraphQuery | sys_graph_query | READ |

### Storage Backend
- Raw key-value store on VirtIO block device (built on Phase 5 driver)
- Key: `ObjectId` (u64, monotonically incrementing)
- Value: Serialized `GraphNode` (fixed-size struct, no heap needed in kernel)

---

## Phase 9 — Agent Runtime (Completed)

### Current Status
- ✅ Added `AgentProcess` and `AgentChannel` to `ObjectType` enum for capability-secured access.
- ✅ Implemented Agent Record tracking structure (`AgentRecord` / `AgentPool`) in kernel-space.
- ✅ Implemented Agent System Calls: `SYS_AGENT_SPAWN` (70), `SYS_CHANNEL_CREATE` (71), `SYS_CHANNEL_SEND` (72), `SYS_CHANNEL_RECV` (73), and `SYS_AGENT_STATUS` (74).
- ✅ Reused the robust existing Capability Channel IPC system to handle secure message transfers between agent harts.
- ✅ Verification program `agent_test` successfully spawns orchestrator and worker agents, constructs IPC channel, executes message transfer, decodes/verifies sender ID and payloads, and terminates cleanly with exit code 0.

### System Calls for Phase 9
| Syscall | Number | Description | Key Arguments |
|---------|--------|-------------|---------------|
| `SYS_AGENT_SPAWN` | 70 | Creates a new agent record in kernel pool | `parent_id`, `intent_ptr`, `intent_len` |
| `SYS_CHANNEL_CREATE` | 71 | Allocates a capability-secured IPC channel | `owner_agent_id` |
| `SYS_CHANNEL_SEND` | 72 | Writes a message to an IPC channel | `channel_id`, `payload_ptr`, `payload_len` |
| `SYS_CHANNEL_RECV` | 73 | Reads a message from an IPC channel | `channel_id`, `out_buf_ptr`, `out_len_ptr` |
| `SYS_AGENT_STATUS` | 74 | Queries the execution state of an agent | `agent_id`, `out_state_ptr` |

---

## Microkernel Architecture — Comparison

| Feature | VeridianOS | Linux (monolithic) | Fuchsia/Zircon | seL4 |
|---------|-----------|-------------------|----------------|------|
| Privilege model | S-mode (Supervisor) only in kernel | Ring 0 (huge TCB) | Zircon in kernel | Microkernel |
| Security model | Capability handles | POSIX UID/GID + root | Capability (FIDL) | Capability (IPC) |
| Scheduler | Heterogeneous (CPU/GPU/NPU) | CFS (CPU-only) | Proportional share | Earliest deadline |
| Storage | Semantic graph (planned) | VFS + ext4/btrfs | FIDL + Fxfs | Delegation to servers |
| Language | Rust | C | C++ | C (verified) |
| Target | RISC-V 64 | x86/ARM/RISC-V | x86/ARM | ARM/x86 |
| Verification | Planned | None | None | Full formal proof |

---

## Build System & Debug Commands

### Prerequisites
```bash
rustup install nightly
rustup target add riscv64gc-unknown-none-elf
brew install qemu
```

### Build Commands
```bash
cargo build --release      # Build kernel + user programs
make run                   # Build disk.img and launch QEMU
make clean                 # Clean build artifacts
cargo clippy               # Lint check
cargo fmt                  # Format code
```

### QEMU Invocation (from Makefile)
```
qemu-system-riscv64 \
  -machine virt \
  -bios default \          # OpenSBI
  -m 128M \                # 128 MB RAM
  -nographic \             # UART console
  -drive file=disk.img,if=virtio,format=raw \  # VirtIO block
  -kernel target/riscv64gc-unknown-none-elf/release/veridian-kernel
```

### Debug Commands
```bash
# Disassemble kernel binary
rust-lldb -o "target create target/riscv64gc-unknown-none-elf/release/veridian-kernel" \
          -o "di -s 0x80200000 -e 0x80201000" -o "quit"

# Symbol lookup
rust-lldb -o "target create ..." -o "image lookup -s trap_vector" -o "quit"

# QEMU with GDB stub (use 'make debug' if available)
qemu-system-riscv64 [...] -s -S    # Listens on port 1234
riscv64-unknown-elf-gdb kernel -ex "target remote :1234"
```

### Workspace Layout
```
OS/
├── Cargo.toml              # Workspace (kernel, user_programs/*)
├── Makefile                # build/run/clean targets
├── rust-toolchain.toml     # nightly + riscv64gc target
├── disk.img                # ustar TAR (auto-built by make)
├── plan.md                 # This file
├── README.md               # Project README
├── docs/
│   ├── ARCHITECTURE.md
│   ├── NEURAL_SCHEDULER_DESIGN.md
│   ├── ACADEMIC_REFERENCES.md
│   └── FUTURE_COMPUTING_TRENDS.md
├── kernel/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         # kmain() entry
│       ├── uart.rs         # UART driver + println!
│       ├── trap.rs         # enter_user_mode, trap_handler
│       ├── panic.rs        # panic handler
│       ├── sbi.rs          # OpenSBI ecall wrappers (set_timer, get_time)
│       ├── arch/riscv64/
│       │   ├── boot.S      # _start, switch_context assembly
│       │   ├── trap.S      # trap_vector assembly
│       │   └── linker.ld   # Memory layout
│       ├── capability/     # Handle, HandleTable, Rights
│       ├── memory/         # Buddy allocator, Sv39 page tables
│       ├── process/        # Thread, Scheduler, ELF loader, spawn
│       ├── syscall/        # syscall_handler, sys_write, sys_exit, etc.
│       ├── nes/            # Neural Execution Subsystem
│       ├── fs/             # InitRAMFS (ustar TAR)
│       └── virtio/         # VirtIO MMIO block driver
└── user_programs/
    ├── hello/              # Simple hello world
    └── neural_test/        # Phase 7 NES verification test
```

---

## Known Issues & Active Debugging

### Issue: Illegal Instruction (scause=2) at 0x8020084A
**Status:** Fix applied (pending verification in next run)

**Symptom:**
```
[SYSCALL Debug] id=1, args=(0x400007CE, 0x43, ...)
UNHANDLED EXCEPTION  Cause: 0x2 (Illegal Instruction)
sepc: 0x8020084A    TrapFrame: 0x80200880
```

**Root Cause:** `sscratch` held a value (~0x80200B00) within kernel TEXT space.
When the `ecall` trap fired, `trap_vector` computed `sp = sscratch - 272`,
landing at `0x80200880` — inside kernel code — and wrote the TrapFrame over live
instructions. The instruction at `0x8020084A` was therefore corrupted.

**Why sscratch was wrong:** The original `enter_user_mode` did `csrw sscratch, sp`
using the *current call-chain sp* (deep in function call frames), not the *thread's
kernel stack top* (`0x8021C120` for thread 1).

**Fix applied to:**
- `kernel/src/trap.rs`: `enter_user_mode` now accepts `kernel_sp` as 3rd parameter
- `kernel/src/process/thread.rs`: trampoline reads `sp` immediately after context
  switch (very near the thread stack top) and passes it as `kernel_sp`

---

## Academic Foundations

| Paper | Venue | Relevance |
|-------|-------|-----------|
| *seL4: Formal Verification of an OS Kernel* (Klein et al.) | SOSP '09 | Capability model, microkernel isolation |
| *LithOS: An OS for Efficient ML on GPUs* | SOSP '25 | AI-native scheduling, GPU kernel objects |
| *Asterinas: A Rust Framekernel with Linux ABI* | ATC '25 | Rust OS safety, syscall compatibility |
| *Theseus OS* (Boos et al.) | OSDI '20 | Rust ownership for OS safety properties |
| *Unikraft: Fast, Specialized Unikernels* | EuroSys '21 | Minimal TCB, specialization |
| *CHERI: Capability Hardware for RISC* (Watson et al.) | IEEE S&P '15 | Hardware capability enforcement |
| *Fuchsia Zircon kernel design* (Google, 2016–) | Open source | Capability-based IPC, handle model |
| *The Semantic File System* (Gifford et al.) | SOSP '91 | Semantic storage inspiration |

---

*Last updated: 2026-05-27 | VeridianOS is dual-licensed MIT + Apache 2.0*
