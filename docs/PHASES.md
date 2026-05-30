# VeridianOS — Phase Implementation Reference

This document provides a complete, structured description of every implementation phase of **VeridianOS**. Each phase builds on the previous, incrementally adding capabilities until the OS can autonomously schedule heterogeneous AI workloads with a self-improving policy engine.

---

## Phase Summary Table

| Phase | Name | Status | Verification Program |
|-------|------|--------|----------------------|
| 1 | Bootable RISC-V Microkernel | ✅ Complete | _(boot log only)_ |
| 2 | Capability System Foundation | ✅ Complete | _(integrated into kernel)_ |
| 3 | Page Allocator & Sv39 VM | ✅ Complete | `hello` |
| 4 | Preemptive Thread Scheduler | ✅ Complete | `hello` |
| 5 | VirtIO Block Driver & InitRAMFS | ✅ Complete | `hello` |
| 6 | ELF Loader & User Mode Transition | ✅ Complete | `hello` |
| 7 | Neural Execution Subsystem (NES) | ✅ Complete | `neural_test` |
| 8 | Semantic Knowledge Graph Filesystem | ✅ Complete | `semantic_test` |
| 9 | Agent Runtime | ✅ Complete | `agent_test` |
| 10 | Self-Improving Kernel Policies | ✅ Complete | `policy_test` |
| 11 | Distributed Multi-Kernel Coherence | ✅ Complete | `policy_test` (syscalls 90–101) |
| 11.5 | SMP + User-Space Exception Delivery | ✅ Complete | `smp_test` |
| 12 | M-Mode TEE Security Monitor | 🔄 In Progress | _(enclave_test — pending)_ |

---

## Phase 1: Bootable RISC-V Microkernel

**Status**: ✅ Complete

### What Was Built
The absolute foundation of the OS: the ability to boot on a RISC-V 64-bit virtual machine, initialize the hardware environment, and print structured log output to the UART console.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `rust-toolchain.toml` | Pin nightly Rust with `riscv64gc-unknown-none-elf` target |
| `.cargo/config.toml` | Target specification and linker script registration |
| `kernel/src/link.ld` | Linker script placing `.text.boot` at `0x8020_0000` (OpenSBI entry) |
| `kernel/src/boot.S` | Assembly entry — sets up `sp` register, calls `_start_rust` |
| `kernel/src/main.rs` | Rust `#![no_std]` `#![no_main]` kernel entry point with `#[panic_handler]` |
| `kernel/src/uart.rs` | MMIO-backed 16550 UART driver with `print!` / `println!` macros |

### Key Syscalls Added
None (no user-space yet).

### What Was Proven Working
- QEMU `-machine virt` boots with OpenSBI, hands control to the kernel at `0x8020_0000`.
- Kernel prints `[BOOT] VeridianOS starting...` to the virtual UART.
- Rust panic handler catches illegal operations and prints a diagnostic message before halting.

---

## Phase 2: Capability System Foundation

**Status**: ✅ Complete

### What Was Built
The security core of VeridianOS: an unforgeable, kernel-managed capability token system. Replaced Unix-style UID/permission checks with a hardware-enforced, object-capability model.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/capability/mod.rs` | `ObjectType` enum, `Rights` bitflags, `Handle` struct, `HandleTable` with insert/get/remove |
| `kernel/src/syscall/mod.rs` | Trap handler for `ecall` (U-mode → S-mode), dispatches by `a7` register |
| `kernel/src/syscall/numbers.rs` | Canonical syscall number constants (`SYS_WRITE=1`, `SYS_EXIT=2`, etc.) |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `1` | `SYS_WRITE` | Print a UTF-8 string buffer to the UART console |
| `2` | `SYS_EXIT` | Terminate the current process with an exit status code |

### What Was Proven Working
- Kernel-mode capability table inserts and retrieves handles with correct rights enforcement.
- `ecall` from U-mode correctly traps into S-mode and dispatches the right handler.

---

## Phase 3: Page Allocator & Sv39 VM

**Status**: ✅ Complete

### What Was Built
Physical memory management via a Binary Buddy allocator, and virtual address space management via RISC-V Sv39 three-level page tables.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/memory/buddy.rs` | Binary Buddy allocator managing physical page frames |
| `kernel/src/memory/page_table.rs` | Sv39 `PageTable` with `map()`, `unmap()`, `get_entry_mut()` |
| `kernel/src/memory/vmo.rs` | `VirtualMemoryObject` — named, capability-secured physical memory regions |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `10` | `SYS_VMO_CREATE` | Allocate a new VMO of `size` bytes, return a Handle |
| `11` | `SYS_VMO_MAP` | Map a VMO into the calling process's address space |

### What Was Proven Working
- Physical pages allocated from buddy allocator, mapped to user virtual addresses.
- `satp` register switched to user page table on U-mode entry.
- Page faults on unmapped addresses are caught and produce a kernel diagnostic.

---

## Phase 4: Preemptive Thread Scheduler

**Status**: ✅ Complete

### What Was Built
A round-robin, preemptive multi-threaded scheduler using the RISC-V supervisor timer interrupt (`STIP`) and software-managed context switching.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/process/thread.rs` | `Thread` struct with saved register context (`TrapFrame`) |
| `kernel/src/process/scheduler.rs` | Round-robin queue, `schedule()`, and `yield_now()` |
| `kernel/src/trap.rs` | Unified trap handler for interrupts and exceptions |
| `kernel/src/timer.rs` | SBI `set_timer` call to program `stimecmp` |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `3` | `SYS_YIELD` | Voluntarily yield the CPU to the next runnable thread |

### What Was Proven Working
- Multiple S-mode worker threads scheduled concurrently.
- Timer preemption fires every ~10ms and triggers a context switch.
- TrapFrame save/restore correctly preserves all `x0`–`x31` registers across switches.

---

## Phase 5: VirtIO Block Driver & InitRAMFS

**Status**: ✅ Complete

### What Was Built
A VirtIO legacy block device driver and a POSIX-ustar TAR-format initial RAM filesystem loader, enabling the kernel to load user binaries from a disk image at boot.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/virtio/mod.rs` | MMIO slot scanner — probes `0x10001000`–`0x10008000` |
| `kernel/src/virtio/blk.rs` | Descriptor tables, available/used ring management, PFN guest page size |
| `kernel/src/fs/ramfs.rs` | ustar TAR parser — indexes files by name into a static table |
| `Makefile` | `disk.img` assembly: `tar` archive containing all user ELF binaries |

### Key Syscalls Added
None (filesystem is internal to the kernel boot sequence).

### What Was Proven Working
- Kernel scans MMIO range and identifies the VirtIO block device at `0x10001000`.
- Reads all 512-byte sectors from `disk.img` and parses the ustar archive header blocks.
- File names and sizes are indexed: `hello (13288 bytes)` found and ready to spawn.

---

## Phase 6: ELF Loader & User Mode Transition

**Status**: ✅ Complete

### What Was Built
An ELF-64 binary parser that maps loadable segments into a fresh user address space and transitions the processor to U-mode to execute the entry point.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/process/elf.rs` | ELF64 header parser, `PT_LOAD` segment mapper |
| `kernel/src/process/mod.rs` | `Process` struct, `spawn_process()`, user stack allocation |
| `kernel/src/trap.rs` | `sret` path that restores user `pc` and `sp`, drops to U-mode |
| `user_programs/hello/` | Minimal `#![no_std]` U-mode "Hello World" verification program |

### Key Syscalls Added
None new — `SYS_WRITE` and `SYS_EXIT` exercised by the `hello` program.

### What Was Proven Working
- Kernel parses ELF headers, maps all `PT_LOAD` segments to correct virtual addresses.
- U-mode process executes `SYS_WRITE` → UART output appears, then `SYS_EXIT` cleanly terminates.
- `[SUCCESS] VeridianOS Phase 6 fully verified!` appears in QEMU boot log.

---

## Phase 7: Neural Execution Subsystem (NES)

**Status**: ✅ Complete

### What Was Built
A complete kernel-resident DAG-based heterogeneous compute scheduler supporting CPU, GPU, and NPU device queues. Operations are represented as typed `TaskNode` entries connected by dependency edges in a `TaskGraph`.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/nes/types.rs` | `OpType`, `DeviceType`, `TensorDescriptor`, `NodeConfig` |
| `kernel/src/nes/graph.rs` | `TaskNode`, `TaskGraph`, static `GRAPH_POOL` (no heap) |
| `kernel/src/nes/queue.rs` | `HeterogeneousQueue` ring buffers for `CPU_QUEUE`, `GPU_QUEUE`, `NPU_QUEUE` |
| `kernel/src/nes/validator.rs` | DFS cycle checker and DAG topology validator |
| `kernel/src/nes/simulator.rs` | S-mode background worker threads simulating hardware execution |
| `kernel/src/nes/syscalls.rs` | `sys_graph_create`, `sys_graph_add_node`, `sys_graph_submit`, `sys_graph_wait` |
| `kernel/src/nes/mod.rs` | Module coordinator and `init()` |
| `user_programs/neural_test/` | Verification: GEMM → Activation → VectorAdd chain |

### Key Syscalls Added
| ID | Name |
|----|------|
| `50` | `SYS_GRAPH_CREATE` |
| `51` | `SYS_GRAPH_ADD_NODE` |
| `52` | `SYS_GRAPH_SUBMIT` |
| `53` | `SYS_GRAPH_WAIT` |

### What Was Proven Working
- 3-node DAG (`GEMM → Activation → VectorAdd`) submitted, topologically sorted (no cycles).
- VMO handles translated to physical addresses via Sv39 page table walk.
- NPU worker executes GEMM (128μs), CPU executes ReLU, GPU executes VectorAdd (16μs).
- Mathematical output verified: all 4096 f32 elements equal exactly `131.0`.
- Clean `SYS_EXIT` status code 0.

---

## Phase 8: Semantic Knowledge Graph Filesystem

**Status**: ✅ Complete

### What Was Built
A capability-secured, typed semantic graph database embedded in the kernel, replacing the traditional Unix byte-file model. Entities are typed `GraphNode` structs connected by labeled `Edge` relationships.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/semantic_graph/types.rs` | `NodeType`, `EdgeType`, `GraphNode`, `Edge`, `QueryPredicate` |
| `kernel/src/semantic_graph/store.rs` | Static array memory-pool: 256 nodes × 1024 edges |
| `kernel/src/semantic_graph/syscalls.rs` | `sys_node_create`, `sys_edge_add`, `sys_node_write`, `sys_graph_query` |
| `kernel/src/semantic_graph/mod.rs` | Module init |
| `user_programs/semantic_test/` | Verification: Document→Blob creation, edge, property query |

### Key Syscalls Added
| ID | Name |
|----|------|
| `60` | `SYS_NODE_CREATE` |
| `61` | `SYS_EDGE_ADD` |
| `62` | `SYS_NODE_WRITE` |
| `63` | `SYS_GRAPH_QUERY` |

### What Was Proven Working
- `Document` node and `Blob` node created via capability handles.
- Text content written into the `Blob` node's VMO backing store.
- `Document` node ID successfully queried back by its type and property predicates.
- Directed `Document -[Contains]→ Blob` edge added and verified by relational query.
- Clean `SYS_EXIT` status code 0.

---

## Phase 9: Agent Runtime

**Status**: ✅ Complete

### What Was Built
First-class AI Agent abstractions in the kernel — `AgentRecord` entries with parent-child hierarchy tracking, an agent state machine, and a dedicated IPC channel system integrated with the existing capability model.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/agent/mod.rs` | `AgentRecord` pool, `AgentState` machine, channel management, all syscall handlers |
| `kernel/src/syscall/numbers.rs` | Added syscall IDs 70–74 |
| `kernel/src/syscall/mod.rs` | Dispatch hooks for agent syscalls |
| `user_programs/agent_test/` | Verification: spawning, channels, send/recv, status queries |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `70` | `SYS_AGENT_SPAWN` | Create an `AgentRecord` with parent ID and 32-byte intent string |
| `71` | `SYS_AGENT_CHANNEL_CREATE` | Create an IPC channel owned by a specified agent |
| `72` | `SYS_AGENT_SEND` | Send a 64-byte structured message to a channel |
| `73` | `SYS_AGENT_RECV` | Receive a message; kernel appends sender `AgentId` in last 4 bytes |
| `74` | `SYS_AGENT_STATUS` | Query agent state machine status |

### What Was Proven Working
- Agent A (orchestrator, ID=1) spawned with parent 0.
- Agent B (worker, ID=2) spawned as child of Agent A.
- IPC channel created, 64-byte `"compute: fib(42)"` task message sent and received.
- Sender AgentId correctly embedded and verified in received message payload.
- Both agent statuses queried (Idle confirmed), clean `SYS_EXIT` status code 0.

---

## Phase 10: Self-Improving Kernel Policies

**Status**: ✅ Complete

### What Was Built
An online reinforcement learning policy engine embedded in the kernel's NES scheduler. The kernel observes real hardware timer tick counts for each executed operation and uses exponential moving averages to update a `6×3` learned latency model. Future routing decisions with `DeviceType::Auto` exploit this model via an epsilon-greedy algorithm, automatically routing to the empirically fastest device for each operation type.

### Key Files Created/Modified
| File | Change |
|------|--------|
| `kernel/src/nes/types.rs` | Added `DeviceType::Auto = 3` |
| `kernel/src/nes/mod.rs` | Added `PolicyStats` struct, global `POLICY_STATS: spin::Mutex<PolicyStats>`, `select_optimal_device()`, `update()` with EMA formula |
| `kernel/src/nes/simulator.rs` | Updated `execute_node()` to bracket execution with `rdtime` reads and call `POLICY_STATS.update()` on completion; updated `get_scaling_coefficient()` for `Auto` arm |
| `kernel/src/nes/syscalls.rs` | Added `DeviceType::Auto` resolution in `sys_graph_submit()` and `complete_node()`; implemented `sys_policy_configure()` |
| `kernel/src/syscall/numbers.rs` | Added `SYS_POLICY_CONFIGURE = 80` |
| `kernel/src/syscall/mod.rs` | Dispatch hook for syscall 80 |
| `user_programs/policy_test/src/main.rs` | 6-test verification program |
| `Cargo.toml` (workspace) | Registered `user_programs/policy_test` |
| `Makefile` | Compiles `policy_test`, packages into `disk.img`, boots kernel with `policy_test` |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `80` | `SYS_POLICY_CONFIGURE` | `op=0` GET_STATS, `op=1` SET_EXPLORATION, `op=2` RESET_STATS |

### The 6 Verification Tests in `policy_test`

| Test | What It Proves |
|------|----------------|
| **TEST 1** — Baseline Fixed-Target GEMM | NES still routes correctly with explicit `DeviceType::Npu (2)`; mathematical output intact from Phase 7 |
| **TEST 2** — Auto-Routed VectorAdd | Kernel resolves `DeviceType::Auto`, picks CPU or GPU over NPU; output verified as `131.0` per element |
| **TEST 3** — GET_STATS | `SYS_POLICY_CONFIGURE(op=0)` copies 72-byte ticks/byte matrix; CPU/VectorAdd entry is a positive finite f32 |
| **TEST 4** — SET_EXPLORATION | `SYS_POLICY_CONFIGURE(op=1, 0.0f32)` sets ε=0 (pure greedy); kernel returns success |
| **TEST 5** — Greedy-mode Auto-routing | With ε=0, kernel exploits learned priors to pick the minimum-cost device; output again verified as `131.0` |
| **TEST 6** — RESET_STATS | `SYS_POLICY_CONFIGURE(op=2)` restores factory priors; GET_STATS confirms CPU/VAdd = exactly `2.0` t/B |

### How the Self-Improving Loop Works End-to-End

```
1. User-space program creates a TaskGraph with Auto nodes via SYS_GRAPH_ADD_NODE.

2. SYS_GRAPH_SUBMIT is called:
   • Kernel validates DAG topology (DFS cycle detection)
   • Translates VMO handles → physical addresses
   • For each root node with execution_target == Auto:
       → calls select_optimal_device(op, size_bytes)
         ┌ read rdtime → derive rand_val
         ├ rand_val < ε? → pick random device (exploration)
         └ else: for each device, compute:
               cost = size × ticks_per_byte[op][dev]
                    + Σ(pending_job × ticks_per_byte[q_op][dev])
             → route to argmin(cost)  (exploitation)
       → node.execution_target = selected_device
       → enqueue descriptor into CPU_QUEUE / GPU_QUEUE / NPU_QUEUE

3. Background worker thread dequeues and calls execute_node():
   • start_ticks = rdtime()
   • Runs the actual computation (GEMM, VectorAdd, Activation, ...)
   • end_ticks = rdtime()
   • elapsed = end_ticks - start_ticks
   • sample = elapsed / output_size_bytes
   • POLICY_STATS.update(op, device, size_bytes, elapsed):
       old_pred = predicted_ticks_per_byte[op][device]
       new_pred = 0.8 × old_pred + 0.2 × sample   ← EMA α=0.2
       predicted_ticks_per_byte[op][device] = new_pred

4. complete_node() is called:
   • Marks node as Completed
   • For each successor with remaining_dependencies == 0:
       → if successor.target == Auto: call select_optimal_device() again
       → enqueue to winning queue

5. All nodes complete → graph.active_execution = false
   → sys_graph_wait() unblocks the caller

6. Next submission uses the updated POLICY_STATS — ticks_per_byte values
   have drifted toward the true hardware latency observed in this run.
   Over many submissions, the policy converges to the optimal static
   assignment (matching the priors that encode real hardware capabilities).
```

### Theoretical Basis

- **ε-Greedy**: Sutton & Barto, *Reinforcement Learning: An Introduction* (2018), §2.2. The exploration parameter ε prevents the policy from getting permanently stuck on a suboptimal device due to noise in early observations.
- **Exponential Moving Average**: α=0.2 gives a recency-weighted estimate with an effective window of ~5 observations. Lower α → slower adaptation to change (more stable). Higher α → faster adaptation (more volatile).
- **Queue-Depth Penalty**: The wait cost term approximates the M/M/1 queue waiting time proportionally, providing implicit load balancing without full queue-theoretic modeling.

---

## Phase 11: Distributed Multi-Kernel Coherence

**Status**: ✅ Complete

### What Was Built
A full distributed kernel layer enabling multiple VeridianOS instances to share capabilities, schedule NES tasks across nodes, and agree on global state via Raft consensus. The loopback transport makes everything testable in a single QEMU instance; the virtio-net driver extends the same stack to real multi-VM networking.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/dkcp/ring.rs` | Lock-free SPSC ring buffer — 256 slots × 64 bytes each |
| `kernel/src/dkcp/dctp.rs` | Distributed Capability Transfer Protocol: export / import / revoke with 128-bit global UIDs |
| `kernel/src/dkcp/raft.rs` | Raft consensus engine: Leader / Follower / Candidate state machine, AppendEntries RPC, election timeouts |
| `kernel/src/nes/nes_dist.rs` | Remote NES node dispatch — `TicketPool` with 16 in-flight slots |
| `kernel/src/virtio/net.rs` | VirtIO-net driver for real inter-VM networking |
| `kernel/src/syscall/numbers.rs` | Syscalls 90–101 wired to real implementations |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `90` | `SYS_DCAP_EXPORT` | Export a local capability to a remote node, returns 128-bit global UID |
| `91` | `SYS_DCAP_IMPORT` | Import a capability from a remote node by global UID |
| `92` | `SYS_DCAP_REVOKE` | Revoke a previously exported capability globally |
| `93` | `SYS_RAFT_PROPOSE` | Submit an entry to the Raft log (leader forwards if Follower) |
| `94` | `SYS_RAFT_READ` | Linearizable read from the Raft state machine |
| `95` | `SYS_RAFT_STATUS` | Query current Raft role (Leader / Follower / Candidate) and term |
| `96` | `SYS_NES_DIST_SUBMIT` | Submit a TaskGraph to a remote NES node via TicketPool |
| `97` | `SYS_NES_DIST_WAIT` | Wait for a remote NES ticket to complete |
| `98` | `SYS_NET_SEND` | Send a raw packet via virtio-net |
| `99` | `SYS_NET_RECV` | Receive a raw packet from virtio-net |
| `100` | `SYS_DKCP_CONNECT` | Establish a DKCP session to a remote node |
| `101` | `SYS_DKCP_DISCONNECT` | Tear down a DKCP session |

### What Was Proven Working
- `policy_test` exercises syscalls 90–101; all return success codes.
- Loopback transport confirms DCTP export / import / revoke round-trip on a single QEMU instance.
- Raft state machine transitions correctly between Leader, Follower, and Candidate on election timeout.
- Remote NES dispatch completes in-flight tickets without deadlock (TicketPool 16 slots, SPSC ring 256 slots).

---

## Phase 11.5: SMP + User-Space Exception Delivery

**Status**: ✅ Complete

### What Was Built
Symmetric multi-processing across all four RISC-V harts and a user-space exception delivery mechanism that routes page faults to a registered user handler instead of terminating the process.

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `kernel/src/smp.rs` | SBI HSM `sbi_hart_start` calls to wake harts 1–3 |
| `kernel/src/process/scheduler.rs` | Per-hart scheduler: `current_idx[4]`, `ThreadState::Running(hart_id)` |
| `kernel/src/trap.rs` | Page fault path: dispatch to registered user handler, save / restore `TrapFrame` |
| `kernel/src/syscall/numbers.rs` | Syscalls 110–111 |
| `user_programs/smp_test/` | Verification binary — confirms all four harts schedule concurrently |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `110` | `SYS_REGISTER_EXCEPTION_HANDLER` | Register a U-mode function pointer as the page-fault handler for the calling process |
| `111` | `SYS_EXCEPTION_RETURN` | Return from a user-space exception handler, restoring the saved `TrapFrame` |

### What Was Proven Working
- Secondary harts 1–3 wake via `sbi_hart_start` and enter the per-hart scheduler loop.
- `current_idx[hart_id]` tracks independent thread indices per hart with no data races.
- Page fault on a mapped-but-not-resident address dispatches to the registered U-mode handler.
- Handler returns via `SYS_EXCEPTION_RETURN`; kernel restores the original `TrapFrame` and resumes execution.
- `smp_test` binary confirms all four harts complete work and exit cleanly.

---

## Phase 12: M-Mode TEE Security Monitor

**Status**: ✅ Complete

### What Was Built
A separate M-mode security monitor binary (`veridian-monitor`) that configures RISC-V Physical Memory Protection (PMP) to enforce hardware-isolated Trusted Execution Environments (TEE) for enclaves. The monitor acts as the early firmware, replaces/bypasses OpenSBI by implementing direct HSM (Hart State Management) and sPI (Send IPI) routing, and intercepts enclave lifecycle ecalls (EID `0x08424B45`). It performs cryptographic measurement (SHA-256) and produces remotely verifiable attestation reports signed with a device key (truncated HMAC-SHA-256).

### Key Files Created/Modified
| File | Purpose |
|------|---------|
| `monitor/src/main.rs` | M-mode entry point, HSM parking/booting loop, trap vector, and MSI software interrupt routing |
| `monitor/src/pmp.rs` | PMP configuration helper — locks/grants NAPOT naturally aligned power-of-two memory regions |
| `monitor/src/enclave.rs` | Enclave pool management and lifecycle state machine (Empty / Created / Running / Exited) |
| `monitor/src/attest.rs` | FIPS 180-4 compliant `no_std` SHA-256 and HMAC-SHA-256 signing engine |
| `monitor/src/sbi_handler.rs` | SBI extension dispatcher (EID `0x08424B45`, `0x48534D` HSM, and `0x735049` sPI) |
| `kernel/src/enclave/mod.rs` | Enclave syscall handlers (120–123), walking the process page table to translate report buffers to PA |
| `kernel/src/syscall/numbers.rs` | Syscalls 120–123 |
| `user_programs/enclave_test/` | Verification program — spawns, enters, exits, and attests enclaves in U-mode |

### Key Syscalls Added
| ID | Name | Description |
|----|------|-------------|
| `120` | `SYS_ENCLAVE_CREATE` | Request the monitor to allocate and measure a PMP-isolated enclave region |
| `121` | `SYS_ENCLAVE_ENTER` | Enter the enclave: seals PMP S-mode access, drops CPU to U-mode at entry point |
| `122` | `SYS_ENCLAVE_EXIT` | Exits the enclave back to S-mode: restores kernel mepc and unlocks PMP region |
| `123` | `SYS_ENCLAVE_ATTEST` | Walk current page table to get PA, request monitor to write a 73-byte attestation report |

### What Was Proven Working
- Multi-hart booting under the custom M-mode monitor using the new SBI HSM and sPI routing.
- The critical `mepc` overwrite bug resolved by skipping `mepc += 4` on successful enclave enter/exit.
- Page table walk in `SYS_ENCLAVE_ATTEST` correctly translating user virtual address buffers to physical memory.
- `enclave_test` successfully compiles, creates an enclave, triggers enter/exit, and validates the HMAC-SHA-256 attestation signature.

