# Changelog

All notable changes to VeridianOS are documented here.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).  
Versioning follows [Semantic Versioning](https://semver.org/) — `v0.x.y` during design phases.

---

## [v0.12.0] — 2026-05-30 — M-Mode TEE Security Monitor

Phase 12: Hardware-attested Trusted Execution Environments for AI agents.

### Added

- **M-mode TEE monitor** (`monitor/` crate) — separate binary loaded by QEMU `-bios`
  - `pmp.rs` — NAPOT PMP encoding, `lock_region` / `unlock_region` / `grant_region` / `lock_monitor_self`
  - `enclave.rs` — static 8-slot enclave pool, `EnclaveState` lifecycle (Empty → Created → Running → Exited)
  - `attest.rs` — SHA-256 (FIPS 180-4) measurement + HMAC-SHA-256 attestation, 73-byte report format
  - `sbi_handler.rs` — SBI extension EID `0x08424B45` ("BKE"), FIDs 0–3
- **Kernel enclave bridge** (`kernel/src/enclave/`) — S-mode syscall handlers with argument validation
- **Syscalls 120–123** — `SYS_ENCLAVE_CREATE`, `SYS_ENCLAVE_ENTER`, `SYS_ENCLAVE_EXIT`, `SYS_ENCLAVE_ATTEST`
- **`AgentRecord.enclave_id: Option<u8>`** — agents can run inside hardware-isolated TEE enclaves
- **`user_programs/enclave_test`** — 3 integration tests: create, enter/exit, attestation HMAC verification
- **`docs/DESIGN.md`** — single unified 935-line project reference (architecture, all phases, syscall table, onboarding, QA, research, roadmap)
- **`docs/PHASE_12_DESIGN.md`** — 1005-line Phase 12 deep-dive (PMP math, state machine, SBI spec, attestation format)
- **`CHANGELOG.md`** — this file

### Changed

- `ROADMAP.md` — Phase 12 marked complete with full implementation details
- `docs/FUTURE_COMPUTING_TRENDS.md` — expanded to 10 trends with real hardware numbers
- `docs/RESEARCH_NOTES.md` — new §4 covering Keystone, PMP spec, RATS RFC 9334, TEE comparison table
- `docs/ACADEMIC_REFERENCES.md` — new §6 with 6 TEE/attestation paper entries
- `monitor/src/main.rs` — fixed `unused_doc_comments` warning on `global_asm!`
- `docs/version_control.md` — current stage updated from Phase 11 to Phase 12

### Security

- M-mode PMP entry 15 is locked at monitor boot (`L=1`) — immutable until system reset
- S-mode kernel cannot read PMP registers; enclave memory isolation is hardware-enforced
- SHA-256 measurement is computed before PMP lock, preventing post-measurement tampering

---

## [v0.11.0] — 2026-05-28 — Distributed Multi-Kernel Coherence

### Added

- **DKCP** — lock-free SPSC ring transport (256 × 64-byte slots, cache-line aligned)
- **DCTP** — distributed capability export/import/revoke with 128-bit UIDs
- **Raft consensus engine** — Follower/Candidate/Leader state machine over DKCP rings
- **Remote NES dispatch** — 16-slot `DistTicket` pool, loopback synthetic result injection
- **Cluster membership** — `ClusterState` with liveness epoch counters, dead-domain detection
- **Syscalls 90–101** — domain join/list/status, remote NES dispatch/wait/abort, cap export/import/revoke, SGF replicate, Raft status

---

## [v0.11.5] — 2026-05-28 — SMP and User-Space Exception Delivery

### Added

- **SMP** — secondary harts 1–3 brought online via SBI HSM `hart_start`; each hart runs independent scheduler loop
- **`SYS_REGISTER_EXCEPTION_HANDLER`** (110) — register user-space fault handler entry point and stack
- **`SYS_EXCEPTION_RESUME`** (111) — resume execution after fault handling
- **`smp_test`** — verifies secondary hart activation and exception handler round-trip

---

## [v0.10.0] — 2026-05-27 — Self-Improving Kernel Policies

### Added

- **EMA latency tracking** — `PolicyStats` matrix (6 op types × 3 devices), α=0.2 exponential moving average
- **Epsilon-greedy router** — ε=0.1 exploration; routes NES nodes to empirically fastest device
- **`rdtime` CSR sampling** — hardware cycle counter timestamps before/after node execution
- **`policy_test`** — verifies router convergence and EMA update correctness

---

## [v0.9.0] — 2026-05-26 — Agent Runtime

### Added

- **`AgentRecord`** — kernel-space AI agent with mailbox, lifecycle state, capability budget
- **`AgentChannel`** — bidirectional capability-secured IPC channel between agents
- **Syscalls 80–84** — `SYS_AGENT_SPAWN/SEND/RECV/STATUS/KILL`
- **`agent_test`** — verifies spawn, message round-trip, agent termination

---

## [v0.8.0] — 2026-05-25 — Semantic Graph Filesystem

### Added

- **SGF node/edge model** — typed `Node` entities + labeled `Edge` relationships
- **Syscalls 70–74** — `SYS_SGF_NODE_CREATE/GET/DELETE`, `SYS_SGF_EDGE_CREATE/QUERY`
- **`semantic_test`** — verifies CRUD and graph traversal

---

## [v0.7.0] — 2026-05-24 — Neural Execution Subsystem

### Added

- **`DeviceQueue`** — per-device (CPU/GPU/NPU) task queues with doorbell MMIO
- **DAG execution** — `NesNode` dependency resolution before dispatch
- **Syscalls 60–62** — `SYS_NES_SUBMIT/WAIT/QUERY`
- **`neural_test`** — verifies node submission and DAG dependency ordering

---

## [v0.1.0–v0.6.0] — 2026-05-20 to 2026-05-23 — Microkernel Core

- v0.1.0 — RISC-V 64 boot, 16550 UART, `#![no_std]` kernel entry
- v0.2.0 — Capability system: `Handle`, `HandleTable`, rights attenuation
- v0.3.0 — Buddy page allocator, Sv39 three-level page tables
- v0.4.0 — Preemptive round-robin scheduler, timer interrupt, context switch
- v0.5.0 — VirtIO block driver, ustar InitRAMFS
- v0.6.0 — ELF loader, user stack setup, U-mode `sret` transition
