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
| **Phase 10**| Self-Improving Policies | [x] | Online latency profiling with cycle counters and $\epsilon$-greedy adaptive routing. |
| **Phase 11**| Distributed Coherence | [x] | Atomic SPSC ring buffers, Distributed Capabilities (DCTP), and full Raft consensus replication. Syscalls 90–101 fully implemented (no stubs). |
| **Phase 11.5**| SMP — Secondary Harts | [x] | Secondary harts 1–3 brought online via SBI HSM (`hart_start`); each hart runs its own scheduler loop. |
| **Phase 11.5**| User-Space Exception Delivery | [x] | `SYS_REGISTER_EXCEPTION_HANDLER` syscall; synchronous fault vectoring to registered user-space handler with saved trap frame. |
| **Phase 12**| M-Mode TEE Monitor | [x] | Separate M-mode binary (`monitor/` crate) with PMP-based enclave isolation, SHA-256 measurement, HMAC-SHA-256 attestation, and SBI extension EID `0x08424B45`. Syscalls 120–123 (`SYS_ENCLAVE_CREATE/ENTER/EXIT/ATTEST`) fully implemented. `enclave_test` verifies full lifecycle + remote attestation on QEMU. |

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

### Phase 11.5: SMP & User-Space Exception Delivery
* **SMP (Secondary Harts):** Secondary harts 1–3 are brought online via SBI HSM `hart_start`. Each hart enters its own supervisor-mode scheduling loop independently of hart 0. Verified in QEMU with `-smp 4`.
* **User-Space Exception Delivery:** Processes may call `SYS_REGISTER_EXCEPTION_HANDLER` to register a handler entry point. On a synchronous fault (page fault, illegal instruction, etc.) the kernel saves the full trap frame and vectors execution to the registered handler instead of terminating the process. The handler can inspect the frame, attempt recovery, and return.

### Phase 12: M-Mode TEE Monitor (In Progress)
* **Goal:** Establish a minimal M-mode Trusted Execution Environment (TEE) monitor that coexists with the S-mode kernel to enforce Physical Memory Protection (PMP) enclaves, provide attestation, and serve as a Keystone-compatible security anchor.
* **Current status:** Scaffold exists — M-mode entry point, trap delegation table, and SBI passthrough stubs are in place. Linker script and build-system integration are the active work items.

---

## <img src="docs/assets/icons/rocket.svg" width="22" height="22" style="vertical-align: middle; margin-right: 8px;" /> Future Development Goals

### Upcoming Milestones

| Milestone | Phase | Description |
| :--- | :--- | :--- |
| Complete M-mode monitor binary | Phase 12 | Finalize linker script and build system integration for the M-mode TEE monitor; produce a standalone `monitor.bin` loaded by QEMU `-bios`. |
| Keystone-compatible PMP enclave isolation | Phase 12.1 | Wire PMP registers from M-mode to carve out S-mode-inaccessible enclave regions; implement `ENCLAVE_CREATE` / `ENCLAVE_ENTER` / `ENCLAVE_EXIT` SBI calls. |
| SHA-256 attestation & remote verification | Phase 12.2 | Hash enclave binary at creation time in M-mode; expose an `ATTEST` SBI call returning a signed measurement report verifiable by a remote party. |
| virtio-net real two-QEMU distributed testing | Phase 13 | Replace DKCP loopback with a real `virtio-net` driver; run two QEMU instances connected via a TAP bridge and execute the full distributed test suite. |
| CHERI pointer safety integration | Phase 14 | Evaluate CHERI-RISC-V capability instructions for hardware-enforced spatial memory safety within the kernel and user-space ABI. |
