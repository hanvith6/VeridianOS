# VeridianOS Version Control Strategy

## Overview

This document defines the long-term version control and repository management strategy for VeridianOS.

Goal:

* Maintain clean architecture evolution
* Support experimental kernel development safely
* Build a strong public GitHub presence
* Enable future contributors to understand the system
* Prevent unstable experiments from breaking stable builds

---

# Repository Philosophy

VeridianOS is not just a hobby kernel.

It should evolve like:

* a systems research project
* a long-term operating system experiment
* an AI-native kernel architecture platform

Version control must reflect:

* stability
* research evolution
* architecture maturity
* experimental isolation

---

# Branch Strategy

## Main Branches

```text
main
develop
feature/*
experimental/*
research/*
```

---

# Branch Definitions

## `main`

Purpose:

* Stable public releases
* Demonstration-ready builds
* Clean showcase branch

Rules:

* Never push unstable code directly
* Only merge tested milestones
* Always boot successfully

Examples:

```text
v0.1.0 Kernel boot
v0.2.0 Virtual memory
v0.3.0 Userspace transition
```

---

## `develop`

Purpose:

* Primary active development branch
* Integration testing
* Daily progress branch

Rules:

* All features merge here first
* Must compile frequently
* Used before merging into `main`

---

## `feature/*`

Purpose:

* Isolated implementation work
* Stable feature-focused development

Examples:

```text
feature/virtual-memory
feature/elf-loader
feature/capability-ipc
feature/virtio-blk
feature/process-scheduler
```

Rules:

* One feature per branch
* Merge into `develop` after testing

---

## `experimental/*`

Purpose:

* High-risk architecture experiments
* Unsafe or unstable ideas
* Research prototypes

Examples:

```text
experimental/machine-mode
experimental/neural-scheduler
experimental/semantic-fs
experimental/distributed-runtime
```

Rules:

* Allowed to break
* Never merge directly into `main`
* Used for exploration

---

## `research/*`

Purpose:

* Architecture investigations
* Design experiments
* Prototype concepts
* Academic-style system exploration

Examples:

```text
research/capability-model
research/semantic-execution
research/ai-native-kernel
research/runtime-abstractions
```

Contents may include:

* design notes
* pseudocode
* diagrams
* papers
* prototype implementations

---

# Versioning Strategy

## Semantic Versioning

Format:

```text
MAJOR.MINOR.PATCH
```

Example:

```text
v0.4.2
```

Where:

* MAJOR = architectural generation
* MINOR = major feature milestone
* PATCH = fixes/improvements

---

# Recommended Evolution Roadmap

## Phase 0 — Foundation

```text
v0.1.0 Boot sequence
v0.2.0 Trap handling
v0.3.0 Memory management
v0.4.0 User-mode support
v0.5.0 ELF loader
```

---

## Phase 1 — Research Kernel

```text
v1.0.0 Stable research kernel
v1.1.0 Capability IPC
v1.2.0 Scheduler improvements
v1.3.0 Semantic runtime prototype
```

---

## Phase 2 — Runtime Layer

```text
v2.0.0 VeridianSBI introduced
v2.1.0 Runtime abstraction layer
v2.2.0 Hardware ownership expansion
```

---

## Phase 3 — Advanced System Control

```text
v3.0.0 Hybrid machine-mode architecture
v3.5.0 Experimental full M-mode runtime
```

---

# Commit Message Style

## Bad Examples

```text
update
fixed stuff
changes
```

---

## Good Examples

```text
kernel: initialize trap vector
memory: implement SV39 mapper
process: add ELF user loader
virtio: add block queue setup
capability: validate channel rights
scheduler: introduce priority queues
```

---

# Recommended Repository Structure

```text
.github/
design/
docs/
kernel/
user_programs/
research/
scripts/
tools/
tests/
```

---

# Documentation Strategy

## Required Public Documents

### README.md

Should contain:

* project overview
* architecture summary
* screenshots/GIFs
* build instructions
* roadmap
* current status

---

### ROADMAP.md

Tracks:

* completed features
* upcoming goals
* experimental directions

Example:

```text
[✓] Kernel boot
[✓] UART output
[✓] Virtual memory
[ ] Capability IPC
[ ] SMP support
[ ] Semantic scheduler v2
```

---

### CHANGELOG.md

Track every public release.

Example:

```text
## v0.4.0

Added:
- User-mode transitions
- ELF loading

Improved:
- Trap handling

Fixed:
- Page fault during stack setup
```

---

# Release Discipline

Do NOT release every commit.

Release only when:

* the system boots reliably
* a milestone is complete
* architecture changed significantly
* demos are possible

---

# GitHub Strategy

## Public Positioning

VeridianOS should be presented as:

```text
Experimental AI-native capability-based operating system research platform
```

NOT:

```text
Linux replacement
```

---

# Public Development Strategy

## Share:

* boot progress
* architecture diagrams
* scheduler evolution
* semantic graph concepts
* runtime experiments
* capability model updates

---

# Important Engineering Rules

## Rule 1 — Never Destroy Stable Builds

Stable builds are critical.

Always isolate:

* scheduler rewrites
* memory rewrites
* machine-mode experiments
* runtime redesigns

inside dedicated branches.

---

## Rule 2 — Document Architecture Decisions

Every major subsystem should explain:

* why it exists
* why the design was chosen
* tradeoffs
* future direction

---

## Rule 3 — Keep Experimental Work Separate

Experimental branches are expected to:

* fail
* crash
* redesign frequently

This is normal.

---

# Long-Term Vision

VeridianOS should evolve through:

```text
Supervisor-mode research kernel
→ capability-first architecture
→ semantic execution runtime
→ custom runtime layer
→ hybrid machine-mode control
→ advanced AI-native systems platform
```

---

# Development Stage Classification

## Current Stage

VeridianOS is currently in:

```text
Design Phase 12 Complete (M-Mode TEE Security Monitor)
```

This means the project is still in:

* architecture exploration
* subsystem stabilization
* interface experimentation
* runtime evolution
* kernel structure refinement

At this stage:

* breaking changes are normal
* redesigns are expected
* subsystem replacement is acceptable
* experimentation is encouraged

The goal is NOT production stability yet.

The goal is:

* discovering the best architecture
* validating kernel concepts
* evolving long-term system direction

---

# Version Control During Design Phases

## Important Principle

Early-stage operating systems change constantly.

Therefore:

```text
Version control should preserve evolution history
NOT freeze architecture too early
```

---

# Recommended Design-Phase Version Model

## Architecture Versions

During early phases, use:

```text
v0.x.y
```

ONLY.

Never jump to:

```text
v1.0.0
```

until:

* architecture stabilizes
* boot process becomes reliable
* memory model stabilizes
* subsystem boundaries are mostly fixed

---

# Recommended Current Structure

## Design Generations

### Phase 1 — Boot Experiments

```text
v0.1.x
```

Examples:

```text
v0.1.0 Initial boot
v0.1.1 UART output
v0.1.2 Trap entry fixes
```

---

### Phase 2 — Memory Foundations

```text
v0.2.x
```

Examples:

```text
v0.2.0 Physical page allocator
v0.2.1 SV39 paging
v0.2.2 Virtual mapping cleanup
```

---

### Phase 3 — Userspace Transition

```text
v0.3.x
```

Examples:

```text
v0.3.0 User-mode entry
v0.3.1 ELF loading
v0.3.2 Syscall routing
```

---

### Phase 4 — Capability Architecture

```text
v0.4.x
```

Examples:

```text
v0.4.0 Capability channels
v0.4.1 Rights validation
v0.4.2 IPC redesign
```

---

### Phase 5 — Semantic Runtime

```text
v0.5.x
```

Examples:

```text
v0.5.0 Semantic graph runtime
v0.5.1 Dependency validation
v0.5.2 Queue redesign
```

---

### Phase 6 — Scheduler Evolution

```text
v0.6.x
```

Examples:

```text
v0.6.0 Priority scheduling
v0.6.1 Neural scheduling experiments
v0.6.2 Runtime coordination
```

---

### Phase 7 — Heterogeneous Neural Scheduler

```text
v0.7.x
```

Examples:

```text
v0.7.0 Heterogeneous task queue abstractions
v0.7.1 DAG task scheduler framework
v0.7.2 Simulated execution profiling
```

---

### Phase 8 — Semantic Knowledge Graph Filesystem

```text
v0.8.x
```

Examples:

```text
v0.8.0 Kernel graph filesystem database
v0.8.1 Relationship traversal system calls
v0.8.2 Secure VMO data attachment to nodes
```

---

### Phase 9 — Agent Runtime

```text
v0.9.x
```

Examples:

```text
v0.9.0 Agent processes isolation in kernel space
v0.9.1 Bidirectional agent IPC channels
v0.9.2 Hierarchical agent spawning and lifecycles
```

---

### Phase 10 — Self-Improving Kernel Policies

```text
v0.10.x
```

Examples:

```text
v0.10.0 Online latency profiling via rdtime CSR
v0.10.1 Epsilon-greedy reinforcement learning device router
v0.10.2 Online Exponential Moving Average estimation
```

---

### Phase 11 — Distributed Coherence

```text
v0.11.x
```

This is the current development stage.

Examples:

```text
v0.11.0 Atomic SPSC lock-free ring buffers (DKCP)
v0.11.1 Distributed Capabilities Transfer Protocol (DCTP)
v0.11.2 S-mode Raft consensus engine replication
```

---

# Handling Large Architecture Changes

## DO NOT Rewrite History

Never delete old architecture evolution.

Instead:

* keep commit history
* preserve experimental branches
* archive old implementations
* document why redesigns happened

This is valuable publicly.

People like seeing evolution.

---

# Recommended Strategy for Old Builds

## Create Archive Tags

Examples:

```text
archive/boot-prototype
archive/old-scheduler
archive/pre-capability-redesign
archive/mode-transition-tests
```

This preserves historical system evolution.

---

# Architecture Snapshots

For major redesigns:

Create:

```text
snapshots/
```

Example:

```text
snapshots/
├── scheduler-v1/
├── semantic-runtime-v1/
└── capability-model-v1/
```

These help:

* future contributors
* debugging regressions
* architecture comparison
* research documentation

---

# Recommended Git Tags

Tag every meaningful milestone.

Examples:

```text
boot-working
paging-enabled
first-userspace
first-capability-ipc
semantic-runtime-v1
```

This is EXTREMELY useful later.

---

# How Mature Projects Handle This

Serious systems projects preserve:

* old schedulers
* old runtimes
* experimental kernels
* architecture pivots

because:

```text
Operating systems evolve through redesign cycles
```

not linear development.

---

# Most Important Rule During Design Phase

## Preserve Architectural History

Your repository is not only code.

It is also:

* a systems research timeline
* an engineering evolution log
* proof of long-term thinking
* a public learning journey

That history becomes valuable later.

---

# Final Principle

The goal is not only to build an operating system.

The goal is to:

* build a credible systems research project
* create a strong public engineering narrative
* develop long-term architecture discipline
* attract contributors and systems communities
* establish technical reputation through consistency
 