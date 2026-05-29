# VeridianOS Architecture Specification

This document describes the software architecture of **VeridianOS** — a next-generation microkernel built in Rust targeting RISC-V 64-bit processors. It reflects the current implementation state as of Phase 12 (in progress).

---

## 1. Design Philosophy

VeridianOS departs from traditional Unix-style monolithic designs. It is built on four core pillars:

1. **Microkernel Architecture**: Device drivers, filesystems, and network stacks live in sandboxed user-space processes, not in the kernel. The Trusted Computing Base (TCB) is kept to a minimal S-mode kernel. A separate M-mode monitor binary (`monitor/` crate) enforces hardware isolation below the kernel itself.

2. **Capability-Based Security**: There is no ambient authority (no `root`, no UID-based file permissions). Access to every resource is governed exclusively by unforgeable capability handles stored in per-process handle tables. Capabilities flow explicitly between processes via IPC; they cannot be guessed or forged.

3. **AI-Native Orchestration**: The kernel treats AI workloads as primary citizens. Instead of running neural inference as high-level application threads, it schedules `TaskGraph` nodes — typed, dependency-aware compute operations — directly onto heterogeneous hardware queues (CPU, GPU, NPU) with explicit latency tracking.

4. **Online Policy Learning**: The neural scheduler is not static. A `PolicyStats` matrix of EMA-smoothed latency priors (6 operation types × 3 device types) is updated after every completed graph node. An ε-greedy selection algorithm uses these priors, plus current queue depth, to route future work toward the empirically fastest device. This loop runs entirely inside the kernel at S-mode with no user-space involvement.

5. **Symmetric Multiprocessing (SMP)**: The kernel runs on 4 RISC-V hardware threads (harts) simultaneously. Each hart runs an independent copy of the round-robin scheduler, selecting from the shared thread pool. The `ThreadState::Running(hart_id)` variant tags which hart owns a thread at any moment, preventing double-scheduling.

6. **Heap-Backed Kernel Objects**: The kernel uses `linked_list_allocator` for a 16 MB kernel heap, enabling heap-allocated thread stacks (`Box<Stack>`) and dynamic process table entries (`Option<Process>`). This removes the need for all-static allocation pools in critical paths.

---

## 2. Kernel Privilege Separation

RISC-V provides three hardware privilege levels. VeridianOS uses all three:

| Privilege Mode | Name | Software Components |
|---|---|---|
| **M-mode** (Machine) | Firmware / TEE Monitor | OpenSBI handles low-level hardware initialization and the standard SBI interface. A separate VeridianOS M-mode monitor (`monitor/` crate) adds SBI extension EID `0x08424B45` for PMP-based enclave management and hardware attestation. |
| **S-mode** (Supervisor) | Microkernel | The Veridian kernel — page table management, capability checks, SMP thread scheduler, IPC message routing, NES task graph dispatch, semantic graph filesystem, agent runtime, distributed coherence (Raft + DCTP + DKCP), and user-space exception delivery. |
| **U-mode** (User) | Drivers, OS Services, Apps | VirtIO block driver, Semantic Graph Filesystem service, neural runtime libraries, user applications, AI agents, and enclave-isolated agent processes. |

---

## 3. Boot Sequence

The full boot sequence for hart 0 (the primary boot hart), from power-on to idle:

```
OpenSBI (M-mode)
  │  Platform initialization, interrupt delegation to S-mode
  │  Transfers control to kernel via mret to 0x80200000
  │
boot.S (_start)
  │  Zero BSS section
  │  Set up initial kernel stack
  │  Write hart_id into tp register (used by get_hart_id())
  │  Call kmain(hart_id, dtb_ptr)
  │
kmain (kernel/src/main.rs)
  ├─ uart::WRITER.lock().init()           — UART serial console
  ├─ trap::init()                         — stvec → trap_vector (trap.S)
  ├─ memory::init(dtb_ptr)
  │    ├─ ALLOCATOR.lock().init(heap_start, 16 MB)   — linked_list_allocator heap
  │    ├─ page_alloc::init(free_mem_start, 0x8800_0000) — buddy page allocator
  │    ├─ KERNEL_PAGE_TABLE identity-map kernel + RAM + UART + VirtIO MMIO
  │    └─ root_table.activate()           — csrw satp, enable Sv39 paging
  ├─ Process::new(pid=1) → PROCESS_TABLE[0]  — root capability process
  ├─ thread::init()
  │    ├─ Register boot context as Thread 0 (Running on hart 0)
  │    └─ Register dummy placeholder threads for harts 1–3
  ├─ nes::init()                          — NES policy stats + task graph pool
  ├─ semantic_graph::init()               — SGF node/edge store
  ├─ agent::init()                        — AgentPool, channel pool
  ├─ dist::cluster::cluster_init(KernelDomainId(0))
  ├─ dist::raft::raft_init()              — Raft state: Leader, term=1
  ├─ virtio::blk::init()                  — VirtIO block device
  ├─ fs::RamFs::load_from_disk()          — Parse ustar RAMFS from disk.img
  ├─ process::spawn(init_binary, elf_data) — ELF load + user thread creation
  ├─ thread::spawn_thread(nes::simulator::cpu_worker)
  ├─ thread::spawn_thread(nes::simulator::gpu_worker)
  ├─ thread::spawn_thread(nes::simulator::npu_worker)
  ├─ smp::init()                          — SBI HSM start for harts 1, 2, 3
  └─ thread::schedule() → WFI idle loop

Secondary harts (harts 1–3) entry: ksecondary_main(_hart_id)
  ├─ csrw satp (load KERNEL_PAGE_TABLE satp)
  ├─ sfence.vma
  ├─ trap::init_secondary()              — stvec → trap_vector
  ├─ csrs sstatus (enable S-mode interrupts)
  └─ loop { schedule(); wfi }
```

The init binary name is read from the DTB `chosen/bootargs` property (`init=<name>`), defaulting to `"policy_test"` if absent or unparseable.

---

## 4. Memory Layout

The QEMU `virt` machine provides 128 MB of RAM starting at `0x8000_0000`. The kernel is loaded by OpenSBI at `0x8020_0000`.

```
Physical Address     Size        Contents
─────────────────────────────────────────────────────────────────────
0x0000_0000          (varies)    OpenSBI firmware (M-mode)
0x1000_0000          4 KB        UART MMIO (NS16550A)
0x1000_1000–         8 × 4 KB   VirtIO MMIO slots (blk, net, …)
0x1000_8FFF

0x8000_0000          2 MB        OpenSBI runtime
0x8020_0000          (varies)    Kernel entry: .text, .rodata, .data, .bss
                                 (linker-defined sections)
_stack_bottom–       64 KB       Boot hart kernel stack
_stack_top           (fixed)     Secondary harts: _stack_top - (hart_id × 8192)

0x803F_B000          16 MB       Kernel heap (_heap_start)
                                 linked_list_allocator LockedHeap
                                 Backs Box<Stack>, Box<PageTable>, Vec, etc.

0x813F_B000          ~110 MB     Free physical memory (_free_mem_start)
                                 Managed by buddy page allocator
                                 Source for alloc_page() calls

0x8800_0000          —           RAM end (QEMU virt 128 MB limit)

0x8900_0000–         3 × 4 KB   Simulated NES doorbell MMIO
0x8900_2FFF                     (backed by physical pages from buddy allocator)

─── User Virtual Address Space (per-process Sv39 page table) ─────────
0x4000_0000          4 KB        ELF PT_LOAD base (user binary segments)
0x4000_1000          4 KB        Guard page (unmapped — stack underflow trap)
0x4000_2000          4 KB        User stack page (READ|WRITE|USER)
0x4000_3000          —           Initial sp value (stack_top)

0x4010_0000–         6 × 16 KB  VMO pages for neural_test / policy_test
0x4015_FFFF                      handles 5–10, 4 pages each

0x5000_0000+         dynamic     SGF VMO allocations (dynamic phys pages)
```

All kernel virtual addresses are identity-mapped (virtual == physical). User address spaces are isolated via separate Sv39 page tables — the kernel's mappings are copied into each process page table so that trap handlers and syscall paths can execute without a page table switch.

---

## 5. Capability Security Model

User-space programs cannot access memory, invoke hardware, or communicate with other processes by default. They must present a **Handle** to the kernel.

### The Handle Model

A `Handle` is an entry in a process-local **Handle Table** (max 64 entries). Each handle carries:
- `object_type: ObjectType` — what kind of kernel object this is
- `object_ptr: usize` — pointer to the kernel-managed object
- `rights: Rights` — bitflags (`READ`, `WRITE`, `EXECUTE`, `DUPLICATE`, `TRANSFER`)

Handles are integers returned to user space. They cannot be forged because the kernel, not user space, manages the handle table. A process that receives a handle through IPC gets only the rights explicitly granted — not the sender's full rights.

### ObjectType Variants

| Variant | Description |
|---|---|
| `None` | Placeholder / invalid |
| `Process` | Isolated address-space container |
| `Thread` | Schedulable execution unit |
| `Channel` | IPC message ring between processes |
| `VirtualMemoryObject` | A physical-memory-backed region mapped into a process |
| `TaskGraph` | A DAG of NES compute nodes |
| `DeviceQueue` | Hardware accelerator command ring (CPU/GPU/NPU) |
| `GraphNode` | A node in the Semantic Graph Filesystem |
| `AgentProcess` | A kernel-tracked AI agent (Phase 9) |
| `AgentChannel` | An IPC channel owned by an agent (Phase 9) |

### Capability Flow

1. Process A creates a Channel; it receives two endpoint handles with full rights.
2. Process A sends one endpoint handle to Process B via `SYS_CHANNEL_SEND`.
3. The kernel removes that handle from Process A's table and inserts it into Process B's table.
4. Process B can now exchange messages with Process A, but cannot read any other memory belonging to Process A.

---

## 6. SMP Architecture

VeridianOS runs on 4 RISC-V harts simultaneously. Hart 0 is the boot hart; harts 1–3 are secondary harts started during boot via the SBI HSM extension.

### Hart Startup

```rust
// kernel/src/main.rs — smp::init()
for hart_id in 1..4 {
    sbi::sbi_hart_start(hart_id, _secondary_start as usize, 0);
}
```

Each secondary hart enters `ksecondary_main`, loads the kernel page table into `satp`, initializes its trap vector, enables S-mode interrupts, then enters the `schedule() / wfi` loop.

### Per-Hart Scheduler State

```rust
struct SchedulerState {
    threads: [Option<Thread>; 16],  // shared thread pool
    current_idx: [usize; 4],        // one current index per hart
}
```

- `current_idx[hart_id]` tracks which thread slot is currently running on that hart.
- `get_hart_id()` reads the `tp` register, which `boot.S` sets to the hart ID at entry.
- `ThreadState::Running(hart_id)` prevents a thread from being double-scheduled onto two harts simultaneously.

### Scheduling Algorithm

`try_schedule()` runs on each hart independently:

1. Read `hart_id` from `tp`.
2. From `current_idx[hart_id]`, scan forward (round-robin) for the next `ThreadState::Ready` thread.
3. Transition the current thread from `Running(hart_id)` → `Ready`.
4. Transition the next thread from `Ready` → `Running(hart_id)`.
5. Update `current_idx[hart_id]`.
6. Switch page tables (`csrw satp`) and call `switch_context()` (assembly context switch).

If no ready thread is found, and the current thread is not `Running(hart_id)`, `schedule()` enables interrupts and executes `wfi` to wait for a timer tick or IPI.

### Timer-Driven Preemption

The S-mode timer interrupt fires periodically. Only hart 0's timer ISR drives distributed subsystems (Raft `raft_tick()`, DKCP processing). All harts respond to their own timer interrupts for preemptive scheduling.

### Secondary Hart Stack Layout

Each secondary hart uses a slice of the boot stack:

```
_stack_top - (hart_id × 8192)
```

Hart 0 uses the full stack top. Hart 1 starts 8 KB below, hart 2 another 8 KB below that, and hart 3 another 8 KB below that.

---

## 7. Neural Execution Subsystem (NES)

The NES replaces the traditional scheduler's concept of CPU time slices with **graph-based compute dispatch**. User processes submit `TaskGraph` DAGs; the kernel resolves dependencies, selects optimal devices, and dispatches nodes to hardware queues.

### Task Graph Model

A `TaskGraph` is a static-pool DAG (no heap allocation required at submission time) of `TaskNode` entries. Each node carries:

- `op_type: OpType` — one of `GEMM`, `Convolution`, `VectorAdd`, `Activation`, `LayerNorm`, `Softmax`
- `execution_target: DeviceType` — `Cpu`, `Gpu`, `Npu`, or `Auto` (kernel decides)
- `inputs / outputs: [TensorDescriptor]` — VMO handles + offsets + shape metadata
- `dependency_count` / `remaining_dependencies` — predecessor tracking for DAG resolution

### Phase 10: Self-Improving Policy

The global `POLICY_STATS` (spinlock-protected) is a `6 × 3` matrix of EMA-smoothed latency priors:

```rust
pub struct PolicyStats {
    pub cumulative_ticks:         [[u64; 3]; 6],  // raw tick accumulator
    pub completion_counts:        [[u64; 3]; 6],  // completions per (op, device)
    pub predicted_ticks_per_byte: [[f32; 3]; 6],  // EMA estimate (α = 0.2)
    pub exploration_rate:         f32,             // ε for ε-greedy
}
```

Matrix layout (row = op_type − 1, column = device as usize):

```
              CPU      GPU      NPU
GEMM      [ 15.0,    3.0,    0.8 ]   ← NPU fastest (matrix accelerator)
Conv      [ 18.0,    4.5,    1.0 ]   ← NPU fastest
VectorAdd [  2.0,    0.4,    8.0 ]   ← GPU fastest (SIMD parallelism)
Activation [  1.5,    0.3,    6.0 ]  ← GPU fastest
LayerNorm  [  3.0,    0.8,    4.0 ]  ← GPU fastest
Softmax    [  5.0,    1.2,    5.0 ]  ← GPU fastest
```

**Device selection** (`select_optimal_device`): reads `rdtime` as a cheap pseudo-random seed. If `(rdtime % 1000) / 1000.0 < ε`, picks a random device (explore). Otherwise, computes `predicted_exec + queue_wait_cost` for each device and returns the argmin (exploit). Queue depth penalty prevents hot-path over-subscription.

**EMA update** (`execute_node`): after each node completes, `elapsed / output_size_bytes` is folded in: `new = 0.8 × old + 0.2 × sample`. This smooths scheduler jitter while tracking genuine workload shifts.

**`DeviceType::Auto`** nodes are resolved at graph submission (root nodes) and again at dependency completion (interior nodes), so every Auto node benefits from the most current priors.

### NES Syscalls

| Syscall | Number | Description |
|---|---|---|
| `SYS_GRAPH_CREATE` | 50 | Allocate a new TaskGraph; return a Handle |
| `SYS_GRAPH_ADD_NODE` | 51 | Append a TaskNode with op type, device target, VMO handles, predecessor IDs |
| `SYS_GRAPH_SUBMIT` | 52 | Validate topology (DFS cycle check), translate VMO → phys, enqueue root nodes |
| `SYS_GRAPH_WAIT` | 53 | Block-poll until all nodes complete or timeout |
| `SYS_POLICY_CONFIGURE` | 80 | `op=0` GET_STATS (copy 6×3 f32 matrix to user), `op=1` SET_EXPLORATION (set ε), `op=2` RESET_STATS |

---

## 8. Semantic Memory and Graph Storage

VeridianOS replaces the Unix hierarchical byte-file model with a **Semantic Knowledge Graph**:

- **Objects (Nodes)**: Entities such as documents, images, contacts, or conversation histories. Each node has a type, a property blob backed by a VMO, and a capability handle.
- **Relationships (Edges)**: Directed, labeled connections between nodes (e.g., `Document A` → *is_invoice_for* → `Company B`).
- **Retrieval**: Intent-based queries matched via `SYS_GRAPH_QUERY`; node types and property predicates are evaluated in-kernel.

VMO allocations for SGF nodes start at `0x5000_0000+` in user virtual address space and are mapped via `alloc_page()` from the buddy allocator.

`next_stack_va` is a per-`Process` field (not a global constant) — it advances by one page per `alloc_stack()` call, supporting multiple stacks per process and clean guard page placement.

### Semantic Graph Syscalls

| Syscall | Number | Description |
|---|---|---|
| `SYS_NODE_CREATE` | 60 | Allocate a typed graph node; return capability handle |
| `SYS_EDGE_ADD` | 61 | Add a directed, labeled edge between two node capabilities |
| `SYS_NODE_WRITE` | 62 | Write property data to a node's VMO backing store |
| `SYS_GRAPH_QUERY` | 63 | Match nodes by type and property predicates |
| `SYS_NODE_DELETE` | 64 | Delete a node and reclaim its VMO physical pages |

---

## 9. Agent Runtime

Phase 9 introduced first-class AI Agent abstractions tracked in kernel space.

### AgentRecord

```rust
pub struct AgentRecord {
    pub id:         AgentId,            // u32, monotonically allocated
    pub parent_id:  AgentId,            // 0 = root/kernel
    pub state:      AgentState,         // Idle | Running | WaitingForMessage | Dead
    pub intent:     [u8; 32],           // 32-byte fixed-size intent label
    pub pid:        usize,              // owning process PID
    pub valid:      bool,
    pub enclave_id: Option<u8>,         // Phase 12: Some(id) when in a TEE enclave
}
```

`enclave_id: Option<u8>` is `Some(id)` when the agent's workload has been placed inside a hardware-attested TEE enclave managed by the M-mode monitor. `None` means the agent runs in ordinary kernel-managed memory.

Up to 16 agents are stored in a static `AgentPool` (no heap per agent record). Up to 16 IPC channels are stored in a static `CHANNELS` pool.

### Agent Syscalls

| Syscall | Number | Description |
|---|---|---|
| `SYS_AGENT_SPAWN` | 70 | Create an AgentRecord with parent ID and intent string; return AgentId |
| `SYS_CHANNEL_CREATE` | 71 | Allocate an IPC channel; install into caller's handle table; return handle ID |
| `SYS_CHANNEL_SEND` | 72 | Write up to 512 bytes into a channel ring buffer |
| `SYS_CHANNEL_RECV` | 73 | Read from a channel; blocks via `block_current_thread()` if empty |
| `SYS_AGENT_STATUS` | 74 | Query an agent's `AgentState` (Idle/Running/WaitingForMessage/Dead) |

---

## 10. User-Space Exception Delivery

Phase 11 added a mechanism for user-space processes to register exception handlers for page faults, enabling user-level fault recovery (e.g., copy-on-write, demand paging stubs).

### Mechanism

Each `Process` carries an `exception_handler: usize` field (virtual address of the user handler, or 0 if unregistered). Each `Thread` carries a `saved_user_context: Option<TrapFrame>` field.

When a page fault occurs from U-mode (scause 12 = instruction page fault, 13 = load page fault, 15 = store/AMO page fault):

1. The trap handler checks whether the faulting process has a non-zero `exception_handler`.
2. If registered: the current `TrapFrame` is saved into `thread.saved_user_context`. The trap frame is rewritten to redirect execution to the handler VA, with fault address in a0.
3. The handler runs in U-mode and calls `SYS_EXCEPTION_RETURN` when done.
4. `SYS_EXCEPTION_RETURN` restores the saved `TrapFrame` from `thread.saved_user_context`, resuming the original instruction.
5. If no handler is registered, the process receives a fatal fault (as before).

### Exception Syscalls

| Syscall | Number | Description |
|---|---|---|
| `SYS_REGISTER_EXCEPTION_HANDLER` | 110 | Register a user VA as the fault handler; stored in `Process.exception_handler` |
| `SYS_EXCEPTION_RETURN` | 111 | Restore the saved `TrapFrame` from `Thread.saved_user_context` and resume |

---

## 11. Distributed Coherence — Phase 11

Phase 11 implements multi-kernel coherence across QEMU domains. All components are fully wired (no stubs). In the current single-QEMU configuration, communication uses a loopback transport; two-QEMU scenarios require the `virtio-net-device` present in `.cargo/config.toml`.

### Subsystems

**Raft Consensus Engine** (`kernel/src/dist/raft.rs`)

Implements the Raft protocol (Ongaro & Ousterhout, ATC'14) for replicating SGF mutations across kernel domains.

- `RaftState`: `current_term`, `voted_for`, `commit_index`, `last_applied`, `role` (`Follower` | `Candidate` | `Leader`), `election_tick`, `log: RaftLog` (128-entry static array), `next_index[MAX_DOMAINS]`, `match_index[MAX_DOMAINS]`.
- `raft_init()`: Starts in `Leader` role at `term=1` (deterministic for single-node loopback).
- `raft_tick()`: Called from hart 0's timer ISR. Followers decrement `election_tick`; on timeout, start election (increment term, vote for self, broadcast `RaftRequestVote`). Leaders send periodic `AppendEntries` heartbeats every `HEARTBEAT_INTERVAL = 30` ticks.
- `append_entry(mutation)`: Leader appends an SGF mutation to the log, advances `commit_index`, broadcasts `AppendEntries`.
- `raft_status(buf, len)`: Writes 32-byte status (role, log_len, current_term, commit_index, last_applied) to user buffer for `SYS_SGF_RAFT_STATUS`.
- Single-node quorum: the node votes for itself and immediately becomes Leader within the same `raft_tick()` call.

**DKCP Ring** (`kernel/src/dist/ring.rs`)

Lock-free SPSC ring buffer for inter-domain messages:
- 256 slots × 64 bytes per slot = 16 KB ring body.
- Atomic `head` (consumer) and `tail` (producer) using `Ordering::Acquire`/`Release`.
- `volatile` reads/writes prevent compiler reordering.
- Page-aligned (`repr(align(4096))`), `Send + Sync`.

**DCTP — Distributed Capability Transfer Protocol** (`kernel/src/dist/dctp.rs`)

Three operations for cross-domain capability sharing:
- `cap_export(handle_id, target_domain)`: Reads the handle from `PROCESS_TABLE`, derives a 128-bit UID (rdtime || domain_id || handle_id || monotonic_seq), registers in `DIST_CAP_TABLE`, sends `CapExportRequest` via loopback ring.
- `cap_import(uid_ptr, uid_len, src_domain)`: Searches `DIST_CAP_TABLE` by 8-byte UID prefix, installs a shadow `Handle` into the calling process's handle table.
- `cap_revoke(handle_id, target_domain)`: Bumps the epoch on the `DistributedCapability` entry (invalidates all remote shadows), sends `CapRevokeNotify`.

**Remote NES Dispatch** (`kernel/src/dist/nes_dist.rs`)

Dispatches NES `TaskNode` executions to remote kernel domains via the DKCP ring, using a `TicketPool` for in-flight tracking.

### Phase 11 Syscalls

| Syscall | Number | Description |
|---|---|---|
| `SYS_DOMAIN_JOIN` | 90 | Join a distributed kernel domain cluster |
| `SYS_DOMAIN_LIST` | 91 | List all live kernel domains |
| `SYS_DOMAIN_STATUS` | 92 | Query status of a specific domain |
| `SYS_GRAPH_DISPATCH_REMOTE` | 93 | Dispatch a NES node to a remote domain |
| `SYS_GRAPH_WAIT_REMOTE` | 94 | Wait for a remote node to complete |
| `SYS_GRAPH_ABORT_REMOTE` | 95 | Abort a remote node |
| `SYS_CAP_EXPORT` | 96 | Export a local capability; returns 64-bit UID token |
| `SYS_CAP_IMPORT` | 97 | Import a capability from a UID token |
| `SYS_CAP_REVOKE_REMOTE` | 98 | Revoke a distributed capability globally |
| `SYS_SGF_REPLICATE_ENABLE` | 99 | Enable Raft replication for an SGF node |
| `SYS_SGF_REPLICATE_QUERY` | 100 | Query replication status of an SGF node |
| `SYS_SGF_RAFT_STATUS` | 101 | Read Raft state (role, term, log_len, commit_index) |

---

## 12. Phase 12 TEE Monitor

Phase 12 adds hardware-attested AI agent enclaves using RISC-V Physical Memory Protection (PMP).

### Architecture

```
User Process (U-mode)
    │  SYS_ENCLAVE_* syscall (ecall, a7=120..123)
Kernel (S-mode) — kernel/src/enclave/mod.rs
    │  SBI ecall (ecall, a7=0x08424B45)
M-mode Monitor — monitor/ crate
    │  PMP configuration + SHA-256 measurement + HMAC attestation
Hardware (RISC-V PMP registers)
```

The kernel cannot bypass PMP entries set by the monitor. Once an enclave region is locked, a compromised S-mode kernel cannot read, write, or execute enclave memory. The HMAC device key never leaves M-mode memory.

### M-mode Monitor

The `monitor/` crate is a separate M-mode binary that implements SBI extension **EID `0x08424B45`** ("BKE"):

| FID | Function | Description |
|---|---|---|
| 0 | `ENCLAVE_CREATE` | Allocate an enclave slot (up to 8 slots, indices 0–7); configure a PMP entry in NAPOT mode over `[phys_addr, phys_addr + size)`; compute SHA-256 measurement of enclave memory at creation time |
| 1 | `ENCLAVE_ENTER` | Lock the PMP entry (deny S-mode access); transfer execution to the enclave entry point in U-mode via `mret` |
| 2 | `ENCLAVE_EXIT` | Unlock the PMP entry; restore kernel context; return via `mret` to S-mode at the instruction after `SYS_ENCLAVE_ENTER` |
| 3 | `ENCLAVE_ATTEST` | Generate a 73-byte attestation report at the specified physical address |

### Attestation Report Layout (73 bytes)

```
Offset  Bytes  Field
     0      1  enclave_id (u8)
     1      8  phys_start (little-endian u64)
     9      8  size (little-endian u64)
    17     32  SHA-256 measurement of enclave memory (computed at create time)
    49     24  HMAC-SHA-256 of (device_key || measurement), truncated to 24 bytes
```

A remote verifier extracts the measurement, verifies the HMAC with the device public key, and compares against a known-good reference image hash to confirm that genuine, unmodified code is running on a legitimate VeridianOS device.

### S-mode Kernel Integration

`kernel/src/enclave/mod.rs` provides the syscall handler layer. Before forwarding to the monitor, it validates:
- `size` is a non-zero power of two (NAPOT requirement).
- `phys_addr` is aligned to `size`.
- `entry_pa` is within `[phys_addr, phys_addr + size)`.
- For `SYS_ENCLAVE_ATTEST`: `report_buf_ptr` is a writeable user-space buffer (validated via `Process::validate_user_buffer`).

SBI error codes from the monitor propagate directly to user space as negative `isize` values.

### Phase 12 Syscalls

| Syscall | Number | Description |
|---|---|---|
| `SYS_ENCLAVE_CREATE` | 120 | Create enclave: phys_addr (NAPOT-aligned), size (power-of-two), entry_pa; returns enclave_id (0–7) |
| `SYS_ENCLAVE_ENTER` | 121 | Enter enclave: PMP-locks region, transfers to enclave U-mode; returns after `SYS_ENCLAVE_EXIT` |
| `SYS_ENCLAVE_EXIT` | 122 | Exit from inside enclave: unlocks PMP, restores kernel context |
| `SYS_ENCLAVE_ATTEST` | 123 | Write 73-byte attestation report to user buffer |

---

## 13. Process and Thread Internals

### Process Struct

```rust
pub struct Process {
    pub pid:               usize,
    pub state:             ProcessState,     // Ready | Running | Blocked | Exited(i32)
    pub page_table:        PageTable,        // Sv39 page table (kernel mappings copied in)
    pub handle_table:      HandleTable,      // Up to 64 capability handles
    pub next_stack_va:     usize,            // Advances per alloc_stack() call
    pub exception_handler: usize,            // U-mode fault handler VA (0 = none)
}
```

`PROCESS_TABLE` is a `Mutex<[Option<Process>; 16]>` — up to 16 concurrent processes. The previous single `CURRENT_PROCESS` global is gone; `with_current_process(f)` looks up the calling hart's current PID via `thread::current_pid()` and finds the matching entry in the table.

### Thread Struct

```rust
pub struct Thread {
    pub tid:                  usize,
    pub pid:                  usize,
    pub state:                ThreadState,              // Ready | Running(hart_id) | Blocked | Exited
    pub context:              ThreadContext,             // ra, sp, s0–s11
    pub stack:                Option<Box<Stack>>,       // 16 KB heap-allocated kernel stack
    pub satp:                 usize,                    // address space for this thread
    pub user_entry:           Option<usize>,            // U-mode ELF entry VA (user threads only)
    pub user_sp:              Option<usize>,            // U-mode initial stack pointer
    pub saved_user_context:   Option<TrapFrame>,        // saved on exception handler dispatch
}
```

Up to 16 threads are stored in `SCHEDULER` (a `Mutex<SchedulerState>`). Kernel threads (`spawn_thread`) use the kernel page table SATP. User threads (`spawn_user_thread`) use the spawning process's page table SATP.

### Stack Layout

User processes get a deterministic stack layout (ASLR disabled pending page-table audit):

```
0x4000_1000  guard page (unmapped — traps stack underflow)
0x4000_2000  user stack page (READ | WRITE | USER)
0x4000_3000  initial sp (stack_top)
```

`Process.next_stack_va` starts at `0x4000_0000` and advances by `PAGE_SIZE` per call, making the layout repeatable and the guard page position predictable.

---

## 14. Build and Runtime Environment

The kernel targets `riscv64gc-unknown-none-elf`. QEMU arguments are defined in `.cargo/config.toml`:

```
qemu-system-riscv64
  -machine virt
  -nographic
  -serial mon:stdio
  -bios default            # OpenSBI
  -smp 4                   # 4 hardware threads (harts)
  -device virtio-net-device
  -netdev user,id=net0
  -drive id=hd0,file=disk.img,format=raw,if=none
  -device virtio-blk-device,drive=hd0
```

Key crate dependencies:
- `linked_list_allocator` — kernel heap (`#[global_allocator]`)
- `spin` — `Mutex` and `RwLock` for all shared state
- `riscv` — CSR access
- `bitflags` — `Rights` capability bitfields

---

## 15. Implementation Status

| Phase | Name | Status |
|---|---|---|
| Phase 1 | Bootable RISC-V microkernel (S-mode, UART, trap vector) | Complete |
| Phase 2 | Capability system (Handle, Rights, HandleTable, ObjectType) | Complete |
| Phase 3 | Page allocator (buddy) + Sv39 virtual memory + identity mapping | Complete |
| Phase 4 | Preemptive round-robin thread scheduler (timer ISR, context switch) | Complete |
| Phase 5 | VirtIO block driver + ustar InitRAMFS loader | Complete |
| Phase 6 | ELF64 loader + U-mode transition (sret, satp swap, user stack) | Complete |
| Phase 7 | Neural Execution Subsystem — TaskGraph DAG dispatch to CPU/GPU/NPU queues | Complete |
| Phase 8 | Semantic Knowledge Graph Filesystem (node/edge/VMO store) | Complete |
| Phase 9 | Agent Runtime (AgentPool, channel IPC, SYS_AGENT_* syscalls) | Complete |
| Phase 10 | Self-improving kernel policies (PolicyStats 6×3, ε-greedy, EMA α=0.2) | Complete |
| Phase 11 | Distributed multi-kernel coherence (Raft, DCTP, DKCP ring, remote NES) | Complete |
| Phase 12 | Hardware-attested TEE enclaves (M-mode monitor, PMP, SHA-256, HMAC) | In Progress |

Phase 12 kernel-side syscall handlers (120–123) and the S-mode → M-mode SBI bridge are implemented. The `monitor/` M-mode binary with full PMP management and attestation is in active development. Virtual-to-physical address translation for the attestation report buffer (`TODO(phase12)` in `sys_enclave_attest`) is pending.
