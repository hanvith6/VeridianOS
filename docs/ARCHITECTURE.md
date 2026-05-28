# VeridianOS Architecture Specification

This document details the software architecture of **VeridianOS** — a next-generation microkernel built in Rust targeting RISC-V 64-bit processors.

---

## 1. Design Philosophy

VeridianOS departs from traditional Unix-style monolithic designs. It is built on four core pillars:

1. **Microkernel Architecture**: Move device drivers, filesystems, and network stacks out of Supervisor mode into sandboxed User-space processes. This reduces the Trusted Computing Base (TCB) to a few thousand lines of code.
2. **Capability-Based Security**: Eliminate ambient authority (e.g., file permissions linked to User IDs like `root`). Access to resources is governed exclusively by explicit, unforgeable capability tokens (Handles).
3. **AI-Native Orchestration**: The kernel is designed around heterogeneous hardware (CPU + GPU/NPU). Instead of treating AI workloads as high-level application processes, they are treated as primary kernel threads scheduled directly onto accelerator hardware with strict latency guarantees.
4. **Online Policy Learning**: The scheduler is not static. It observes real hardware timing data and continuously updates a learned latency model using exponential moving averages, routing future work to the historically fastest device for each operation type.

---

## 2. Kernel Privilege Separation

RISC-V processors provide three hardware privilege levels. VeridianOS uses them as follows:

| Privilege Mode | Name | Software Components |
|---|---|---|
| **M-mode** (Machine) | Firmware / Bootloader | OpenSBI (Supervisor Binary Interface) — handles low-level hardware initialization and basic console I/O. |
| **S-mode** (Supervisor) | Microkernel | Veridian Kernel — page table management, capability access checks, basic thread scheduling, IPC message routing, NES, and the self-improving policy engine. |
| **U-mode** (User) | Drivers, OS Services, Apps | Device drivers (virtio), semantic file graph service, neural network runtime libraries, user apps, and AI agents. |

---

## 3. Capability Security Model

In VeridianOS, user-space programs cannot access memory, write to hardware, or call services by default. They must present a **Handle** to the kernel.

### The Handle Model
* A **Handle** is a process-local reference to a **Kernel Object** (such as a Process, Thread, Channel, TaskGraph, GraphNode, or Virtual Memory Object).
* Each process maintains a private **Handle Table** managed by the kernel.
* Handles are associated with **Rights** (bitflags like `READ`, `WRITE`, `EXECUTE`, `DUPLICATE`, `TRANSFER`).

### Object Creation and Flow
1. Process A creates a Channel object. It receives two channel endpoints (Handles) with full rights.
2. Process A wants to communicate with Process B. It uses the `sys_channel_write` syscall to send one of the endpoint handles to Process B.
3. The kernel removes the handle from Process A's Handle Table and inserts it into Process B's Handle Table.
4. Process B can now write messages to Process A using its new handle, but it cannot access any other memory belonging to Process A.

---

## 4. Neural Execution Subsystem (NES)

Traditional schedulers allocate CPU execution time slices to threads. VeridianOS schedules **Graph Execution Nodes** onto a collection of heterogeneous compute units.

### 4.1 Heterogeneous Scheduling Matrix

```
               ┌────────────────────────┐
               │    Neural Scheduler    │
               └───────────┬────────────┘
                           │
         ┌─────────────────┼─────────────────┐
         ▼                 ▼                 ▼
  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
  │     CPU     │   │     GPU     │   │     NPU     │
  │ Sequential  │   │  Parallel   │   │  Matrix /   │
  │ Control Flow│   │ Workloads   │   │ Tensor Ops  │
  └─────────────┘   └─────────────┘   └─────────────┘
```

The scheduler parses graph execution dependencies (e.g., an LLM layer inference node waiting on an input token vector node) and streams them dynamically to the most efficient available hardware queue, minimizing CPU-GPU/NPU memory copies.

### 4.2 TaskGraph and TaskNode

A `TaskGraph` is a Directed Acyclic Graph (DAG) of `TaskNode` entries stored in a static pool (no heap allocation in S-mode). Each node carries:

- `op_type: OpType` — one of `GEMM`, `Convolution`, `VectorAdd`, `Activation`, `LayerNorm`, `Softmax`
- `execution_target: DeviceType` — `Cpu`, `Gpu`, `Npu`, or `Auto`
- `inputs / outputs: [TensorDescriptor]` — VMO handles + offsets + shape metadata
- `dependency_count` and `remaining_dependencies` — predecessor count tracking for dependency resolution

### 4.3 Key Syscalls

| Syscall ID | Name | Description |
|---|---|---|
| `50` | `SYS_GRAPH_CREATE` | Allocate a new TaskGraph in the kernel pool; return a Handle |
| `51` | `SYS_GRAPH_ADD_NODE` | Append a TaskNode with op type, device target, VMO handles, and predecessor IDs |
| `52` | `SYS_GRAPH_SUBMIT` | Validate topology (DFS cycle check), translate VMO→phys, enqueue root nodes |
| `53` | `SYS_GRAPH_WAIT` | Block polling until all nodes complete or timeout expires |

---

## 5. Phase 10: Self-Improving Kernel Scheduler

> *"The best optimizer is one that has seen the data."* — adapted from online learning theory.

Phase 10 introduces an **online reinforcement learning loop** directly inside the kernel scheduler. The kernel observes real hardware timer tick counts for each executed operation and uses this signal to improve future routing decisions — completely transparently to user-space applications.

### 5.1 PolicyStats Structure

The global `POLICY_STATS` object (protected by a spinlock) is a `6×3` matrix of learned latency priors:

```rust
pub struct PolicyStats {
    // Accumulated raw ticks per (op, device) pair
    pub cumulative_ticks:         [[u64; 3]; 6],
    // Number of completed executions per pair
    pub completion_counts:        [[u64; 3]; 6],
    // EMA-smoothed ticks-per-byte estimate
    pub predicted_ticks_per_byte: [[f32; 3]; 6],
    // ε-greedy exploration rate (0.0 = pure greedy, 1.0 = fully random)
    pub exploration_rate:         f32,
}
```

**Matrix layout** (row = `op_type - 1`, column = `device as usize`):

```
                  CPU     GPU     NPU
  GEMM      [  15.0,    3.0,    0.8  ]   ← NPU fastest (matrix math accelerator)
  Conv      [  18.0,    4.5,    1.0  ]   ← NPU fastest
  VectorAdd [   2.0,    0.4,    8.0  ]   ← GPU fastest (SIMD parallelism)
  Activation [  1.5,    0.3,    6.0  ]   ← GPU fastest
  LayerNorm  [  3.0,    0.8,    4.0  ]   ← GPU fastest
  Softmax    [  5.0,    1.2,    5.0  ]   ← GPU fastest
```

These priors are seeded from domain knowledge (NPU excels at matrix math; GPU excels at elementwise ops) and then refined by the EMA feedback loop.

### 5.2 Epsilon-Greedy Device Selection (`select_optimal_device`)

```
select_optimal_device(op: OpType, size_bytes: usize) → DeviceType
│
├─ Read hardware timer via `rdtime` (RISC-V CSR)
│
├─ rand_val = (rdtime % 1000) / 1000.0
│
├─ if rand_val < exploration_rate (ε):
│    └─ Pick a random device (CPU / GPU / NPU)  ← EXPLORE
│
└─ else:
     For each device ∈ {CPU, GPU, NPU}:
       predicted_exec  = size_bytes × ticks_per_byte[op][device]
       queue_wait_cost = Σ (pending_job_size × ticks_per_byte[q_op][device])
       total_cost      = predicted_exec + queue_wait_cost
     └─ Return argmin(total_cost)                ← EXPLOIT
```

The queue-depth penalty ensures that a device with low per-byte cost but a long pending backlog is not over-subscribed, providing implicit load balancing across the three hardware queues.

### 5.3 EMA Feedback Loop (`execute_node`)

After every node completes, the simulator calls `PolicyStats::update()`:

```
execute_node(desc, device):
  start_ticks  ← rdtime()          // hardware timer snapshot
  
  ... execute workload (GEMM / VectorAdd / Activation / etc.) ...
  
  end_ticks    ← rdtime()
  elapsed      = end_ticks − start_ticks
  
  sample       = elapsed / output_size_bytes
  old_pred     = predicted_ticks_per_byte[op][device]
  
  // Exponential Moving Average: α = 0.2
  new_pred     = 0.8 × old_pred + 0.2 × sample
  
  predicted_ticks_per_byte[op][device] ← new_pred
```

The EMA formula `new = (1−α)×old + α×sample` with α = 0.2 gives 80% weight to history and 20% to each new observation. This smooths transient noise (cache effects, scheduler jitter) while still tracking genuine workload changes.

### 5.4 DeviceType::Auto Resolution

`DeviceType::Auto = 3` is the new device type value that signals "let the kernel decide." Resolution happens at two points:

1. **Graph submission** (`sys_graph_submit`): Root nodes with `Auto` target are resolved before being enqueued.
2. **Dependency completion** (`complete_node`): When a successor node becomes `Ready`, if its target is `Auto`, `select_optimal_device()` is called again with the current policy state at that moment.

This means even mid-graph, every `Auto` node benefits from the most up-to-date learned priors.

### 5.5 SYS_POLICY_CONFIGURE (Syscall 80)

Userspace can inspect and configure the policy engine without recompiling the kernel:

```
sys_policy_configure(op: usize, arg1: usize, arg2: usize) → isize

  op = 0 (GET_STATS):
    Copies the 6×3 f32 ticks_per_byte matrix (72 bytes) to the
    user buffer at arg1. Returns 0 on success, -EINVAL if
    arg1 == 0 or arg2 < 72.

  op = 1 (SET_EXPLORATION):
    arg1 = f32::to_bits(new_epsilon)
    Sets exploration_rate ∈ [0.0, 1.0]. Setting to 0.0 enables
    pure greedy mode; setting to 1.0 forces full random exploration.

  op = 2 (RESET_STATS):
    Resets the entire PolicyStats to factory priors (the seeded
    domain-knowledge table). Useful for A/B testing and benchmarking.
```

### 5.6 End-to-End Self-Improvement Loop

```
┌─────────────────────────────────────────────────────────┐
│                  SELF-IMPROVING LOOP                    │
│                                                         │
│  User submits graph with DeviceType::Auto nodes         │
│           │                                             │
│           ▼                                             │
│  Kernel calls select_optimal_device()                   │
│    • reads POLICY_STATS (learned ticks/byte priors)     │
│    • ε-greedy: explore randomly OR exploit best device  │
│    • accounts for current queue depth (wait penalty)    │
│           │                                             │
│           ▼                                             │
│  Node dispatched to CPU / GPU / NPU queue               │
│           │                                             │
│           ▼                                             │
│  Worker thread dequeues and calls execute_node()        │
│    • rdtime() before execution                          │
│    • runs the actual compute (GEMM, VAdd, etc.)         │
│    • rdtime() after execution                           │
│    • POLICY_STATS.update() with elapsed ticks           │
│    • EMA: 80% history + 20% observation                 │
│           │                                             │
│           ▼                                             │
│  Next submission uses improved predictions              │
│     → converges toward empirically optimal routing      │
└─────────────────────────────────────────────────────────┘
```

---

## 6. Semantic Memory and Graph Storage

The Unix file system represents data as linear arrays of bytes located in a hierarchical tree of directory names.

VeridianOS stores data as a **Semantic Knowledge Graph**:
* **Objects**: Entities such as a document, an image, a contact, or a chat history.
* **Relationships**: Directed, labeled connections between entities (e.g., `Document A` -> *is invoice for* -> `Company B`).
* **Storage Engine**: High-performance key-value/graph database executing directly on bare-metal blocks.
* **Retrieval**: Intent-based natural language searches resolved via local on-device embedders and vector indexes running on the NPU.

### Semantic Graph Syscalls

| Syscall ID | Name | Description |
|---|---|---|
| `60` | `SYS_NODE_CREATE` | Allocate a typed graph node (Document, Blob, Agent, etc.) |
| `61` | `SYS_EDGE_ADD` | Add a directed, labeled edge between two node capabilities |
| `62` | `SYS_NODE_WRITE` | Write property data to a node's VMO backing store |
| `63` | `SYS_GRAPH_QUERY` | Match nodes by type and property predicates |

---

## 7. Agent Runtime

Phase 9 introduced **first-class AI Agent abstractions** in kernel-space.

### Agent System Calls

| Syscall ID | Name | Description |
|---|---|---|
| `70` | `SYS_AGENT_SPAWN` | Create an AgentRecord with parent ID and intent string |
| `71` | `SYS_AGENT_CHANNEL_CREATE` | Create an IPC channel owned by an agent |
| `72` | `SYS_AGENT_SEND` | Send a 64-byte structured message via an agent channel |
| `73` | `SYS_AGENT_RECV` | Receive a message and extract the sender AgentId |
| `74` | `SYS_AGENT_STATUS` | Query an agent's state machine (Idle/Running/Waiting/Dead) |

---

## 8. Implementation Status

| Phase | Name | Status |
|---|---|---|
| Phase 1 | Bootable RISC-V microkernel | ✅ Complete |
| Phase 2 | Capability System Foundation | ✅ Complete |
| Phase 3 | Page Allocator & Sv39 VM | ✅ Complete |
| Phase 4 | Preemptive Thread Scheduler | ✅ Complete |
| Phase 5 | VirtIO Block Driver & InitRAMFS | ✅ Complete |
| Phase 6 | ELF Loader & User Mode Transition | ✅ Complete |
| Phase 7 | Neural Execution Subsystem (NES) | ✅ Complete |
| Phase 8 | Semantic Knowledge Graph Filesystem | ✅ Complete |
| Phase 9 | Agent Runtime | ✅ Complete |
| Phase 10 | Self-Improving Kernel Policies | ✅ Complete |
