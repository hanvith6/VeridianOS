# VeridianOS Project Roadmap

This document outlines the design and implementation phases of **VeridianOS**, a clean-slate capability-based AI-native operating system. Each phase builds incrementally on previous foundations.

---

## <img src="docs/assets/icons/roadmap.svg" width="22" height="22" style="vertical-align: middle; margin-right: 8px;" /> System Evolution Timeline

| Phase | Milestone | Status | Description |
| :--- | :--- | :---: | :--- |
| **Phase 1** | Bootable Microkernel | [x] | RISC-V 64-bit boot assembly, basic linker script, 16550 UART logging. |
| **Phase 2** | Capability Foundations | [x] | Unforgeable object-capabilities, `HandleTable`, and basic syscall routing. |
| **Phase 3** | Memory & Virtualization | [x] | Buddy allocator for page frames and Sv39 page tables for address spaces. |
| **Phase 4** | Thread Scheduler | [x] | Context switching, thread lifecycle state machine, and timer preemption. |
| **Phase 5** | InitRAMFS & Block IO | [x] | VirtIO block device interface and POSIX Ustar tar filesystem extraction. |
| **Phase 6** | Userspace execution | [x] | ELF parsing, user stacks allocation, and Ring 3 mode transitions. |
| **Phase 7** | Neural Execution (NES) | [x] | Heterogeneous task queues (CPU/GPU/NPU) and DAG schedulers. |
| **Phase 8** | Semantic Graph FS (SGF) | [x] | Entity node and labeled relationship edge graph DB in kernel space. |
| **Phase 9** | Agent Runtime | [x] | Kernel-space AI agents, communication channels, and process isolation. |
| **Phase 10**| Self-Improving Policies | [x] | online latency profiling with cycle counters and $\epsilon$-greedy adaptive routing. |
| **Phase 11**| Distributed Coherence | [x] | Atomic SPSC ring buffers, Distributed Capabilities, and Raft consensus replication. |

---

## <img src="docs/assets/logo.png" width="22" height="22" style="vertical-align: middle; margin-right: 8px; border-radius: 4px;" /> Subsystem Deep Dive

### Phase 1 to 4: The Microkernel Core
* **Goal:** Boot on bare metal/QEMU, register trap vectors, and manage raw resources.
* **Security:** Every hardware page and scheduling slot requires a security ticket (`Handle`).

### Phase 5 to 6: Userspace & Storage
* **Goal:** Execute isolated user binaries mapped from raw block storage.
* **Execution:** Programs compile targeting `riscv64gc-unknown-none-elf` and execute in user privilege mode (Ring 3).

### Phase 7 to 9: Semantic & Agent Runtimes
* **Goal:** Establish a graph filesystem instead of flat directories, and build agent lifecycles.
* **Philosophy:** Data is structured as a knowledge graph; computations are structured as neural dataflow graphs.

### Phase 10: Online Feedback Optimization
* **Goal:** Self-optimizing scheduling.
* **Mechanism:** The kernel acts as a learning agent, dynamically redirecting compute tasks to the fastest device.

### Phase 11: Multi-Kernel Clustered System
* **Goal:** Share capabilities and filesystem state across physical computers/harts.
* **Mechanisms:**
  - **DKCP (Distributed Kernel Coherence Protocol):** Atomic SPSC ring transport (64-byte cache-line messages, lock-free enqueue/dequeue) with loopback simulation in QEMU.
  - **Cluster Membership:** `ClusterState` tracks up to 8 `KernelDomainId` peers with per-domain liveness epoch counters; domains declared Dead after 5 missed heartbeat ticks.
  - **DCTP (Distributed Capability Transfer Protocol):** Cap export derives a 128-bit UID from rdtime + domain + handle; shadow Handles installed into remote process tables on import; epoch-based revocation propagated via `CapRevokeNotify`.
  - **Remote NES Dispatch:** `DistTicket` pool tracks 16 in-flight remote graph nodes; loopback immediately injects synthetic `GraphNodeResult` for QEMU single-instance verification; `wait_remote` uses rdtime-based timeout.
  - **Raft Consensus Engine:** Complete Raft state machine (Follower → Candidate → Leader) replicated over the DKCP ring; single-node cluster wins quorum immediately; `append_entry` commits `SemanticGraphMutation` records to a 128-slot static log.
  - **Syscalls 90–101** fully wired to real implementations (no stubs).

---

## <img src="docs/assets/icons/rocket.svg" width="22" height="22" style="vertical-align: middle; margin-right: 8px;" /> Future Development Goals

### Upcoming Milestones (Post-Phase 11)
1. **Network Hardware Acceleration:** Upgrade from loopback shared memory to real high-speed PCIe network interfaces (`virtio-net` hardware acceleration).
2. **Dynamic Host Joining:** Allow nodes to dynamically join and leave the SGF consensus ring without restarting.
3. **Formal Verification Proofs:** Draft mathematical correctness proofs for the Distributed Capability Transfer Protocol (DCTP) rules using TLA+.
4. **LLM Compiler Integration:** Develop a lightweight user-space compiler translating high-level natural language instructions directly into kernel-space NES task graphs.
