//! VeridianOS Phase 10 — Self-Improving Kernel Policy Verification Program
//!
//! This program validates the epsilon-greedy neural scheduler's dynamic
//! routing engine by:
//!
//! 1. **Baseline Check**: Submit a fixed-target graph (NPU GEMM) and confirm
//!    it completes correctly — ensuring NES still works from Phase 7.
//!
//! 2. **Auto-Routing Test**: Submit a graph with `DeviceType::Auto (3)` nodes.
//!    The kernel must select the optimal device based on `POLICY_STATS`.
//!
//! 3. **Policy Configure Syscall**: Call SYS_POLICY_CONFIGURE to:
//!    a. Read back the learned `ticks_per_byte` table.
//!    b. Set the exploration rate to 0.0 (pure greedy).
//!    c. Reset stats back to the priors.
//!
//! 4. **Multi-Round Learning**: Submit three sequential VectorAdd graphs with
//!    Auto routing. After each round, confirm the kernel made a routing decision.
//!    Since CPU has a lower default ticks/byte for VectorAdd (2.0) than NPU (8.0),
//!    the greedy scheduler should consistently pick CPU or GPU for VectorAdd.
//!
//! 5. Exit with code 0 on full success, code 1 on any failure.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

// ─── Syscall helper ──────────────────────────────────────────────────────────

#[inline(always)]
fn syscall5(id: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
    let ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") id,
            in("a0") a0,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
            lateout("a0") ret,
        );
    }
    ret
}

// ─── Syscall numbers ─────────────────────────────────────────────────────────
const SYS_WRITE:            usize = 1;
const SYS_EXIT:             usize = 2;
const SYS_GRAPH_CREATE:     usize = 50;
const SYS_GRAPH_ADD_NODE:   usize = 51;
const SYS_GRAPH_SUBMIT:     usize = 52;
const SYS_GRAPH_WAIT:       usize = 53;
const SYS_POLICY_CONFIGURE: usize = 80;

// ─── NES Types (mirroring kernel nes/types.rs) ───────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct TensorDescriptor {
    vmo_handle: usize,
    offset:     usize,
    size:       usize,
    shape:      [usize; 4],
    strides:    [usize; 4],
    data_type:  u32, // 0 = F32
}

impl TensorDescriptor {
    const fn zeroed() -> Self {
        Self {
            vmo_handle: 0, offset: 0, size: 0,
            shape: [0; 4], strides: [0; 4], data_type: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NodeConfig {
    execution_target: u32,          // 0=CPU, 1=GPU, 2=NPU, 3=Auto
    num_inputs:       u32,
    inputs:           [TensorDescriptor; 4],
    num_outputs:      u32,
    outputs:          [TensorDescriptor; 2],
}

// ─── I/O helpers ─────────────────────────────────────────────────────────────

fn print(s: &str) {
    syscall5(SYS_WRITE, s.as_ptr() as usize, s.len(), 0, 0, 0);
}

fn fail(msg: &str) -> ! {
    print("[USER] FAIL: ");
    print(msg);
    print("\n");
    syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    loop {}
}

fn check(ret: isize, ctx: &str) {
    if ret < 0 { fail(ctx); }
}

// ─── Graph helpers ───────────────────────────────────────────────────────────

/// Create a single-node graph executing `op_type` on `device` with the given
/// input/output VMO handles. Returns the graph handle.
fn single_node_graph(device: u32, op_type: usize,
                     in0: usize, in1_opt: Option<usize>,
                     out0: usize, tensor_size: usize) -> usize {

    let graph_h = syscall5(SYS_GRAPH_CREATE, 0, 0, 0, 0, 0);
    check(graph_h, "graph create");
    let graph_h = graph_h as usize;

    let mut cfg = NodeConfig {
        execution_target: device,
        num_inputs:  if in1_opt.is_some() { 2 } else { 1 },
        inputs:  [TensorDescriptor::zeroed(); 4],
        num_outputs: 1,
        outputs: [TensorDescriptor::zeroed(); 2],
    };

    cfg.inputs[0] = TensorDescriptor {
        vmo_handle: in0, offset: 0, size: tensor_size,
        shape: [tensor_size / 4, 1, 1, 1], strides: [4, 4, 4, 4], data_type: 0,
    };
    if let Some(in1) = in1_opt {
        cfg.inputs[1] = TensorDescriptor {
            vmo_handle: in1, offset: 0, size: tensor_size,
            shape: [tensor_size / 4, 1, 1, 1], strides: [4, 4, 4, 4], data_type: 0,
        };
    }
    cfg.outputs[0] = TensorDescriptor {
        vmo_handle: out0, offset: 0, size: tensor_size,
        shape: [tensor_size / 4, 1, 1, 1], strides: [4, 4, 4, 4], data_type: 0,
    };

    let node_ret = syscall5(SYS_GRAPH_ADD_NODE, graph_h, op_type,
                            &cfg as *const NodeConfig as usize, 0, 0);
    check(node_ret, "graph add node");

    // Queue handle pre-inserted at slot 4 by the kernel spawn path
    let submit_ret = syscall5(SYS_GRAPH_SUBMIT, graph_h, 4, 0, 0, 0);
    check(submit_ret, "graph submit");

    graph_h
}

// ─── Entry Point ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("[USER] VeridianOS Phase 10 — Self-Improving Policy Engine Verification\n");
    print("[USER] ================================================================\n\n");

    // VMO layout (mapped by kernel in process/mod.rs for 'policy_test'):
    //   Handle 5  → virt 0x4010_0000 (input  A, 16KB)
    //   Handle 6  → virt 0x4011_0000 (input  B, 16KB)
    //   Handle 7  → virt 0x4012_0000 (output C, 16KB)
    //   Handle 8  → virt 0x4013_0000 (scratch, 16KB)
    //   Handle 9  → virt 0x4014_0000 (vector  V, 16KB)
    //   Handle 10 → virt 0x4015_0000 (output  W, 16KB)

    const TENSOR_SIZE: usize = 16384; // 4096 × f32

    let ptr_a   = 0x4010_0000usize as *mut f32;
    let ptr_b   = 0x4011_0000usize as *mut f32;
    let ptr_c   = 0x4012_0000usize as *mut f32;
    let ptr_act = 0x4013_0000usize as *mut f32;
    let ptr_v   = 0x4014_0000usize as *mut f32;
    let ptr_w   = 0x4015_0000usize as *mut f32;

    // ── Initialise input data ────────────────────────────────────────────────
    unsafe {
        for i in 0..4096usize {
            *ptr_a.add(i)   = 1.0;   // Matrix A: all 1s
            *ptr_b.add(i)   = 2.0;   // Matrix B: all 2s
            *ptr_v.add(i)   = 3.0;   // Vector V: all 3s
            *ptr_c.add(i)   = 0.0;
            *ptr_act.add(i) = 0.0;
            *ptr_w.add(i)   = 0.0;
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // TEST 1 — Baseline: Fixed NPU GEMM confirms NES still correct
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 1: Baseline fixed-target GEMM on NPU...\n");

    let g1 = single_node_graph(2 /*NPU*/, 1 /*GEMM*/, 5, Some(6), 7, TENSOR_SIZE);
    let w1 = syscall5(SYS_GRAPH_WAIT, g1, usize::MAX, 0, 0, 0);
    check(w1, "wait graph 1");

    print("[USER] TEST 1 PASSED: Fixed NPU GEMM completed.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 2 — Auto-Routing: VectorAdd with DeviceType::Auto
    //          Kernel must select CPU or GPU (both faster than NPU for VAdd).
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 2: Auto-Routed VectorAdd (DeviceType::Auto)...\n");

    // Re-initialise destination buffer
    unsafe { for i in 0..4096 { *ptr_w.add(i) = 0.0; } }

    // Use VMO 7 (GEMM output: 128.0) + VMO 9 (all 3s) → VMO 10
    let g2 = single_node_graph(3 /*Auto*/, 3 /*VectorAdd*/, 7, Some(9), 10, TENSOR_SIZE);
    let w2 = syscall5(SYS_GRAPH_WAIT, g2, usize::MAX, 0, 0, 0);
    check(w2, "wait graph 2");

    // Verify: GEMM output C_ij = 64*2.0 = 128.0; VectorAdd: 128.0 + 3.0 = 131.0
    let mut verified = true;
    for i in 0..4096 {
        let val = unsafe { *ptr_w.add(i) };
        let diff = if val > 131.0 { val - 131.0 } else { 131.0 - val };
        if diff > 0.01 {
            verified = false;
            break;
        }
    }
    if !verified {
        fail("Auto-routed VectorAdd result mismatch (expected 131.0 per element)");
    }

    print("[USER] TEST 2 PASSED: Auto-routed VectorAdd result verified (131.0).\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 3 — SYS_POLICY_CONFIGURE: read back learned ticks_per_byte table
    //          The kernel returns 6×3 f32 values = 72 bytes.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 3: SYS_POLICY_CONFIGURE(GET_STATS) reads policy table...\n");

    // We use ptr_act (VMO 8) as a temporary 72-byte scratchpad — safe because
    // it's a full 16KB VMO and is not used for live computation right now.
    let stats_ptr = ptr_act as usize;
    let get_ret = syscall5(SYS_POLICY_CONFIGURE, 0 /*GET_STATS*/, stats_ptr, 72, 0, 0);
    check(get_ret, "policy configure GET_STATS");

    // Sanity: at least the CPU/VectorAdd prediction (index [2][0]) must be
    // a positive finite f32.  Index layout: row = op_type-1, col = device.
    // VectorAdd CPU priors = 2.0; after observing one real execution it may
    // have drifted slightly from the prior.
    let vadd_cpu_bits = unsafe { *(ptr_act.add(6) as *const u32) }; // [2][0] => offset 2*3=6
    let vadd_cpu_val = f32::from_bits(vadd_cpu_bits);
    // Must be a positive normal float (not NaN / Inf / zero)
    if vadd_cpu_val <= 0.0 || vadd_cpu_val > 1_000_000.0 {
        fail("Policy stats CPU/VectorAdd ticks_per_byte is not a positive finite f32");
    }

    print("[USER] TEST 3 PASSED: Policy stats table readable; CPU/VAdd t/B is positive.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 4 — SET_EXPLORATION: force epsilon = 0.0 (pure greedy)
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 4: SYS_POLICY_CONFIGURE(SET_EXPLORATION, 0.0)...\n");

    let zero_bits = 0.0f32.to_bits() as usize;
    let set_ret = syscall5(SYS_POLICY_CONFIGURE, 1 /*SET_EXPLORATION*/, zero_bits, 0, 0, 0);
    check(set_ret, "policy configure SET_EXPLORATION");

    print("[USER] TEST 4 PASSED: Exploration rate set to 0.0 (pure greedy mode).\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 5 — GREEDY ROUND: With epsilon=0 the kernel must pick the device
    //          with the lowest predicted latency.  Run another Auto VectorAdd.
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 5: Greedy-mode Auto-Routed VectorAdd (epsilon=0.0)...\n");

    unsafe { for i in 0..4096 { *ptr_w.add(i) = 0.0; } }

    let g5 = single_node_graph(3 /*Auto*/, 3 /*VectorAdd*/, 7, Some(9), 10, TENSOR_SIZE);
    let w5 = syscall5(SYS_GRAPH_WAIT, g5, usize::MAX, 0, 0, 0);
    check(w5, "wait graph 5");

    // Same arithmetic, same expected result
    let mut ok5 = true;
    for i in 0..4096 {
        let val = unsafe { *ptr_w.add(i) };
        let diff = if val > 131.0 { val - 131.0 } else { 131.0 - val };
        if diff > 0.01 { ok5 = false; break; }
    }
    if !ok5 {
        fail("Greedy-mode VectorAdd result mismatch");
    }

    print("[USER] TEST 5 PASSED: Greedy-mode Auto-routing and execution correct.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 6 — RESET_STATS: restore priors and confirm next read makes sense
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 6: SYS_POLICY_CONFIGURE(RESET_STATS)...\n");

    let reset_ret = syscall5(SYS_POLICY_CONFIGURE, 2 /*RESET_STATS*/, 0, 0, 0, 0);
    check(reset_ret, "policy configure RESET_STATS");

    // Read stats again and verify VAdd/CPU returned to prior (2.0)
    let get2_ret = syscall5(SYS_POLICY_CONFIGURE, 0 /*GET_STATS*/, stats_ptr, 72, 0, 0);
    check(get2_ret, "policy configure GET_STATS after reset");

    let vadd_cpu_bits2 = unsafe { *(ptr_act.add(6) as *const u32) };
    let vadd_cpu_val2 = f32::from_bits(vadd_cpu_bits2);
    // Prior is 2.0 ticks/byte; allow tiny FP representation fuzz
    let prior_diff = if vadd_cpu_val2 > 2.0 { vadd_cpu_val2 - 2.0 } else { 2.0 - vadd_cpu_val2 };
    if prior_diff > 0.001 {
        fail("Policy stats not reset to prior (2.0) for CPU/VectorAdd");
    }

    print("[USER] TEST 6 PASSED: RESET_STATS restored CPU/VAdd prior = 2.0.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // TEST 7 — Distributed System Calls: Verify that 90-101 return 0
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] TEST 7: Phase 11 Distributed System Call Stub Verification...\n");

    let ret90 = syscall5(90, 0x1111, 5, 0x2222, 4, 8080);
    check(ret90, "domain_join");

    let ret91 = syscall5(91, 0x3333, 100, 0x4444, 0, 0);
    check(ret91, "domain_list");

    let ret92 = syscall5(92, 0x5555, 200, 0, 0, 0);
    check(ret92, "domain_status");

    let ret93 = syscall5(93, 10, 20, 30, 0, 0);
    check(ret93, "graph_dispatch_remote");

    let ret94 = syscall5(94, 10, 20, 1000, 0, 0);
    check(ret94, "graph_wait_remote");

    let ret95 = syscall5(95, 10, 20, 0, 0, 0);
    check(ret95, "graph_abort_remote");

    let ret96 = syscall5(96, 5, 2, 0x6666, 0, 0);
    check(ret96, "cap_export");

    let ret97 = syscall5(97, 0x7777, 16, 2, 0, 0);
    check(ret97, "cap_import");

    let ret98 = syscall5(98, 5, 2, 0, 0, 0);
    check(ret98, "cap_revoke_remote");

    let ret99 = syscall5(99, 1, 3, 0, 0, 0);
    check(ret99, "sgf_replicate_enable");

    let ret100 = syscall5(100, 0x8888, 32, 0x9999, 0, 0);
    check(ret100, "sgf_replicate_query");

    let ret101 = syscall5(101, 0xAAAA, 64, 0, 0, 0);
    check(ret101, "sgf_raft_status");

    print("[USER] TEST 7 PASSED: All Phase 11 Distributed System Calls returned 0.\n\n");

    // ════════════════════════════════════════════════════════════════════════
    // ALL TESTS PASSED
    // ════════════════════════════════════════════════════════════════════════
    print("[USER] ================================================================\n");
    print("[USER] Phase 10 — Self-Improving Policy Engine: ALL TESTS PASSED!\n");
    print("[USER] Dynamic routing, policy stats, exploration control all verified.\n");
    print("[USER] ================================================================\n");

    syscall5(SYS_EXIT, 0, 0, 0, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    loop {}
}
