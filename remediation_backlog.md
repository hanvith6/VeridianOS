# VeridianOS Remediation Backlog

Generated: 2026-05-30 | Source: docs/AUDIT_REPORT.md

## Status Legend
- 🔴 OPEN | 🟡 IN PROGRESS | ✅ DONE | ⛔ BLOCKED

---

## CRITICAL (fix before any other work)

| ID | Status | Finding | File | Sprint |
|----|--------|---------|------|--------|
| C-1 | ✅ DONE | `m_trap_handler` not marked `unsafe` — clippy fails | `monitor/src/main.rs:338` | S1 |
| C-2 | 🟡 IN PROGRESS | DTB parser: unvalidated pointer arithmetic over untrusted input | `kernel/src/main.rs:348-449` | S1 |
| C-3 | 🔴 OPEN | All RAM mapped RWX — `.text` writable, heap executable | `kernel/src/memory/mod.rs:62-76` | S1 |
| C-4 | 🔴 OPEN | Rights amplification: `Handle::new()` takes arbitrary rights | `kernel/src/capability/mod.rs:59` | S1 |

---

## HIGH

| ID | Status | Finding | File | Sprint |
|----|--------|---------|------|--------|
| H-1 | 🔴 OPEN | `domain_join/list/status` dereference unvalidated user pointers | `kernel/src/dist/cluster.rs:200,231,264` | S2 |
| H-2 | 🔴 OPEN | `lock_region()` never sets `PMP_L` — enclave entries not permanently locked | `monitor/src/pmp.rs:169` | S2 |
| H-3 | 🔴 OPEN | `grant_region()` grants `PMP_X` during binary loading | `monitor/src/pmp.rs:234` | S2 |
| H-4 | 🔴 OPEN | `ENCLAVE_POOL` is `pub static mut` — data race on SMP | `monitor/src/enclave.rs:110` | S2 |
| H-5 | 🔴 OPEN | `HART_STATES` mutable static; concurrent hart access UB | `monitor/src/main.rs:121` | S2 |
| H-6 | 🔴 OPEN | `VirtioNetTransport::send` reads hardcoded 64 bytes regardless of struct size | `kernel/src/dist/transport.rs:57` | S2 |
| H-7 | 🔴 OPEN | SUM bit permanently enabled — S-mode reads all user pages | `kernel/src/trap.rs:32` | S2 |
| H-8 | 🔴 OPEN | `HandleTable::set()` silently overwrites handles; no generation counter | `kernel/src/capability/mod.rs:91` | S2 |

---

## MEDIUM

| ID | Status | Finding | File | Sprint |
|----|--------|---------|------|--------|
| M-1 | 🔴 OPEN | ASLR disabled (TODO comment) | `kernel/src/process/mod.rs:109` | S3 |
| M-2 | 🔴 OPEN | Secondary harts have PMP self-protection window gap | `monitor/src/main.rs:166` | S3 |
| M-3 | 🔴 OPEN | Cluster messages have all-zero MAC — unauthenticated | `kernel/src/dist/cluster.rs:150` | S3 |
| M-4 | 🔴 OPEN | Hardcoded VMO addresses not via buddy allocator | `kernel/src/process/mod.rs:253` | S3 |

---

## LOW

| ID | Status | Finding | File |
|----|--------|---------|------|
| L-1 | 🔴 OPEN | `#![allow(unused_unsafe)]` in monitor binary | `monitor/src/main.rs:22` |
| L-2 | 🔴 OPEN | Dead test threads with unsafe CSR manipulation silenced | `kernel/src/main.rs:57,79` |
| L-3 | 🔴 OPEN | `static mut TEST_BUF` aliasing hazard | `kernel/src/memory/page_alloc.rs:253` |

---

## CI/CD (no .github/ exists)

| ID | Status | Task | Sprint |
|----|--------|------|--------|
| CI-1 | 🔴 OPEN | Create `.github/workflows/ci.yml` — build + clippy + fmt | S1 |
| CI-2 | 🔴 OPEN | Pin nightly date in `rust-toolchain.toml` | S1 |
| CI-3 | 🔴 OPEN | Add QEMU smoke test to CI (requires self-hosted or qemu action) | S3 |

---

## QA (0 unit tests, 0 #[test] functions)

| ID | Status | Task | Sprint |
|----|--------|------|--------|
| QA-1 | 🔴 OPEN | `rights_test` user program — verifies `-EACCES` on attenuated handles | S2 |
| QA-2 | 🔴 OPEN | `syscall_robustness_test` — null/invalid args to every syscall | S2 |
| QA-3 | 🔴 OPEN | Buddy allocator `#[cfg(test)]` unit tests | S2 |
| QA-4 | 🔴 OPEN | Enclave pool exhaustion + PMP overlap tests | S2 |
| QA-5 | 🔴 OPEN | Exception delivery negative path test | S3 |

---

## DOCS

| ID | Status | Task |
|----|--------|------|
| D-1 | 🔴 OPEN | Create `SECURITY.md` (responsible disclosure) |
| D-2 | 🔴 OPEN | `workspace.package.version` already fixed to `0.12.1` ✅ |

---

## Execution Order (this session)

```
CYCLE 1:  C-2 DTB parser fix  →  build  →  checkpoint
CYCLE 2:  C-3 RWX memory map  →  build  →  boot verify  →  checkpoint
CYCLE 3:  C-4 Rights amplification  →  build  →  checkpoint
CYCLE 4:  CI-1 CI/CD pipeline  →  CI-2 pin nightly  →  checkpoint
CYCLE 5:  H-1 user ptr validation  →  H-2 PMP_L  →  H-3 grant_region X  →  build
CYCLE 6:  H-4 ENCLAVE_POOL mutex  →  H-5 HART_STATES atomic  →  build
CYCLE 7:  QA-1 rights_test  →  QA-2 syscall_robustness_test  →  QA-3 allocator tests
CYCLE 8:  D-1 SECURITY.md  →  readiness check  →  SESSION_STATE update
```
