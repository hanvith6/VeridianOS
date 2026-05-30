# VeridianOS — Senior Engineering Audit Report

| Attribute | Value |
|---|---|
| **Audit Date** | 2026-05-30 |
| **Version Audited** | v0.12.1 |
| **Codebase Size** | 61 Rust files · 13,371 lines |
| **Phases Covered** | 1–12 |
| **Auditors** | Architecture · Security · QA · DevOps · SDLC |

---

## Executive Summary

VeridianOS is a technically ambitious, structurally sound research microkernel with a well-documented architecture and a clear phase-by-phase development history. The M-mode / S-mode privilege split is correctly implemented, the capability model is architecturally correct in intent, and the Phase 12 TEE monitor represents genuine systems research work.

However, the project has critical gaps that prevent it from being classified as production-ready: zero automated CI, zero unit tests, several CRITICAL unsafe code issues including an unvalidated DTB parser and a uniformly RWX kernel memory map, an incomplete capability rights enforcement system, and a hardcoded attestation device key.

**Final Verdict: APPROVED FOR STAGING ONLY**

The project is development-complete for its research scope. It is not MVP production-ready. Estimated 3–4 sprint effort to reach staging deployment readiness.

---

## 1. Phase Completion Matrix

| Phase | Name | Status | Completion | Risks | Action Required |
|---|---|---|---|---|---|
| 1 | Bootable Microkernel | ✅ Complete | 95% | DTB parser is unvalidated (CRITICAL) | Fix `parse_bootargs` bounds checks |
| 2 | Capability System | ✅ Complete | 70% | Rights amplification possible; `Handle::new()` takes arbitrary rights | Implement `derive_handle()` with parent rights masking |
| 3 | Page Allocator & Sv39 | ✅ Complete | 85% | Entire RAM mapped RWX; no section-level protection | Implement R/X/W-separated page table mappings |
| 4 | Thread Scheduler | ✅ Complete | 90% | SUM bit permanently set; reduces U-mode isolation | Gate SUM around copy windows only |
| 5 | VirtIO Block + InitRAMFS | ✅ Complete | 90% | No error-path testing for malformed TAR input | Add malformed-input test case |
| 6 | ELF Loader + U-mode | ✅ Complete | 85% | ASLR disabled (TODO in code) | Re-enable ASLR with timer-seeded offset |
| 7 | Neural Execution Subsystem | ✅ Complete | 90% | DAG cycle and queue-overflow paths untested | Add negative-path tests |
| 8 | Semantic Graph Filesystem | ✅ Complete | 85% | Edge-deletion and node-exhaustion untested | Add SGF error-path user program |
| 9 | Agent Runtime | ✅ Complete | 85% | Mailbox overflow and dead-agent send untested | Add agent robustness test |
| 10 | Self-Improving Policies | ✅ Complete | 90% | EMA convergence only tested on single QEMU | Determinism under non-uniform latency unverified |
| 11 | Distributed Coherence | ✅ Complete | 75% | DKCP/Raft/DCTP tested loopback only; never against a real peer | Phase 13 two-QEMU virtio-net test required |
| 11.5 | SMP + Exception Delivery | ✅ Complete | 85% | Exception handler double-fault path untested | Add fault-in-handler test |
| 12 | M-Mode TEE Monitor | ✅ Complete | 75% | Enclave PMP entries not locked (no `PMP_L`); `grant_region` gives `X`; device key hardcoded; pool exhaustion untested | See Security Remediation Plan §Sprint 2 |

---

## 2. Architecture Audit

### Grade: C+

The two-privilege-level split (M-mode monitor / S-mode kernel) is architecturally sound. PMP entry 15 (monitor self-protection) is correctly locked before S-mode starts. The capability system has the correct structural model.

### Critical Architecture Findings

**Module Coupling — `kernel/src/main.rs` is a God-file**
`kmain` directly initializes all 10+ subsystems sequentially with no error recovery. Any subsystem failure causes print-and-continue, leaving the kernel in an inconsistent state. Recommendation: introduce an `InitResult` type and a staged initialization sequence with abort-on-failure semantics.

**PROCESS_TABLE global mutex contention**
`kernel/src/process/mod.rs` exposes `PROCESS_TABLE` as a single `Mutex<[Option<Process>; 16]>` locked by every subsystem. On a 4-hart SMP kernel this is a single-point serialization bottleneck and a lock-order hazard. Recommendation: per-process locks or a lock-free process table.

**Distributed layer runs unconditionally on single-instance**
`dist/` initializes and ticks on every timer interrupt even with no network peer. Wasted interrupt budget on every single-QEMU boot. Recommendation: gate `dist::init()` on a detected network device.

**`USE_REAL_NET = true` is a dead constant**
`dist/transport.rs:8` — the constant is never read. The transport selection is based on `virtio::net::is_initialized()`, not this flag. The constant creates a false impression of configuration. Remove or wire it correctly.

**Hardcoded VMO addresses in `spawn()`**
`process/mod.rs:253-264` hardcodes physical addresses (e.g., `0x8610_0000`) for test process VMOs without allocating through the buddy allocator. These can alias kernel heap on real hardware or a different QEMU memory layout.

---

## 3. Code Review Report

### Summary

| Severity | Count |
|---|---|
| CRITICAL | 4 |
| HIGH | 8 |
| MEDIUM | 4 |
| LOW | 3 |

### CRITICAL

**[CRITICAL] `kernel/src/main.rs:348-449` — Unvalidated DTB parser**
`parse_bootargs` reads `off_struct`, `off_strings`, and property `len` directly from the untrusted DTB blob without bounds-checking any value against `totalsize`. An attacker-controlled DTB (e.g., via QEMU `-dtb`) can cause out-of-bounds reads across physical memory. The `core::mem::transmute` at line 433 manufactures a `'static` lifetime for a DTB-backed string — use-after-free if the DTB memory is later reused.
**Fix:** Validate `off_struct < totalsize`, `off_strings < totalsize`, and `off_struct + len <= totalsize` before every pointer advance. Copy bootargs into a static buffer instead of transmuting the lifetime.

**[CRITICAL] `kernel/src/memory/mod.rs:62-76` — Entire RAM mapped RWX**
All physical RAM is identity-mapped with `READ | WRITE | EXECUTE`. The `.text` section is writable and heap pages are executable. Any write primitive in kernel space becomes a code injection primitive.
**Fix:** Separate mapping passes per linker section: `.text` → RX, `.rodata` → R, `.data`/`.bss`/heap → RW.

**[CRITICAL] `monitor/src/main.rs:338` — `m_trap_handler` not marked `unsafe` (FIXED)**
`m_trap_handler(*mut TrapFrame)` dereferenced a raw pointer without being marked `unsafe`. This caused `cargo clippy` to fail.
**Status: Fixed in this audit session** — function now marked `pub unsafe extern "C"` with SAFETY comment.

**[CRITICAL] `kernel/src/capability/mod.rs:59` + `process/mod.rs:227-265` — Rights amplification**
`Handle::new()` accepts any `Rights` bitfield without checking caller authority. `spawn()` grants full `READ | WRITE | DUPLICATE | EXECUTE` rights with no parent capability check.
**Fix:** Implement `fn derive_handle(parent: &Handle, requested: Rights) -> Result<Handle, RightsError>` that enforces `new_rights = parent.rights & requested`.

### HIGH

| Location | Finding | Fix |
|---|---|---|
| `capability/mod.rs:91` | `HandleTable::set()` silently overwrites existing handles; no generation counter | Add generation field; error on overwrite |
| `dist/cluster.rs:200,231,264` | `domain_join/list/status` dereference raw user pointers without address-space validation | Call `validate_user_buffer` before dereferencing |
| `dist/transport.rs:57` | `VirtioNetTransport::send` reads exactly 64 bytes via raw ptr regardless of `DkcpMessage` size | Use `size_of::<DkcpMessage>()` or add `const_assert!` |
| `monitor/src/pmp.rs:169` | `lock_region()` never sets `PMP_L` — enclave entries can be reconfigured after sealing | Add `PMP_L` to the cfg field in `lock_region()` |
| `monitor/src/pmp.rs:234` | `grant_region()` grants `PMP_X` during binary loading, contradicting "X withheld until enter" | Remove `PMP_X` from `grant_region()`; add separate `grant_rw_only()` |
| `monitor/src/enclave.rs:110` | `ENCLAVE_POOL` is `pub static mut` — concurrent hart access is UB | Wrap in `spin::Mutex` |
| `monitor/src/main.rs:121` | `HART_STATES` mutable static accessed from multiple harts without synchronization | Replace fields with `AtomicUsize` |
| `kernel/src/trap.rs:32` | `SUM` bit permanently enabled — S-mode can read/write all user pages globally | Gate SUM around kernel-to-user copy operations only |

### MEDIUM

| Location | Finding |
|---|---|
| `process/mod.rs:109-121` | ASLR disabled (TODO comment); deterministic stack address `0x40002000` |
| `monitor/src/main.rs:166` | Secondary harts have a PMP self-protection window gap between `_start` and `lock_monitor_self()` |
| `dist/cluster.rs:150` | Cluster messages have all-zero MAC field — unauthenticated |
| `process/mod.rs:253-264` | Hardcoded physical VMO addresses not allocated via buddy allocator |

### LOW

| Location | Finding |
|---|---|
| `monitor/src/main.rs:22` | `#![allow(unused_unsafe)]` in security-critical binary — remove to surface cleanup opportunities |
| `kernel/src/main.rs:57,79` | Dead `test_thread_1/2` with unsafe CSR manipulation silenced by `#[allow(dead_code)]` |
| `kernel/src/page_alloc.rs:253` | `static mut TEST_BUF` in allocator test causes aliasing hazard if called twice |

---

## 4. Security Audit

### Security Score: 41 / 100

| Category | Score | Notes |
|---|---|---|
| Privilege separation (M/S split) | 8/10 | Correct; secondary hart window gap |
| PMP correctness | 5/10 | Entry 15 locked correctly; enclave entries not locked; grant_region gives X |
| Capability enforcement | 3/10 | No rights derivation; handle forgery via set(); object_ptr unvalidated |
| Memory isolation | 3/10 | All RAM RWX; SUM permanently set; ASLR disabled |
| Input validation | 2/10 | DTB parser unsafe; cluster user-ptr derefs unvalidated |
| Unsafe discipline | 4/10 | ~35% SAFETY comment coverage; 1 unsafe per 51 lines (5× cautious baseline) |
| Cryptographic correctness | 4/10 | SHA-256 correct; symmetric HMAC key wrong threat model for attestation |
| Test coverage | 0/10 | Zero `#[test]` functions |
| DoS resilience | 5/10 | Several panic paths on bad input in init sequence |
| Distributed security | 7/10 | Unauthenticated cluster messages; dead USE_REAL_NET constant |

### OWASP Kernel Adaptation

| Threat Class | Present | Evidence |
|---|---|---|
| Memory Corruption | YES | DTB parser unbounded reads; RWX memory map |
| Privilege Escalation | YES | Rights amplification in Handle::new(); SUM permanently set |
| Confused Deputy | YES | domain_list/status write to caller-supplied ptrs |
| TOCTOU | YES | enclave_attest reads slot without holding lock in SMP context |
| Hardcoded Credentials | YES | DEVICE_KEY compile-time constant in attest.rs |
| Unvalidated Input | YES | DTB, cluster user pointers |
| Insecure Deserialization | PARTIAL | DkcpMessage raw ptr read with fixed 64-byte length |

### Security Remediation Plan

**Sprint 1 — Blockers:**
1. ✅ Mark `m_trap_handler` `unsafe` *(fixed this session)*
2. Fix DTB parser bounds checking (`main.rs:348-449`)
3. Implement R/W/X-separated kernel memory map (`memory/mod.rs:62-76`)
4. Validate user pointers in `domain_join/list/status` (`cluster.rs:200,231,264`)

**Sprint 2 — High Priority:**
5. Add `PMP_L` to `lock_region()` (`pmp.rs:169`)
6. Remove `PMP_X` from `grant_region()` (`pmp.rs:234`)
7. Implement `derive_handle()` rights derivation (`capability/mod.rs`)
8. Wrap `ENCLAVE_POOL` in `spin::Mutex`; replace `HART_STATES` fields with `AtomicUsize`

**Sprint 3 — Hardening:**
9. Replace `DEVICE_KEY` with runtime-read identity; upgrade to Ed25519 signing
10. Re-enable ASLR with timer-seeded stack offset (`process/mod.rs:109`)
11. Gate SUM bit around copy windows only (`trap.rs:32`)
12. Add `const_assert!(size_of::<DkcpMessage>() == 64)` + SAFETY comments in `transport.rs`

---

## 5. QA Report

### Test Coverage Score: 18 / 100

| Layer | Coverage | Notes |
|---|---|---|
| Unit tests (`#[test]`) | 0% | Zero test functions anywhere in workspace |
| Integration (QEMU programs) | ~40% happy paths | 7 binaries cover Phase 1–12 happy paths only |
| Negative path / error injection | 0% | No test sends invalid args, null ptrs, or exhausted resources |
| Multi-node distributed | 0% | Raft/DCTP/DKCP never tested against real peer |
| Property-based / fuzz | 0% | None |

### Top 5 Missing Tests (Prioritized)

1. **`rights_test`** — Calls syscalls with attenuated handles, verifies `-EACCES` returned. Critical security gap.
2. **`syscall_robustness_test`** — Sends null pointers and invalid handle IDs to every syscall. Verifies error codes, not panics.
3. **Buddy allocator `#[test]`** — Unit tests for split/merge/double-free. Extractable to `--lib` test even in `no_std`.
4. **Enclave pool exhaustion** — Extend `enclave_test`: create 9th enclave, expect error; two overlapping enclave regions, expect rejection.
5. **Exception delivery negative path** — Fault with no handler (expect clean process termination); fault inside handler (expect no recursive loop).

### QA Risk by Subsystem

| Subsystem | Risk |
|---|---|
| Buddy allocator / Sv39 | 🔴 HIGH — Foundation; zero unit tests; any regression = kernel panic |
| HandleTable / capability rights | 🔴 HIGH — Entire security model; no negative-path tests |
| Raft/DCTP/DKCP | 🔴 HIGH — Never tested against real peer |
| NES DAG / SGF / Agent | 🟠 MEDIUM — Happy path tested; error paths dark |
| M-mode TEE / PMP | 🟠 MEDIUM — Lifecycle tested; pool exhaustion and PMP overlap untested |
| UART / boot pipeline | 🟢 LOW — Exercised on every boot |

---

## 6. Performance Report

### Findings

**No performance benchmarks exist.** There is no `benches/` directory, no Criterion setup, and no `rdtime`-based microbenchmark extraction.

What is observable from code structure:

| Path | Concern |
|---|---|
| PROCESS_TABLE global mutex | Single lock for all 4 harts; O(N) scan for free slot |
| Dist heartbeat on every timer tick | Unconditional even with no network peer |
| NES DAG topological sort | O(V+E) on every `sys_graph_submit`; no caching |
| Raft tick on every timer interrupt | Runs on hart 0 unconditionally |
| DKCP ring `send` | Lock-free SPSC — correct and fast |
| PMP NAPOT encoding | Computed inline, no cache — acceptable |
| SHA-256 in attestation | ~300 cycles/byte software implementation; acceptable for enclave creation (one-shot) |

**Recommendation:** Add `rdtime`-based microbenchmarks for the three highest-frequency paths: context switch, syscall round-trip, and NES node dispatch.

---

## 7. DevOps Report

### DevOps Readiness Score: 12 / 100

| Category | Score | Notes |
|---|---|---|
| CI/CD pipelines | 0/25 | No `.github/` directory |
| Build reproducibility | 8/15 | Toolchain pinned by channel, not by date |
| Deployment automation | 5/15 | Single `make run` command; no artifact archiving |
| Monitoring/observability | 5/15 | UART output only; no structured logging |
| Rollback strategy | 4/15 | Git tags enable manual rollback; no automation |
| Nightly QEMU smoke test | 0/15 | Not possible without CI |

### Top 3 Immediate Actions

1. **Create `.github/workflows/ci.yml`** — build + clippy + fmt on every push to `develop`. Under 30 lines of YAML. Unblocks all other quality gates.
2. **Fix clippy and enforce `-D warnings`** — ✅ Clippy error fixed this session. Wire to CI.
3. **Pin nightly date** — Change `channel = "nightly"` to `channel = "nightly-YYYY-MM-DD"` for build reproducibility.

---

## 8. Documentation Report

### Documentation Completeness Score: 82 / 100

| Document | Status | Quality |
|---|---|---|
| `README.md` | ✅ Present | Good — pitch, quick start, roadmap links |
| `CONTRIBUTING.md` | ✅ Present | Good — branch model, commit format, syscall guide |
| `CHANGELOG.md` | ✅ Present | Good — all releases documented |
| `ROADMAP.md` | ✅ Present | Good — phase status table |
| `docs/DESIGN.md` | ✅ Present | Excellent — 935-line unified reference |
| `docs/ARCHITECTURE.md` | ✅ Present | Good |
| `docs/PHASE_01-12_DESIGN.md` | ✅ All 12 present | Good — mermaid diagrams, code, syscall specs |
| `docs/INDEX.md` | ✅ Present | Good — audience routing hub |
| `SECURITY.md` | ❌ Missing | No responsible disclosure policy |
| `workspace.package.version` | ⚠️ Wrong | Shows `0.1.0`, not `0.12.1` |
| `tests/` directory | ❌ Missing | Documented in version_control.md but absent |
| API/SBI reference | ✅ Covered | In DESIGN.md §15 and phase docs |
| Runbooks / troubleshooting | ⚠️ Partial | ONBOARDING.md has gotchas; no structured runbook |

---

## 9. SDLC Report

### SDLC Maturity: Level 2 / 5

The project has Level 3 documentation (branch strategy, commit format, CONTRIBUTING guide, CHANGELOG discipline) but Level 1 tooling enforcement (no CI, no automated quality gates, honor-system only). To reach Level 3, tooling must enforce the documented standards.

### Key Gaps

- `workspace.package.version = "0.1.0"` — misleading; should be `0.12.1`
- No CI pipeline to enforce stated code standards
- Large multi-subsystem commits on `develop` instead of atomic feature branches
- ~25% of commits use `feat:` prefix instead of the project-defined subsystem token
- No `SECURITY.md`

---

## 10. Production Readiness Assessment

### Can this project be deployed today?
**Yes — to QEMU for research/demo purposes only.**
`make run` boots cleanly, all 7 verification programs pass, and Phase 12 TEE attestation works end-to-end.

### Is it production-ready?
**No.** The DTB parser has unvalidated pointer arithmetic over untrusted input, the kernel memory map is uniformly RWX, capability rights can be amplified, and the device key is a compile-time constant. These are not edge cases — they are load-bearing security properties that are absent.

### Is it MVP-ready?
**No, but close for its defined research MVP scope.** 3–4 sprints of targeted work would address the critical gaps.

### Is it enterprise-ready?
**No.** Zero unit tests, no CI, no formal verification, no production-grade attestation, no side-channel protections. Enterprise readiness is several roadmap phases away.

### Blockers and Estimated Effort

| Blocker | Effort |
|---|---|
| CI/CD pipeline setup | 2 days |
| DTB parser security fix | 1 day |
| R/W/X kernel memory map | 2 days |
| Capability rights derivation | 3 days |
| PMP `PMP_L` + `grant_region` X removal | 1 day |
| User pointer validation in cluster.rs | 1 day |
| Workspace version consistency | 2 hours |
| SAFETY comment coverage to 80%+ | 3 days |
| **Total Sprint 1+2** | **~2.5 weeks** |

---

## 11. Next Sprint Plan

**Sprint Goal:** "CI-enabled, clippy-clean, and critical security blockers resolved"

### User Stories

| Story | Acceptance Criteria |
|---|---|
| US-1: Build gate on every push | `.github/workflows/ci.yml` triggers on push/PR; `cargo build` passes in CI |
| US-2: Clippy clean (✅ unblocked) | `cargo clippy -D warnings` exits 0; CI step added |
| US-3: Pin nightly date | `rust-toolchain.toml` uses `nightly-YYYY-MM-DD`; CI reproducible |
| US-4: Fix DTB parser | Bounds-checked parser; `transmute` removed; no UB path |
| US-5: R/W/X memory map | `.text`→RX, `.rodata`→R, heap/data→RW; verified boot |
| US-6: Version consistency | `workspace.package.version = "0.12.1"`; CHANGELOG entry |
| US-7: Capability rights derivation | `derive_handle()` implemented; `spawn()` uses it; amplification impossible |

### Definition of Done
- [ ] `cargo build` — zero errors
- [ ] `cargo clippy -D warnings` — zero diagnostics
- [ ] `cargo fmt --check` — zero diff
- [ ] `.github/workflows/ci.yml` — green on HEAD
- [ ] DTB parser — all pointer arithmetic bounds-checked
- [ ] Memory map — section-separated R/W/X flags
- [ ] `derive_handle()` — rights amplification impossible
- [ ] `workspace.package.version = "0.12.1"`
- [ ] All changes via `feature/sprint-1-security-ci` branch → `develop` → `main`
- [ ] `v0.13.0` tagged on `main`

### Risk Register

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| R/W/X memory map breaks boot | Medium | High | Test incrementally; add `.rodata` first, then `.text` RX |
| Nightly date unavailable on CI runner | Medium | Medium | Test 3 recent dates; document chosen date |
| `derive_handle()` breaks existing spawn paths | Low | High | Add a `spawn_with_full_rights()` escape hatch for kernel-internal use only |

---

## Final Decision

> **APPROVED FOR STAGING ONLY**

**Justification:**

The kernel boots reliably, all 12 phases produce correct UART verification output, the M-mode TEE isolation is structurally sound, the documentation is production-grade, and the SDLC process is well-defined. This project is unambiguously development-complete for its research scope.

It cannot be approved for MVP production deployment because four critical security properties are absent: the DTB parser performs unvalidated pointer arithmetic over untrusted input (memory corruption vector), all physical RAM is mapped RWX (code injection via any write primitive), capability rights can be amplified without a parent check (privilege escalation), and the TEE device key is a compile-time constant (attestation forgery). Additionally, zero unit tests and zero CI pipelines mean regressions are invisible until manual QEMU execution.

These are not cosmetic issues — they are fundamental to the security model the project is explicitly designed to provide. Two to three focused sprints addressing the items in the remediation plan above would bring this project to MVP production readiness for a research/demo deployment target.
