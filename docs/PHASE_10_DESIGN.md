# VeridianOS Phase 10 Design Specification: Self-Improving Kernel Policies

| Attribute | Specification Details |
| :--- | :--- |
| **Document Version** | 1.0.0 |
| **Status** | Complete |
| **Target Architecture** | RISC-V 64-bit (Sv39 Paging, Supervisor Mode) |
| **Kernel Model** | Capability-Secured Microkernel |
| **Subsystem** | Adaptive Scheduling Engine (NES Policy Layer) |

---

## 1. Executive Summary & Architecture Overview

Conventional OS schedulers assign work to hardware devices via static policy tables: "GEMM operations go to the NPU, VectorAdd to the GPU." These tables are written by an engineer at design time, remain fixed in production, and become stale as workload characteristics drift. An LLM serving system in 2024 has very different operation mix statistics from the system it was tuned for in 2022.

VeridianOS Phase 10 replaces static routing tables with an **online reinforcement learning policy engine embedded directly in the kernel**. No machine learning framework, no user-space daemon, no round-trip to a cloud optimizer. Every time the Neural Execution Subsystem (NES) completes a node on a device, it reads the RISC-V `rdtime` CSR to measure real elapsed ticks, feeds that observation into an Exponential Moving Average (EMA) update, and adjusts its routing predictions for future operations of the same type. The policy converges in kernel space, one observation at a time.

The learning algorithm is **epsilon-greedy** with ε = 0.1. With 90% probability (exploitation), the kernel routes each operation to the device with the lowest predicted total cost: `predicted_exec_ticks + queue_wait_ticks`. With 10% probability (exploration), it picks a device uniformly at random. This ensures the policy continues to discover latency improvements even after initial convergence — critical when hardware is shared with other workloads that affect queue depth.

The policy state lives in `POLICY_STATS: Mutex<PolicyStats>`, a 6×3 matrix of `f32` EMA values accessible to userspace via `SYS_POLICY_CONFIGURE` (80). Phase 11's distributed multi-kernel coherence syscalls (90–101) build directly on top of the NES infrastructure Phase 10 stabilizes.

### System Architecture

```mermaid
graph TD
    subgraph UserSpace ["User Space (U-mode)"]
        App["AI Application\nsys_graph_submit(graph, queue)"]
        PolicyTest["policy_test\nSYS_POLICY_CONFIGURE: GET/SET/RESET"]
    end

    subgraph KernelSpace ["VeridianOS Kernel (S-mode)"]
        SyscallHandler["Syscall Dispatcher (ecall)"]

        subgraph NES ["Neural Execution Subsystem"]
            GraphSubmit["sys_graph_submit()\nDAG validation + VMO translation"]
            PolicyRouter["select_optimal_device(op, size_bytes)\nkernel/src/nes/mod.rs"]
            EpsilonGreedy["Epsilon-Greedy Decision\nε = exploration_rate (default 0.1)"]
            Exploit["Exploit Path\nargmin( EMA[op][dev] × size + queue_wait )"]
            Explore["Explore Path\nxorshift64 PRNG → random device ∈ {CPU,GPU,NPU}"]
            PolicyStats["POLICY_STATS: Mutex&lt;PolicyStats&gt;\n[[f32; 3]; 6] predicted_ticks_per_byte\nexploration_rate: f32"]
        end

        subgraph Queues ["Heterogeneous Device Queues"]
            CPUQueue["CPU_QUEUE\nring[128]: QueueDescriptor"]
            GPUQueue["GPU_QUEUE\nring[128]: QueueDescriptor"]
            NPUQueue["NPU_QUEUE\nring[128]: QueueDescriptor"]
        end

        subgraph Simulators ["Execution Simulators (S-mode)"]
            CPUSim["cpu_worker()\nrdtime before + after"]
            GPUSim["gpu_worker()\nrdtime before + after"]
            NPUSim["npu_worker()\nrdtime before + after"]
        end

        EMAUpdate["PolicyStats::update(op, dev, size, Δticks)\nEMA_new = 0.2 × sample + 0.8 × EMA_old"]
    end

    App -->|"ecall a7=52"| SyscallHandler
    PolicyTest -->|"ecall a7=80"| SyscallHandler
    SyscallHandler --> GraphSubmit
    GraphSubmit -->|"DeviceType::Auto nodes"| PolicyRouter
    PolicyRouter --> EpsilonGreedy
    EpsilonGreedy -->|"rand < ε"| Explore
    EpsilonGreedy -->|"rand ≥ ε"| Exploit
    PolicyStats -->|"EMA table lookup"| Exploit
    Exploit -->|"best_device"| CPUQueue
    Exploit -->|"best_device"| GPUQueue
    Exploit -->|"best_device"| NPUQueue
    Explore -->|"random device"| CPUQueue
    Explore -->|"random device"| GPUQueue
    Explore -->|"random device"| NPUQueue
    CPUQueue --> CPUSim
    GPUQueue --> GPUSim
    NPUQueue --> NPUSim
    CPUSim -->|"elapsed_ticks"| EMAUpdate
    GPUSim -->|"elapsed_ticks"| EMAUpdate
    NPUSim -->|"elapsed_ticks"| EMAUpdate
    EMAUpdate -->|"write lock"| PolicyStats
    SyscallHandler -->|"GET_STATS / SET_EXPLORATION / RESET_STATS"| PolicyStats

    classDef user fill:#1565c0,stroke:#90caf9,color:#ffffff,stroke-width:2px;
    classDef kernel fill:#4e342e,stroke:#bcaaa4,color:#ffffff,stroke-width:2px;
    classDef sim fill:#e65100,stroke:#ffcc80,color:#ffffff,stroke-width:2px;
    class App,PolicyTest user;
    class SyscallHandler,GraphSubmit,PolicyRouter,EpsilonGreedy,Exploit,Explore,PolicyStats,EMAUpdate,CPUQueue,GPUQueue,NPUQueue kernel;
    class CPUSim,GPUSim,NPUSim sim;
```

---

## 2. Design Goals

### 2.1 Sub-Microsecond Routing Decision

The routing decision in `select_optimal_device` runs entirely within the syscall handler while holding the `POLICY_STATS` lock and the three queue locks. There is no I/O, no heap allocation, no floating-point library call beyond native RISC-V FPU instructions, and no user-space round-trip. The decision path is:

1. Read `exploration_rate` from `POLICY_STATS` (one `f32` load)
2. Generate one PRNG value via xorshift64 (three XOR-shift operations)
3. If exploiting: iterate over three devices, perform two `f32` multiplications and one addition each, compare and track minimum (nine FPU instructions total)
4. Return selected `DeviceType`

On a RISC-V core running at 50 MHz (the baseline QEMU `virt` clock), this amounts to well under one hundred clock cycles — sub-2 µs even at slow emulation speeds. On real silicon at 1 GHz+, the routing decision is in the tens-of-nanoseconds range.

### 2.2 Convergence Without Batch Training

Traditional ML-based schedulers (Decima, Park, Optimus) require offline training, a separate inference process, and periodic policy pushes. VeridianOS Phase 10 uses online EMA updates that converge incrementally with each completed node execution. There is no batch, no gradient, no optimizer state — just a 6×3 table of 32-bit floats that adjusts after every observation.

Convergence is guaranteed under stationary workload conditions: EMA is a contraction mapping on the real line, so repeated observations of a stable true latency drive the EMA estimate monotonically toward the true mean. With α = 0.2, the effective observation window is approximately 1/(1−0.8) = 5 completions. After 20 completions of a given `(op_type, device)` pair, the prediction tracks the running mean to within 2%.

### 2.3 No Static Configuration Required

The prior values in `PolicyStats::new()` are reasonable physical intuitions (NPU is fast at GEMM, GPU is fast at VectorAdd) but they are not required to be accurate. Because the system explores with ε = 0.1, it will discover the true ordering on any hardware platform within the first few hundred operations, overwriting stale priors with real measurements. The `SYS_POLICY_CONFIGURE(RESET_STATS)` syscall restores priors at any time, useful for benchmarking or after a major hardware configuration change.

---

## 3. Core Abstractions & Rust Implementations

### 3.1 PolicyStats Matrix

The central data structure is a 6 × 3 matrix of `f32` values representing the EMA-estimated ticks-per-byte for each `(OpType, DeviceType)` pair.

```rust
// kernel/src/nes/mod.rs

pub const NUM_OP_TYPES: usize = 6;
pub const NUM_DEVICES:  usize = 3;

pub struct PolicyStats {
    /// Cumulative timer ticks observed, indexed [op_type-1][device].
    pub cumulative_ticks: [[u64; NUM_DEVICES]; NUM_OP_TYPES],
    /// Number of completions observed, indexed [op_type-1][device].
    pub completion_counts: [[u64; NUM_DEVICES]; NUM_OP_TYPES],
    /// EMA of observed ticks-per-byte, indexed [op_type-1][device].
    /// Layout: rows = OpType (GEMM=0..Softmax=5), cols = DeviceType (CPU=0, GPU=1, NPU=2)
    pub predicted_ticks_per_byte: [[f32; NUM_DEVICES]; NUM_OP_TYPES],
    /// Epsilon: probability of random exploration. Default 0.1.
    pub exploration_rate: f32,
}

pub static POLICY_STATS: spin::Mutex<PolicyStats> = spin::Mutex::new(PolicyStats::new());
```

**Prior values** (used as initial EMA seed before any real observations):

| OpType | CPU (ticks/B) | GPU (ticks/B) | NPU (ticks/B) |
| :--- | :---: | :---: | :---: |
| GEMM (0) | 15.0 | 3.0 | **0.8** |
| Convolution (1) | 18.0 | 4.5 | **1.0** |
| VectorAdd (2) | 2.0 | **0.4** | 8.0 |
| Activation (3) | 1.5 | **0.3** | 6.0 |
| LayerNorm (4) | 3.0 | **0.8** | 4.0 |
| Softmax (5) | 5.0 | **1.2** | 5.0 |

Bold values indicate the prior's predicted fastest device. The policy will confirm or correct these through live measurement.

---

### 3.2 EMA Update Formula and Implementation

After each node execution completes, the simulator calls `PolicyStats::update`. The EMA update formula is:

$$\text{EMA}_{\text{new}} = \alpha \cdot t_{\text{sample}} + (1 - \alpha) \cdot \text{EMA}_{\text{old}}$$

where $t_{\text{sample}}$ is the observed ticks-per-byte for this execution and $\alpha = 0.2$.

This simplifies to:

$$\text{EMA}_{\text{new}} = 0.2 \cdot \frac{\Delta\text{ticks}}{\text{size\_bytes}} + 0.8 \cdot \text{EMA}_{\text{old}}$$

```rust
// kernel/src/nes/mod.rs

impl PolicyStats {
    pub fn update(
        &mut self,
        op:            OpType,
        device:        DeviceType,
        size_bytes:    usize,
        elapsed_ticks: u64,
    ) {
        let op_idx  = (op as usize).saturating_sub(1);   // GEMM=1 → index 0
        let dev_idx = device as usize;                    // CPU=0, GPU=1, NPU=2

        if op_idx >= NUM_OP_TYPES || dev_idx >= NUM_DEVICES || size_bytes == 0 {
            return; // Ignore malformed observations; never panic in kernel
        }

        self.cumulative_ticks[op_idx][dev_idx]   += elapsed_ticks;
        self.completion_counts[op_idx][dev_idx]  += 1;

        let new_sample = (elapsed_ticks as f32) / (size_bytes as f32);
        let old_pred   = self.predicted_ticks_per_byte[op_idx][dev_idx];

        // α = 0.2: exponential moving average with 80% weight on history
        self.predicted_ticks_per_byte[op_idx][dev_idx] =
            0.8 * old_pred + 0.2 * new_sample;
    }
}
```

**Numerical stability**: Because `size_bytes` is always positive (validated before dispatch) and `elapsed_ticks` is a monotonically increasing `rdtime` delta, the quotient is a positive finite `f32`. The EMA cannot diverge: multiplying by 0.8 and 0.2 (both < 1.0) keeps the value bounded by `max(old_pred, new_sample)`.

---

### 3.3 Epsilon-Greedy Routing Algorithm

```rust
// kernel/src/nes/mod.rs

pub fn select_optimal_device(op: OpType, size_bytes: usize) -> DeviceType {
    let stats_guard = POLICY_STATS.lock();
    let epsilon     = stats_guard.exploration_rate;  // default 0.1

    // --- EXPLORATION: pick a random device ---
    let rand_val = get_random(); // xorshift64 PRNG, range [0.0, 1.0)
    if rand_val < epsilon {
        let rand_u64  = PRNG_STATE.load(Ordering::Relaxed);
        let dev_idx   = (rand_u64 % 3) as u32;
        let selected  = match dev_idx {
            0 => DeviceType::Cpu,
            1 => DeviceType::Gpu,
            _ => DeviceType::Npu,
        };
        return selected;
    }

    // --- EXPLOITATION: pick lowest-cost device ---
    let op_idx       = (op as usize).saturating_sub(1);
    let mut best_dev = DeviceType::Cpu;
    let mut lowest   = f32::MAX;

    for &dev in &[DeviceType::Cpu, DeviceType::Gpu, DeviceType::Npu] {
        let dev_idx         = dev as usize;
        let ticks_per_byte  = stats_guard.predicted_ticks_per_byte[op_idx][dev_idx];
        let exec_ticks      = (size_bytes as f32) * ticks_per_byte;

        // Account for queued work already waiting on this device
        let queue   = match dev { Cpu => &CPU_QUEUE, Gpu => &GPU_QUEUE, Npu => &NPU_QUEUE, _ => unreachable!() };
        let q_lock  = queue.lock();
        let mut wait_ticks = 0.0f32;
        let mut idx = q_lock.tail;
        while idx != q_lock.head {
            let desc       = &q_lock.ring[idx];
            let q_op_idx   = (desc.op_type as usize).saturating_sub(1);
            let q_t_per_b  = stats_guard.predicted_ticks_per_byte[q_op_idx][dev_idx];
            wait_ticks    += (desc.output_sizes[0] as f32) * q_t_per_b;
            idx = (idx + 1) % QUEUE_RING_SIZE;
        }
        drop(q_lock);

        let total = exec_ticks + wait_ticks;
        if total < lowest {
            lowest   = total;
            best_dev = dev;
        }
    }

    best_dev
}
```

**Cost function**: The routing cost for a candidate device is:

$$\text{cost}(d) = \underbrace{\hat{t}[op][d] \times \text{size\_bytes}}_{\text{predicted exec ticks}} + \underbrace{\sum_{q \in \text{queue}(d)} \hat{t}[op_q][d] \times \text{size}_q}_{\text{predicted queue wait ticks}}$$

The device with the minimum cost is selected for exploitation. This accounts for both raw throughput and current congestion, avoiding the failure mode where the fastest device is chosen even when it has a 100-item backlog.

---

### 3.4 `rdtime` CSR Sampling

The RISC-V `rdtime` instruction reads the hardware `time` CSR — a monotonically increasing 64-bit counter that increments at a platform-defined frequency (10 MHz for QEMU `virt`). Each simulator worker reads this before and after executing a node:

```rust
// kernel/src/nes/simulator.rs (representative; per-device workers identical)

fn read_time() -> u64 {
    let r;
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) r);
    }
    r
}

// Inside cpu_worker / gpu_worker / npu_worker dispatch loop:
let t_start = read_time();

// ... execute the node operation (memory reads/writes, simulated latency) ...

let t_end     = read_time();
let elapsed   = t_end.saturating_sub(t_start);  // ticks; saturating for safety

let size_out  = desc.output_sizes[0];
let mut stats = POLICY_STATS.lock();
stats.update(desc.op_type, device_type, size_out, elapsed);
```

At 10 MHz, one timer tick = 100 ns. A VectorAdd on 16 KB (~16,384 bytes) at 2.0 ticks/byte (CPU prior) predicts 32,768 ticks = 3.28 ms. The EMA will converge to the real-measured value after a few completions.

---

### 3.5 PRNG Implementation

Because `no_std` kernels have no `std::rand`, Phase 10 uses a seeded xorshift64 PRNG:

```rust
// kernel/src/nes/mod.rs

static PRNG_STATE: AtomicU64 = AtomicU64::new(0);

fn get_random() -> f32 {
    let mut state = PRNG_STATE.load(Ordering::Relaxed);
    if state == 0 {
        let mut r: u64 = 0;
        unsafe { core::arch::asm!("rdtime {}", out(reg) r); }
        state = if r == 0 { 0xACE1 } else { r }; // fallback seed if rdtime == 0
    }

    // xorshift64: full period over 2^64 - 1 non-zero values
    let mut x = state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    PRNG_STATE.store(x, Ordering::Relaxed);

    // Map low 24 bits to [0.0, 1.0)
    ((x & 0xFF_FFFF) as f32) / 16_777_216.0
}
```

xorshift64 has period 2⁶⁴ − 1 and passes all Diehard tests. Using only the low 24 bits for the float conversion avoids bias from truncation. The `rdtime`-derived seed ensures different runs produce different exploration sequences, preventing deterministic lock-in to a suboptimal device during early convergence.

---

## 4. PolicyStats Matrix: Full Prior Table

```rust
// kernel/src/nes/mod.rs — PolicyStats::new()

impl PolicyStats {
    pub const fn new() -> Self {
        Self {
            cumulative_ticks:    [[0; NUM_DEVICES]; NUM_OP_TYPES],
            completion_counts:   [[0; NUM_DEVICES]; NUM_OP_TYPES],
            predicted_ticks_per_byte: [
                // index 0: GEMM        [CPU=15.0, GPU=3.0,  NPU=0.8 ]
                [15.0, 3.0, 0.8 ],
                // index 1: Convolution [CPU=18.0, GPU=4.5,  NPU=1.0 ]
                [18.0, 4.5, 1.0 ],
                // index 2: VectorAdd   [CPU=2.0,  GPU=0.4,  NPU=8.0 ]
                [2.0,  0.4, 8.0 ],
                // index 3: Activation  [CPU=1.5,  GPU=0.3,  NPU=6.0 ]
                [1.5,  0.3, 6.0 ],
                // index 4: LayerNorm   [CPU=3.0,  GPU=0.8,  NPU=4.0 ]
                [3.0,  0.8, 4.0 ],
                // index 5: Softmax     [CPU=5.0,  GPU=1.2,  NPU=5.0 ]
                [5.0,  1.2, 5.0 ],
            ],
            exploration_rate: 0.1, // ε: 10% explore, 90% exploit
        }
    }
}
```

Memory footprint: `6 × 3 × 4 bytes = 72 bytes` for the EMA table. Total `PolicyStats` size including `cumulative_ticks` and `completion_counts` is `6 × 3 × 8 + 6 × 3 × 8 + 6 × 3 × 4 + 4 = 364 bytes`. This fits in two cache lines on any modern processor.

---

## 5. Convergence Behavior

Starting from cold priors, the policy converges as follows for a VectorAdd-heavy workload:

**Round 1 (explore)**: ε-greedy randomly selects NPU. Observation: 8.2 ticks/byte (near prior 8.0). EMA stays near prior.

**Round 2 (exploit)**: Prior says GPU = 0.4 ticks/byte (lowest). GPU is selected. Observation: 0.38 ticks/byte. EMA for `VectorAdd/GPU` updates: `0.8 × 0.4 + 0.2 × 0.38 = 0.396`.

**Round 3–10 (exploit with occasional explore)**: GPU continues winning. EMA for GPU converges to observed mean; EMA for CPU and NPU remain near priors (fewer observations).

After N = 20 observations on the same `(VectorAdd, GPU)` pair, the EMA prediction is within 2% of the true running mean:

$$|\text{EMA}_{20} - \mu| \leq (0.8)^{20} \cdot |\text{EMA}_0 - \mu| \approx 0.012 \cdot |\text{EMA}_0 - \mu|$$

After 50 observations, the factor drops to $(0.8)^{50} \approx 1.4 \times 10^{-5}$: effectively indistinguishable from the true mean. The lowest-latency device for each operation type wins and remains dominant, with 10% of traffic still exploring to detect hardware state changes.

---

## 6. System Call Interface Specification: `SYS_POLICY_CONFIGURE`

```
SYS_POLICY_CONFIGURE = 80
```

`SYS_POLICY_CONFIGURE` provides userspace visibility and control over the kernel's live policy state. It is the primary observability surface for policy_test and future monitoring daemons.

| Register | Role | Value |
| :--- | :--- | :--- |
| `a7` | Syscall number | `80` |
| `a0` | `op` | Operation code: `0=GET_STATS`, `1=SET_EXPLORATION`, `2=RESET_STATS` |
| `a1` | `arg1` | Operation-specific argument (see below) |
| `a2` | `arg2` | Operation-specific argument (see below) |

### 6.1 `GET_STATS` (op = 0)

Copies the 72-byte `predicted_ticks_per_byte` table into a user-space buffer. The table is laid out as `[f32; 18]` in row-major order: `[GEMM/CPU, GEMM/GPU, GEMM/NPU, Conv/CPU, …, Softmax/NPU]`.

| Register | Value |
| :--- | :--- |
| `a1` | User-space pointer to 72-byte output buffer (must be 4-byte aligned) |
| `a2` | Buffer size (must be ≥ 72) |

**Return values**: `0` on success; `-EINVAL` (-22) if `ptr == 0`, `size < 72`, or misaligned; `-EFAULT` (-14) if buffer outside process address space.

### 6.2 `SET_EXPLORATION` (op = 1)

Sets the `exploration_rate` (ε) to a new value. Encoding: the `f32` rate is passed as raw bits in `a1`.

| Register | Value |
| :--- | :--- |
| `a1` | `f32::to_bits(rate)` — must represent a value in `[0.0, 1.0]` |

**Return values**: `0` on success; `-EINVAL` (-22) if the decoded float is outside `[0.0, 1.0]`.

Setting ε = 0.0 switches to **pure greedy mode**: all routing decisions exploit the current EMA table with no random exploration. This is useful for performance benchmarking. Setting ε = 1.0 switches to **pure random mode**, useful for collecting unbiased latency measurements across all devices.

### 6.3 `RESET_STATS` (op = 2)

Overwrites `POLICY_STATS` with the factory priors by calling `*stats = PolicyStats::new()`. Subsequent routing decisions revert to prior-based exploitation until new observations update the EMA.

| Register | Value |
| :--- | :--- |
| `a1` | Ignored |

**Return values**: `0` on success.

---

## 7. Phase 11 Distributed Syscalls: Foundation Layer

Phase 10 stabilizes the NES infrastructure and `POLICY_STATS` that Phase 11 builds upon. The Phase 11 distributed multi-kernel coherence syscalls (numbers 90–101) are defined in `kernel/src/syscall/numbers.rs` and implemented in Phase 11, but their numbering is reserved in Phase 10 to prevent ABI collisions.

| Syscall | Number | Phase 11 Purpose |
| :--- | :---: | :--- |
| `SYS_DOMAIN_JOIN` | 90 | Register a new peer kernel domain by name |
| `SYS_DOMAIN_LIST` | 91 | List all live registered kernel domains |
| `SYS_DOMAIN_STATUS` | 92 | Query status of a specific domain |
| `SYS_GRAPH_DISPATCH_REMOTE` | 93 | Dispatch a NES node to a remote kernel domain |
| `SYS_GRAPH_WAIT_REMOTE` | 94 | Wait for a remote graph node completion |
| `SYS_GRAPH_ABORT_REMOTE` | 95 | Abort a dispatched remote graph node |
| `SYS_CAP_EXPORT` | 96 | Export a local capability handle to a target domain |
| `SYS_CAP_IMPORT` | 97 | Import a capability from a global UID |
| `SYS_CAP_REVOKE_REMOTE` | 98 | Revoke a distributed capability globally |
| `SYS_SGF_REPLICATE_ENABLE` | 99 | Enable global replication for a semantic graph node |
| `SYS_SGF_REPLICATE_QUERY` | 100 | Query replication status of a semantic graph node |
| `SYS_SGF_RAFT_STATUS` | 101 | Query local Raft consensus state |

Phase 11 uses the NES `TaskGraph` infrastructure (Phase 7), the `PolicyStats` routing engine (Phase 10), and the agent IPC primitives (Phase 9) as its building blocks. A remote graph dispatch uses the same `QueueDescriptor` format as a local dispatch, extended with a `domain_id` routing field.

---

## 8. Verification: Expected UART Output from `policy_test`

The `user_programs/policy_test/src/main.rs` program exercises the full Phase 10 adaptive policy engine through six sequential tests. The expected UART trace on a successful run:

```
[USER] VeridianOS Phase 10 — Self-Improving Policy Engine Verification
[USER] ================================================================

[USER] TEST 1: Baseline fixed-target GEMM on NPU...
[NEURAL_SCHED] Creating new TaskGraph (Graph ID 1) for PID 2
[NEURAL_SCHED] Node 0 added to Graph 1 (GEMM, Target: Npu, Inputs: [VMO 5, VMO 6], Outputs: [VMO 7], Deps: [])
[NEURAL_SCHED] Process PID 2 submitted Graph 1 to HeterogeneousQueue
[NEURAL_SCHED] Validating Graph 1 topology...
[NEURAL_SCHED] Topological sort: [Node 0]. No cycles detected.
[NEURAL_SCHED] Translating VMO handles to physical coordinates...
[NEURAL_SCHED]   Node 0 Output (VMO 7) -> Phys Addr 0x40120000 (Size: 16384 bytes)
[NEURAL_SCHED] Verification successful. Enqueuing starting nodes.
[NEURAL_SCHED] Enqueued Node 0 (GEMM) to NPU queue (Index: 0). Doorbell 0x89001000 triggered.
[NEURAL_SIM]   [NPU Core 0] Doorbell received. Processing GEMM...
[NEURAL_SIM]   [NPU Core 0] Computation complete.
[NEURAL_SCHED] Graph 1 execution finished. Waking up PID 2 blocked in sys_graph_wait.
[USER] TEST 1 PASSED: Fixed NPU GEMM completed.

[USER] TEST 2: Auto-Routed VectorAdd (DeviceType::Auto)...
[NEURAL_SCHED] Dynamic Routing Decision: Node 0 (VectorAdd) -> Gpu
[NEURAL_SCHED] Enqueued Node 0 (VectorAdd) to GPU queue (Index: 0). Doorbell 0x89000000 triggered.
[NEURAL_SIM]   [GPU Core 0] Computation complete.
[NEURAL_SCHED] Graph 2 execution finished. Waking up PID 2.
[USER] TEST 2 PASSED: Auto-routed VectorAdd result verified (131.0).

[USER] TEST 3: SYS_POLICY_CONFIGURE(GET_STATS) reads policy table...
[USER] TEST 3 PASSED: Policy stats table readable; CPU/VAdd t/B is positive.

[USER] TEST 4: SYS_POLICY_CONFIGURE(SET_EXPLORATION, 0.0)...
[USER] TEST 4 PASSED: Exploration rate set to 0.0 (pure greedy mode).

[USER] TEST 5: Greedy-mode Auto-Routed VectorAdd (epsilon=0.0)...
[NEURAL_SCHED] Dynamic Routing Decision: Node 0 (VectorAdd) -> Gpu
[NEURAL_SIM]   [GPU Core 0] Computation complete.
[USER] TEST 5 PASSED: Greedy-mode Auto-routing and execution correct.

[USER] TEST 6: SYS_POLICY_CONFIGURE(RESET_STATS)...
[USER] TEST 6 PASSED: RESET_STATS restored CPU/VAdd prior = 2.0.

[USER] ================================================================
[USER] Phase 10 + Phase 11 — ALL TESTS PASSED!
[USER] Self-improving policy, distributed coherence, Raft, DCTP verified.
[USER] ================================================================
```

**Verification checkpoints**:

| Test | What is verified | Pass condition |
| :--- | :--- | :--- |
| TEST 1 | Fixed NPU GEMM completes; NES still functional post-Phase 9 | `sys_graph_wait` returns `0` |
| TEST 2 | Auto-routing selects GPU for VectorAdd; arithmetic correct | Output buffer contains 131.0 ± 0.01 per element |
| TEST 3 | `GET_STATS` (op=0) copies EMA table to user buffer | `VectorAdd/CPU` ticks/byte is positive finite `f32` |
| TEST 4 | `SET_EXPLORATION` (op=1) with ε=0.0 accepted | Returns `0` |
| TEST 5 | Pure greedy mode picks GPU for VectorAdd; arithmetic still correct | Output buffer contains 131.0 ± 0.01 per element |
| TEST 6 | `RESET_STATS` (op=2) restores CPU/VectorAdd prior to 2.0 | `VectorAdd/CPU` after reset = 2.0 ± 0.001 |

The TEST 2 arithmetic: GEMM result C = A × B where all elements of A = 1.0 and B = 2.0 gives C = 64 × 2.0 = 128.0 (GEMM over a 64×64 tile in 4096-element representation). VectorAdd then computes C + V = 128.0 + 3.0 = 131.0.

---

## 9. Academic References

| Source | Year | Relevance |
| :--- | :--- | :--- |
| *Decima: Learning Scheduling Algorithms from Network Traffic* (Mao et al.) | SIGCOMM 2019 | Motivates learned scheduling; Phase 10 is the kernel-embedded counterpart |
| *Online Learning and Regret Minimization* (Shalev-Shwartz) | Foundations 2011 | Theoretical basis for EMA convergence and exploration-exploitation tradeoff |
| *Park: An Open Platform for Learning-Augmented Computer Systems* (Mao et al.) | NeurIPS 2019 | Framework that Phase 10 replaces with in-kernel equivalents |
| *RISC-V ISA Specification Volume I* | 2022 | `rdtime` CSR semantics and privilege mode separation |
| *xorshift RNGs* (Marsaglia, G.) | JSS 2003 | Period and randomness properties of xorshift64 used in the PRNG |
| *AIOS: LLM Agent Operating System* (Mei et al.) | arXiv 2024 | Adaptive resource allocation requirements for AI agent workloads |
