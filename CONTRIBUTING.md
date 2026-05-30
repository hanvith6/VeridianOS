# Contributing to VeridianOS

VeridianOS is an experimental AI-native capability-based microkernel. Contributions that
advance the architecture, add well-tested subsystems, or improve documentation are welcome.
Read this guide fully before opening a PR.

---

## Quick Contribution Checklist

Every pull request must satisfy all five of these before review:

- **Targets `develop`, not `main`** — `main` is stable-only; all work merges to `develop` first.
- **Builds cleanly** — `cargo build` passes with zero errors and zero warnings.
- **Tests pass** — `make run` produces the expected UART verification log for the affected subsystem.
- **Commit messages follow the format** — `subsystem: short description` (see below).
- **New subsystem or syscall ships with a design doc** — either a new `PHASE_XX_DESIGN.md` or an
  update to the relevant existing one.

---

## Branch Model

VeridianOS uses a five-branch strategy. See `docs/version_control.md` for full rationale.

| Branch | Purpose | Rules |
| :--- | :--- | :--- |
| `main` | Stable public releases and milestone tags | Never push directly; only merge tested milestones from `develop` |
| `develop` | Primary integration branch | All features merge here first; must compile at all times |
| `feature/*` | Isolated implementation work | One feature per branch; merge into `develop` after testing |
| `experimental/*` | High-risk architecture experiments | Allowed to fail; never merge directly into `main` |
| `research/*` | Design investigations, pseudocode, diagrams | May contain non-compiling prototypes |

**Tag milestones** on `main` using the `v0.X.Y` semantic versioning scheme defined in
`docs/version_control.md`. Never jump to `v1.0.0` until architecture stabilizes.

```text
# Good branch names
feature/virtio-block-driver
feature/sgf-hash-index
experimental/distributed-capability-transfer
research/m-mode-tee-v2
```

---

## Commit Message Format

Follow the `subsystem: description` convention used throughout the project history.

```text
<subsystem>: <imperative, present-tense description under 72 characters>

[Optional body explaining why, not what. Wrap at 72 characters.]
```

**Subsystem tokens:**

| Token | Covers |
| :--- | :--- |
| `kernel` | Core kernel init, boot, trap entry |
| `memory` | Physical allocator, SV39 paging, VMOs |
| `process` | Process/thread lifecycle, ELF loader |
| `capability` | Handle table, rights enforcement |
| `syscall` | Syscall dispatch, numbers.rs |
| `ipc` | Channels, capability transfer |
| `scheduler` | Thread scheduler, priority queues |
| `nes` | Neural Execution Subsystem (Phase 7) |
| `sgf` | Semantic Graph Filesystem (Phase 8) |
| `agent` | Agent runtime (Phase 9) |
| `monitor` | M-mode security monitor |
| `docs` | Design documents, README, CHANGELOG |
| `build` | Makefile, Cargo.toml, toolchain |
| `test` | Verification binaries in `user_programs/` |

```text
# Good
sgf: add O(1) hash index for ObjectType lookups
capability: enforce Rights::DUPLICATE on handle clone
memory: fix off-by-one in SV39 PTE flag encoding

# Bad
update
fixed stuff
WIP changes
```

---

## Build and Test Before Submitting

Verify your changes compile and produce correct output before opening a PR.

**Build the kernel:**
```bash
cargo build
```

Expected: zero errors, zero warnings. Treat all warnings as errors — the CI pipeline does.

**Run in QEMU:**
```bash
make run
```

For a Phase 8 SGF change, the UART output should include the semantic_test verification
sequence ending with:

```
[SEMANTIC_FS] Semantic Knowledge Graph Filesystem Initialized.
[USER] Starting Semantic Knowledge Graph Filesystem Verification program...
[USER] Created Document node capability successfully.
[USER] Created Blob node capability successfully.
[USER] Wrote text content into Blob node VMO successfully.
[USER] Successfully queried Document node ID by properties.
[USER] Added directed edge (Document -[Contains]-> Blob) successfully.
[USER] Relational edge query verification SUCCESS!
[USER] Semantic Knowledge Graph Filesystem Verification SUCCESS!
```

If your change touches a different subsystem, confirm its corresponding verification
binary exits cleanly with no `[ERROR]` or `[FAIL]` lines in the UART log.

---

## Adding a New Syscall

1. **Register the syscall number** in `kernel/src/syscall/numbers.rs`. Append to the
   existing `SYS_*` const block; never reuse or renumber existing constants.

2. **Implement the handler** in the relevant subsystem module
   (e.g., `kernel/src/semantic_graph/syscalls.rs`). The function signature must:
   - Accept only `usize` arguments (the raw register values from `a0`–`a4`).
   - Return `isize` (zero or positive for success; negative errno for failure).
   - Include a `// SAFETY:` comment on every `unsafe` block.

3. **Wire into the dispatcher** in `kernel/src/syscall/handler.rs`. Add a match arm:
   ```rust
   SYS_YOUR_CALL => subsystem::syscalls::sys_your_call(a0, a1, a2),
   ```

4. **Write a test binary** under `user_programs/your_subsystem_test/src/main.rs` that
   exercises the syscall and prints a clear pass/fail line. Add it to the workspace
   `Cargo.toml` and confirm it runs via `make run`.

---

## Adding a New Phase

New phases represent major architectural milestones. Follow this scaffolding:

1. **Create the design document** at `docs/PHASE_XX_DESIGN.md` before writing any code.
   Model it after `docs/PHASE_08_DESIGN.md`: include a metadata table, executive summary,
   Mermaid architecture diagram, Rust struct definitions, syscall ABI table, and expected
   UART verification traces.

2. **Scaffold the kernel module** at `kernel/src/<subsystem>/mod.rs`. Export an `init()`
   function that prints a single `[SUBSYSTEM] Initialized.` line to UART on boot.

3. **Add the verification binary** at `user_programs/<subsystem>_test/src/main.rs`.
   The binary must be `#![no_std]` and `#![no_main]`, use only raw `ecall` syscalls,
   and exit with code `0` on success.

4. **Update `ROADMAP.md` and `CHANGELOG.md`** to mark the new phase and list its syscall
   numbers. Update the phase version section in `docs/version_control.md`.

---

## Code Standards

All kernel code must follow these rules without exception:

- **`#![no_std]`** throughout the kernel crate — no heap allocations, no standard library.
- **No heap in the monitor** — `monitor/` (M-mode security monitor) must never allocate.
  Use only stack variables and static arrays.
- **`unsafe` requires `// SAFETY:` comments** — every `unsafe` block must have an inline
  comment explaining which invariant is being upheld and why it holds. Reviewers will
  reject `unsafe` blocks without justification.
- **Static slab allocation** — represent kernel resource pools as fixed-size static arrays
  (`[T; N]`) with slot allocation flags. Never use `Vec` or `Box` inside the kernel.
- **`repr(C)` for ABI-crossing structs** — any struct shared between kernel and user space
  via a pointer must be `#[repr(C)]`.
- **Capability checks before every resource access** — every syscall handler must validate
  that the calling process holds the required `Rights` flag on the relevant `Handle` before
  touching the underlying kernel object.

---

## Opening a PR

1. Push your branch to the repo and open a PR against `develop`, never `main`.
2. Title the PR with the subsystem and a concise description:
   `sgf: add hash index for O(1) ObjectType lookups`
3. In the PR body, link the relevant phase design document and paste the actual UART
   output from `make run` showing the verification binary passing.
4. Assign the PR to yourself and add the `kernel` or `docs` label as appropriate.
5. Address all review comments before requesting re-review. Do not force-push after
   review has started; add fixup commits instead.

Questions? Open a GitHub Discussion rather than an issue.
