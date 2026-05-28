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
