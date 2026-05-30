# 📚 VeridianOS — Academic & Research Foundations

VeridianOS is built on a foundation of cutting-edge research in operating system architecture, formal verification, language safety, and hardware-software co-design. This document organizes the peer-reviewed papers, books, and industry studies that directly inspire and guide the design of VeridianOS.

---

## 1. AI-Native & Accelerator-First Kernels

Traditional operating systems treat GPUs, TPUs, and NPUs as external, opaque I/O peripherals accessed via user-space library drivers. VeridianOS treats accelerators as primary execution targets scheduled directly by the microkernel.

### LithOS: An Operating System for Efficient Machine Learning on GPUs
* **Venue:** SOSP '25
* **Core Contribution:** LithOS proposes a clean-slate "GPU OS" architecture that bypasses standard driver latency bottlenecks. It introduces a Texture Processing Cluster (TPC) spatial-sharing scheduler, kernel atomization (splitting long kernels to prevent Head-of-Line blocking), and dynamic device-level rightsizing.
* **Influence on VeridianOS:** Guides our Phase 7 **Neural Scheduler** design, specifically showing how direct spatial scheduling of execution graphs on hardware cores yields orders-of-magnitude lower latency compared to monolithic CPU-driver interaction.

### AIOS: LLM Agent Operating System
* **Venue:** COLM '25 (Rutgers University)
* **Core Contribution:** Explores treating Large Language Models and autonomous AI agents as primary system processes. AIOS designs scheduler abstractions for concurrent agent loops, mitigating LLM context window swapping delays.
* **Influence on VeridianOS:** Informs our native user-space **Agent Runtime** layer, ensuring the OS can manage agent loops, allocate context windows, and schedule execution graphs without application-level runtime overhead.

### Learning Scheduling Algorithms for Data Processing Clusters (Decima)
* **Venue:** ACM SIGCOMM '19
* **Core Contribution:** Decima uses Graph Neural Networks and Reinforcement Learning to automatically learn scheduling policies for Directed Acyclic Graphs (DAGs) of tasks in parallel computing clusters.
* **Influence on VeridianOS:** Validates representing AI workloads as Task Graphs (DAGs) and scheduling them dynamically. Inspires our S-Mode low-overhead scheduling.

### Bandit-Based Scheduling for Heterogeneous CPU-GPU Architectures
* **Venue:** Journal of Systems Architecture '22
* **Core Contribution:** Applies Multi-Armed Bandit (MAB) algorithms with epsilon-greedy exploration to assign compute tasks to heterogeneous CPU and GPU cores.
* **Influence on VeridianOS:** Directly supports our Phase 10 online EMA learning and epsilon-greedy dynamic routing engine.

---

## 2. Capability-Based Security

VeridianOS replaces ambient authority (hierarchical folders, User IDs, and `root` privileges) with an unforgeable, kernel-managed capability system where every action requires holding a discrete cryptographic token.

### seL4: Formal Verification of an OS Kernel
* **Venue:** SOSP '09 (with ongoing proofs through USENIX ATC/OSDI '23-'25)
* **Core Contribution:** The pioneering microkernel that mathematically proved functional correctness (proof that C code matches Isabelle/HOL specifications) and security properties (integrity, confidentiality, and isolation).
* **Influence on VeridianOS:** Provides the core mathematical model for our `Handle` and `HandleTable` abstractions. seL4 proved that capability-based microkernels can achieve bulletproof security without sacrificing performance.

### LionsOS: A Composable seL4-based Operating System
* **Venue:** arXiv / UNSW '24
* **Core Contribution:** Builds a modular, clean-slate operating system framework on top of seL4, proving that drivers, filesystems, and networking can be organized as composable user-space processes (cells) communicating via verified IPC channels.
* **Influence on VeridianOS:** Serves as the blueprint for VeridianOS's user-space driver model, verifying that our VirtIO drivers can run entirely in U-mode.

---

## 3. Rust-Based Kernel Engineering

Writing operating systems in C/C++ introduces systemic memory-safety vulnerabilities. VeridianOS uses Rust's type system to enforce safety at compile time and separate the microkernel's unsafe core from safe abstractions.

### Asterinas: A Linux ABI-Compatible, Rust-Based Framekernel OS
* **Venue:** USENIX ATC '25
* **Core Contribution:** Details the "Framekernel" architecture. Unlike monolithic kernels with sparse safety checks, Asterinas partitions the OS into a minimal, unsafe Trusted Computing Base (TCB) framework and a large body of verified safe components, maintaining full Linux ABI compatibility.
* **Influence on VeridianOS:** Guides the architecture of our physical memory and page table modules, confirming that a production-grade kernel can restrict `unsafe` code to less than 5% of the codebase.

### An Empirical Study of Rust-for-Linux: Success, Dissatisfaction, and Compromise
* **Venue:** USENIX ATC '24 (Best Paper)
* **Core Contribution:** A comprehensive analysis of the real-world friction between Rust language safety and C-based legacy interfaces inside the Linux kernel. It highlights the complexities of creating safe abstractions around asynchronous locking and raw page tables.
* **Influence on VeridianOS:** Justifies our clean-slate, Rust-first design. By rejecting the Unix/C legacy codebase entirely, VeridianOS avoids the compromise of wrapping unsafe C APIs, allowing the Rust type system to operate natively at the lowest levels of the kernel.

---

## 4. Unikernels & Library Operating Systems

For virtualized and cloud-native environments, general-purpose kernels carry unnecessary complexity and vulnerability surface.

### μFork: Supporting POSIX fork Within a Single-Address-Space OS
* **Venue:** SOSP '25
* **Core Contribution:** Explores maintaining developer familiarity (e.g., POSIX `fork`) inside modern library OSs and single-address-space environments without cloning heavy virtual address trees.
* **Influence on VeridianOS:** Inspires the capability transfer mechanisms in our `process::spawn` and IPC layer.

### Unikraft: Fast, Lightweight, and Safe Virtual Machines
* **Venue:** EuroSys '21 / Ongoing Integration '25
* **Core Contribution:** A fully modular library OS toolchain that allows developers to build tailored unikernels by compiling only the necessary hardware and library layers. In 2025, the integration of a Unikraft backend with OCaml's MirageOS proved that multi-language library OS stacks can achieve near-zero boot overhead.
* **Influence on VeridianOS:** Guides the modular structure of our workspace crates, enabling VeridianOS to compile down to a hyper-optimized library OS for virtual machines.

---

## 5. Quantum & Neuromorphic Computing OS Stacks

Looking ahead to the next 30 years, computing will rely on architectures that depart entirely from the von Neumann model.

### QOS: A Quantum Operating System
* **Venue:** arXiv / ACM '25
* **Core Contribution:** Proposes "Qernel" — a hardware-agnostic OS kernel abstraction for NISQ-era quantum processors. QOS manages multi-programming scheduling and circuit placement on QPUs, achieving a **456.5× increase in execution fidelity** and **9.6× higher resource utilization**.
* **Influence on VeridianOS:** Forms the basis of our long-term architectural planning. The VeridianOS capability model is designed to easily map quantum execution queues as kernel objects, enabling hybrid classical-quantum scheduling.

### Neuromorphic Intermediate Representation (NIR) & Event-Stream Runtimes
* **Venue:** Open Neuromorphic Consortium '24-'25
* **Core Contribution:** A unified programming abstraction and Intermediate Representation (IR) that compiles Spiking Neural Networks (SNNs) onto heterogeneous neuromorphic hardware (e.g., Intel Loihi 2, SpiNNaker 2, IBM NorthPole).
* **Influence on VeridianOS:** Informs how the VeridianOS scheduler interacts with neuromorphic processors. Event-stream data from sensors is routed directly into scheduler queues as asynchronous kernel interrupts, bypassing standard linear framebuffers.

---

## 6. Trusted Execution Environments & Attestation

Phase 12 of VeridianOS introduces hardware-enforced enclave isolation via a RISC-V Security Monitor and PMP-based memory protection. The references in this section are the primary academic and standards foundations for that implementation.

### Keystone: An Open Framework for Architecting TEEs
* **Venue:** USENIX Security Symposium 2020
* **Authors:** Dayeol Lee, David Kohlbrenner, Shweta Shinde, Krste Asanović, Dawn Song (UC Berkeley)
* **Core Contribution:** Keystone is the first open-source TEE framework targeting RISC-V. It uses M-mode Physical Memory Protection (PMP) registers and a minimal (~2,000 LoC C) Security Monitor to create hardware-isolated enclaves without any proprietary hardware extensions. The SM sits below OpenSBI in the boot stack and interposes on all S-mode ecalls via a custom SBI Extension ID. Keystone demonstrates that a fully auditable, hardware-enforced TEE can be built from standard RISC-V primitives available on every compliant implementation.
* **Influence on VeridianOS Phase 12:** Direct architectural blueprint for the VeridianOS Security Monitor. The EID convention (`0x08424B45`), 8-slot enclave pool, lock-bit self-protection pattern, and SHA-256 measurement-at-creation design all derive from the Keystone reference implementation. VeridianOS Phase 12 is best understood as a capability-system-integrated Keystone variant, where enclave handles are first-class kernel capability objects rather than opaque SM-internal identifiers.

### RISC-V Privileged Architecture Specification v1.12
* **Venue:** RISC-V Foundation (ratified specification), 2021
* **Authors:** Andrew Waterman, Krste Asanović, et al. (RISC-V International)
* **Relevant Sections:** §3.6 Physical Memory Protection, §3.3 Machine-Level Traps, §3.1 Machine-Level CSRs
* **Core Contribution:** The authoritative specification for RISC-V M-mode behavior, including the complete PMP CSR layout (pmpcfg0–3, pmpaddr0–15), NAPOT address encoding formula, lock bit semantics, and priority rules (lowest-numbered matching entry wins). §3.3 specifies how ecalls from S-mode and U-mode are vectored through `mtvec` and how the `mcause` register encodes the exception type, which is the mechanism the SM uses to intercept SBI calls.
* **Influence on VeridianOS Phase 12:** Every PMP register access in `security_monitor/src/pmp.rs` is written to the exact bit layout and encoding defined in §3.6. The NAPOT formula `pmpaddr = (base >> 2) | ((size/8) - 1)` implemented in `SecurityMonitor::configure_pmp` is taken verbatim from this specification. The lock bit write-once semantics documented in §3.6.1 are what make `lock_monitor_self()` effective — once written, the SM's PMP self-protection entry cannot be cleared by any privilege level until hart reset.

### SBI Specification v2.0
* **Venue:** RISC-V International (ratified specification), 2023
* **Authors:** RISC-V Platform Runtime Services Task Group
* **Relevant Sections:** Chapter 3 (Binary Encoding — EID/FID convention), Chapter 5 (HART State Management Extension), Appendix A (Legacy Extensions)
* **Core Contribution:** The RISC-V Supervisor Binary Interface specification defines the calling convention between S-mode software and M-mode firmware. It establishes the Extension ID (EID) and Function ID (FID) namespace, the register-based argument passing protocol (`a0`–`a5` for arguments, `a0`/`a1` for error/value return), and the HART state machine (STARTED, STOPPED, START_PENDING, STOP_PENDING, SUSPENDED). The HSM extension (EID `0x48534D`) defines `sbi_hart_start` and `sbi_hart_stop` calls that the SM must intercept on enclave entry/exit.
* **Influence on VeridianOS Phase 12:** The Phase 12 SM registers custom SBI extension EID `0x08424B45` following the vendor-defined EID range specified in SBI §3.2. All SM ecall handlers use the SBI return value convention (`SbiRet { error, value }`). The HART state management calls are intercepted by the SM's trap handler to enforce that a hart cannot enter an enclave while in STOPPED state, following the HSM state machine constraints in SBI Chapter 5.

### RFC 9334 — Remote ATtestation procedureS (RATS) Architecture
* **Venue:** Internet Engineering Task Force (IETF), January 2023
* **Authors:** Henk Birkholz, Dave Thaler, Michael Richardson, Ned Smith, Wei Pan
* **Core Contribution:** RFC 9334 defines the architectural vocabulary and data-flow models for remote attestation. It introduces the Attester / Verifier / Relying Party roles, the Evidence / Endorsement / Attestation Result token types, and two reference interaction models: the Passport Model (Verifier issues a signed token that the Attester presents to the Relying Party) and the Background Check Model (Relying Party verifies Evidence directly). The RFC also defines freshness mechanisms (nonce-based and timestamp-based) to prevent replay attacks against attestation tokens.
* **Influence on VeridianOS Phase 12:** The Phase 12 attestation architecture follows the RATS Passport Model: the SM (Attester) generates a signed 73-byte Evidence report, which a remote Verifier checks against a reference measurement before the Relying Party (application server) trusts enclave output. The 24-byte HMAC tag in the Phase 12 report format corresponds to the RATS "Cryptographic Binding" requirement. The planned upgrade to Ed25519 (Phase 12.1) aligns with the RATS guidance on asymmetric attestation keys for open deployments where the Verifier cannot share a symmetric secret with every device.

### Hardware-Assisted Isolation for Trusted Execution: A Comparative Survey
* **Venue:** IEEE Security & Privacy (S&P), 2022
* **Authors:** Moritz Schneider, Aritra Dhar, Ivan Puddu, Kari Kostiainen, Srdjan Capkun (ETH Zürich)
* **Core Contribution:** A systematic comparative analysis of hardware TEE architectures including Intel SGX, Intel TDX, AMD SEV-SNP, ARM TrustZone, and academic RISC-V proposals. The survey defines a threat model taxonomy covering four attacker capabilities: (1) privileged software attacker (OS/hypervisor), (2) physical DRAM attacker, (3) side-channel attacker, and (4) supply-chain attacker. It evaluates each architecture against this taxonomy and identifies which threat classes each design addresses, which it partially mitigates, and which it leaves entirely unaddressed. The survey also provides TCB size comparisons and attestation chain analysis.
* **Influence on VeridianOS Phase 12:** The threat model taxonomy from this paper directly informs Section 4.5 of `RESEARCH_NOTES.md` ("Open Problems Phase 12 Does Not Solve"). The four threat categories documented there — side-channel attacks, physical DRAM attacks, supply-chain trust, and multi-core isolation — map to the four attacker capability classes in this survey. The comparative TEE table in Section 4.4 of the research notes also draws on the architecture summaries in this paper for SGX, TDX, SEV-SNP, and TrustZone.

### Sanctuary: ARMing TrustZone with User-space Enclaves
* **Venue:** Network and Distributed System Security Symposium (NDSS) 2019
* **Authors:** Ferdinand Brasser, David Gens, Patrick Jauernig, Ahmad-Reza Sadeghi, Christian Wachsmann (TU Darmstadt / Fraunhofer SIT)
* **Core Contribution:** Sanctuary proposes a lightweight enclave model layered on top of ARM TrustZone that allows untrusted user-space applications to create isolated enclave instances without requiring OS kernel modifications. It introduces a "Sanctuary Monitor" in EL3 (TrustZone Secure World) that manages enclave creation, memory isolation, and attestation — conceptually equivalent to the RISC-V Security Monitor in Keystone and VeridianOS Phase 12 but targeting the ARM privilege model. Sanctuary demonstrates that a small EL3 monitor (~3,000 LoC) can provide meaningful enclave isolation on commodity ARM hardware already deployed in mobile devices.
* **Influence on VeridianOS Phase 12:** Sanctuary provides an important analogical reference: it demonstrates that the Keystone/VeridianOS "thin monitor in the highest privilege level" pattern is architecturally sound across different ISAs (ARM EL3 and RISC-V M-mode serve the same structural role). The Sanctuary paper's analysis of the trust boundary between the normal-world OS and the enclave monitor also informs the Phase 12 design decision to keep the SM's SBI interface surface minimal — each additional ecall function is an additional attack surface that the untrusted S-mode OS can probe.

---

## Cross-Reference Map

The table below shows which reference directly influenced which VeridianOS source file or design decision, to make it easier to trace an implementation choice back to its academic justification.

| Reference | Primary VeridianOS Artifact | Design Decision Grounded |
|-----------|----------------------------|--------------------------|
| Keystone (USENIX Sec '20) | `security_monitor/src/lib.rs`, `security_monitor/src/pmp.rs` | 8-slot enclave pool, EID `0x08424B45`, `lock_monitor_self()`, SHA-256 measurement |
| RISC-V Privileged Spec §3.6 | `security_monitor/src/pmp.rs` | NAPOT encoding formula, pmpcfg bit layout, lock bit write-once semantics |
| SBI Spec v2.0 Chapter 3 | `security_monitor/src/ecall.rs` | EID/FID register calling convention, `SbiRet` error/value return pair |
| RFC 9334 (RATS) | `security_monitor/src/attestation.rs` | Passport Model flow, Evidence token structure, freshness nonce requirement |
| IEEE S&P Survey '22 | `docs/RESEARCH_NOTES.md §4.5` | Four-class threat model taxonomy used to enumerate Phase 12 open problems |
| Sanctuary (NDSS '19) | Architecture review, SM interface design | Minimal SBI surface principle; cross-ISA validation of M-mode/EL3 monitor pattern |
| seL4 (SOSP '09) | `kernel/src/capability/handle_table.rs` | Unforgeable handle abstraction, rights attenuation on enclave handle delegation |
| Decima (SIGCOMM '19) | `scheduler/src/bandit.rs` | Task graph (DAG) representation for AI workloads, online learning in the kernel |
| Bandit Scheduling (JSA '22) | `scheduler/src/bandit.rs` | Epsilon-greedy EMA update, near-zero kernel overhead justification |
| Asterinas (ATC '25) | `kernel/src/` unsafe boundary | Confine `unsafe` to <5% of codebase; Framekernel TCB isolation pattern |

This map is maintained alongside the source. When a new paper influences a design decision, add a row here and cite the paper in the relevant source file's module-level documentation comment.

---

*Last reviewed: Phase 12 implementation complete. Next review scheduled at Phase 13 feature branch open.*

*Total references: 16 (4 in §1, 2 in §2, 2 in §3, 2 in §4, 2 in §5, 6 in §6).*
