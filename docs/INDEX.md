# VeridianOS Documentation

VeridianOS is a clean-slate, open-source microkernel operating system written from scratch in **Rust** for **RISC-V 64-bit**. It replaces forty-year-old OS paradigms with first-class kernel abstractions for AI agents, heterogeneous accelerators, and distributed capability sharing — enforcing a zero-trust object-capability security model (no `root`, no ambient authority) across a Semantic Graph Filesystem, a Neural Execution Subsystem with online self-improving scheduling, and a distributed multi-kernel coherence protocol backed by a full Raft consensus engine. As of Phase 12, it adds M-mode PMP-based TEE isolation with SHA-256 measurement and HMAC attestation.

---

## Reading Paths

**For Recruiters and Engineering Managers**
> Goal: Understand the technical ambition and engineering quality in 10 minutes

1. [README](../README.md) — Project overview, architecture pillars, and quick start
2. [ROADMAP](../ROADMAP.md) — 12-phase build progression with status
3. [DESIGN.md](DESIGN.md) §1–2 — Project vision and architecture overview
4. [Phase 12 TEE Monitor](PHASE_12_DESIGN.md) — Representative phase doc showing design depth

---

**For Systems Researchers**
> Goal: Evaluate the architecture and academic grounding

1. [DESIGN.md](DESIGN.md) — Full technical reference covering all 20 sections: capability system, memory management, NES, SGF, agent runtime, distributed coherence, SMP, TEE, syscall table, and testing strategy
2. [research/RESEARCH_NOTES.md](research/RESEARCH_NOTES.md) — Academic foundations, design rationale, and open problems
3. [research/ACADEMIC_REFERENCES.md](research/ACADEMIC_REFERENCES.md) — Annotated citations (seL4, LithOS, Asterinas, Fuchsia/Zircon, Raft, Keystone)
4. Phase design docs for subsystems of interest — each covers goals, data structures, algorithms, and verification

---

**For Contributors**
> Goal: Build it, understand the codebase, add a feature

1. [ONBOARDING.md](ONBOARDING.md) — Prerequisites, toolchain setup, build steps, and first contribution walkthrough
2. [DESIGN.md](DESIGN.md) §15–17 — Syscall reference, developer onboarding supplement, and testing/QA strategy
3. [../CONTRIBUTING.md](../CONTRIBUTING.md) — Branch flow, commit style, PR conventions
4. Phase doc for the subsystem you want to touch — find it in the index below

---

**For the General Tech Community**
> Goal: Understand what makes this OS different

1. [README](../README.md) — The pitch: what it is, why it exists, how to boot it in 2 minutes
2. [research/FUTURE_COMPUTING_TRENDS.md](research/FUTURE_COMPUTING_TRENDS.md) — Why legacy OS assumptions break down from 2025–2055 and how VeridianOS is positioned for each shift
3. [DESIGN.md](DESIGN.md) §1 — Project vision and design principles
4. [ROADMAP](../ROADMAP.md) — What has been built, phase by phase

---

## Document Index

| Document | Description | Best For |
|----------|-------------|----------|
| [DESIGN.md](DESIGN.md) | Unified technical reference — all phases, syscall table, architecture, research, testing | Everyone |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Deep architectural detail with component diagrams and module breakdown | Researchers, Contributors |
| [ONBOARDING.md](ONBOARDING.md) | Dev environment setup, build steps, QEMU run, first contribution walkthrough | Contributors |
| [version_control.md](version_control.md) | Branch strategy (`develop`/`main`), commit conventions, tagging discipline | Contributors |
| [NEURAL_SCHEDULER_DESIGN.md](NEURAL_SCHEDULER_DESIGN.md) | Phase 7 deep dive: heterogeneous queues (CPU/GPU/NPU) and DAG scheduler design | Researchers, Contributors |
| **Phase Design Docs** | | |
| [PHASE_01_DESIGN.md](PHASE_01_DESIGN.md) | Boot sequence, linker script, UART driver, assembly entry, supervisor-mode transition | Researchers, Contributors |
| [PHASE_02_DESIGN.md](PHASE_02_DESIGN.md) | Capability system: Handles, HandleTable, rights attenuation, syscall routing | Researchers, Contributors |
| [PHASE_03_DESIGN.md](PHASE_03_DESIGN.md) | Binary Buddy page allocator and Sv39 three-level page tables | Researchers, Contributors |
| [PHASE_04_DESIGN.md](PHASE_04_DESIGN.md) | Preemptive thread scheduler: context switching, state machine, timer-driven preemption | Researchers, Contributors |
| [PHASE_05_DESIGN.md](PHASE_05_DESIGN.md) | VirtIO block driver and POSIX ustar InitRAMFS scanner | Researchers, Contributors |
| [PHASE_06_DESIGN.md](PHASE_06_DESIGN.md) | ELF loader, user stack initialization, Ring 3 mode transition | Researchers, Contributors |
| [PHASE_07_DESIGN.md](PHASE_07_DESIGN.md) | Neural Execution Subsystem: heterogeneous queues, DAG schedulers, NES syscalls | Researchers, Contributors |
| [PHASE_08_DESIGN.md](PHASE_08_DESIGN.md) | Semantic Graph Filesystem: typed nodes, labeled edges, graph traversal syscalls | Researchers, Contributors |
| [PHASE_09_DESIGN.md](PHASE_09_DESIGN.md) | Agent Runtime: AgentProcess, AgentChannel, lifecycle syscalls 70–74 | Researchers, Contributors |
| [PHASE_10_DESIGN.md](PHASE_10_DESIGN.md) | Self-improving scheduling: cycle counters, ε-greedy router, online EMA policy updates | Researchers, Contributors |
| [PHASE_11_DESIGN.md](PHASE_11_DESIGN.md) | Distributed coherence: DKCP rings, DCTP, remote NES dispatch, Raft engine, syscalls 90–101 | Researchers, Contributors |
| [PHASE_12_DESIGN.md](PHASE_12_DESIGN.md) | M-mode TEE monitor: PMP enclave isolation, SHA-256 measurement, HMAC attestation, syscalls 120–123 | Researchers, Contributors |
| **Research** | | |
| [research/RESEARCH_NOTES.md](research/RESEARCH_NOTES.md) | Academic foundations, design rationale, open research problems | Researchers |
| [research/ACADEMIC_REFERENCES.md](research/ACADEMIC_REFERENCES.md) | Annotated citations: seL4, LithOS, Asterinas, Fuchsia, Raft, Keystone, and more | Researchers |
| [research/FUTURE_COMPUTING_TRENDS.md](research/FUTURE_COMPUTING_TRENDS.md) | 2025–2055 computing paradigm shifts and VeridianOS positioning across each | Everyone |

---

## Architecture at a Glance

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

---

## Phase Completion Status

| Phase | Name | Status | Verification |
|-------|------|--------|--------------|
| 1 | Bootable RISC-V Microkernel | ✅ | UART output on QEMU boot |
| 2 | Capability System Foundation | ✅ | Handle alloc/revoke syscall tests |
| 3 | Page Allocator and Sv39 VM | ✅ | Buddy allocator + page table tests |
| 4 | Preemptive Thread Scheduler | ✅ | Context switch and timer preemption tests |
| 5 | VirtIO Block Driver and InitRAMFS | ✅ | Block read + ustar extraction tests |
| 6 | ELF Loader and User Mode | ✅ | User binary executes in Ring 3 |
| 7 | Neural Execution Subsystem | ✅ | DAG dispatch across CPU/GPU/NPU queues |
| 8 | Semantic Graph Filesystem | ✅ | Node/edge create, query, traverse syscalls |
| 9 | Agent Runtime | ✅ | Agent spawn, channel messaging, lifecycle syscalls 70–74 |
| 10 | Self-Improving Kernel Policies | ✅ | EMA latency tables, ε-greedy router convergence |
| 11 | Distributed Multi-Kernel Coherence | ✅ | DKCP rings, DCTP, Raft log; syscalls 90–101 verified |
| 11.5 | SMP — Secondary Harts | ✅ | Harts 1–3 online via SBI HSM, `-smp 4` QEMU verified |
| 11.5 | User-Space Exception Delivery | ✅ | `SYS_REGISTER_EXCEPTION_HANDLER` fault vectoring verified |
| 12 | M-Mode TEE Monitor | ✅ | `enclave_test`: full lifecycle + remote attestation on QEMU |
