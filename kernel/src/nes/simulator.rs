//! High-Fidelity Software Simulation Engine for NES.

use super::types::{OpType, DeviceType};
use super::queue::{QueueDescriptor, CPU_QUEUE, GPU_QUEUE, NPU_QUEUE};
use super::graph::{NodeState, GRAPH_POOL};

fn read_time() -> u64 {
    let r;
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) r);
    }
    r
}

fn simulate_delay(us: usize) {
    let start_time = read_time();
    // QEMU virt frequency is 10MHz (10 ticks per microsecond)
    let ticks = (us as u64) * 10;
    while read_time() - start_time < ticks {
        crate::process::thread::schedule();
    }
}

fn get_scaling_coefficient(op: OpType, device: DeviceType) -> f32 {
    match device {
        DeviceType::Cpu => {
            match op {
                OpType::GEMM => 15.0,
                OpType::Convolution => 18.0,
                OpType::VectorAdd => 2.0,
                OpType::Activation => 1.5,
                OpType::LayerNorm => 3.0,
                OpType::Softmax => 5.0,
            }
        }
        DeviceType::Gpu => {
            match op {
                OpType::GEMM => 3.0,
                OpType::Convolution => 4.5,
                OpType::VectorAdd => 0.4,
                OpType::Activation => 0.3,
                OpType::LayerNorm => 0.8,
                OpType::Softmax => 1.2,
            }
        }
        DeviceType::Npu | DeviceType::Auto => {
            match op {
                OpType::GEMM => 0.8,
                OpType::Convolution => 1.0,
                OpType::VectorAdd => 8.0,
                OpType::Activation => 6.0,
                OpType::LayerNorm => 4.0,
                OpType::Softmax => 5.0,
            }
        }
    }
}

pub fn execute_node(desc: QueueDescriptor, device: DeviceType) {
    let start_ticks = read_time();

    // 1. Output receiving notifications
    match device {
        DeviceType::Cpu => {
            crate::println!("[NEURAL_SIM]   [CPU Core 0] Processing {:?}...", desc.op_type);
            crate::print!("[NEURAL_SIM]   [CPU Core 0] Inputs: [");
            for i in 0..desc.num_inputs {
                if i > 0 { crate::print!(", "); }
                crate::print!("0x{:X}", desc.inputs_phys[i]);
            }
            crate::println!("], Output: 0x{:X}", desc.outputs_phys[0]);
        }
        DeviceType::Gpu => {
            crate::println!("[NEURAL_SIM]   [GPU Core 0] Doorbell received. Processing {:?}...", desc.op_type);
            crate::print!("[NEURAL_SIM]   [GPU Core 0] Inputs: [");
            for i in 0..desc.num_inputs {
                if i > 0 { crate::print!(", "); }
                crate::print!("0x{:X}", desc.inputs_phys[i]);
            }
            crate::println!("], Output: 0x{:X}", desc.outputs_phys[0]);
        }
        DeviceType::Npu | DeviceType::Auto => {
            crate::println!("[NEURAL_SIM]   [NPU Core 0] Doorbell received. Processing {:?}...", desc.op_type);
            crate::print!("[NEURAL_SIM]   [NPU Core 0] Inputs: [");
            for i in 0..desc.num_inputs {
                if i > 0 { crate::print!(", "); }
                crate::print!("0x{:X}", desc.inputs_phys[i]);
            }
            crate::println!("], Output: 0x{:X}", desc.outputs_phys[0]);
        }
    }

    // 2. Perform latency simulation
    let size_kb = (desc.output_sizes[0] as f32) / 1024.0;
    let latency_us = (size_kb * get_scaling_coefficient(desc.op_type, device)) as usize;
    let latency_type = match desc.op_type {
        OpType::GEMM | OpType::Convolution => "matrix math execution latency",
        _ => "math execution latency",
    };
    crate::println!("[NEURAL_SIM]   [{:?} Core 0] Simulating {}: {} us...", device, latency_type, latency_us);
    simulate_delay(latency_us);

    // 3. Process operations / mutate memory
    match desc.op_type {
        OpType::Activation => {
            if device == DeviceType::Cpu {
                crate::println!("[NEURAL_SIM]   [CPU Core 0] Processing ReLU Activation on buffer 0x{:X}. Output written to 0x{:X}.", desc.inputs_phys[0], desc.outputs_phys[0]);
            }
            let count = desc.output_sizes[0] / 4;
            let in0 = desc.inputs_phys[0] as *const f32;
            let out = desc.outputs_phys[0] as *mut f32;
            unsafe {
                for idx in 0..count {
                    let val = core::ptr::read_volatile(in0.add(idx));
                    let val_relu = if val > 0.0 { val } else { 0.0 };
                    core::ptr::write_volatile(out.add(idx), val_relu);
                }
            }
        }
        OpType::VectorAdd => {
            let count = desc.output_sizes[0] / 4;
            let in0 = desc.inputs_phys[0] as *const f32;
            let in1 = desc.inputs_phys[1] as *const f32;
            let out = desc.outputs_phys[0] as *mut f32;
            unsafe {
                for idx in 0..count {
                    let val0 = core::ptr::read_volatile(in0.add(idx));
                    let val1 = core::ptr::read_volatile(in1.add(idx));
                    core::ptr::write_volatile(out.add(idx), val0 + val1);
                }
            }
        }
        OpType::GEMM => {
            let in0 = desc.inputs_phys[0] as *const f32;
            let in1 = desc.inputs_phys[1] as *const f32;
            let out = desc.outputs_phys[0] as *mut f32;
            unsafe {
                for i in 0..64 {
                    for j in 0..64 {
                        let mut sum = 0.0f32;
                        for k in 0..64 {
                            let val_a = core::ptr::read_volatile(in0.add(i * 64 + k));
                            let val_b = core::ptr::read_volatile(in1.add(k * 64 + j));
                            sum += val_a * val_b;
                        }
                        core::ptr::write_volatile(out.add(i * 64 + j), sum);
                    }
                }
            }
        }
        _ => {}
    }

    let end_ticks = read_time();
    let elapsed = end_ticks.saturating_sub(start_ticks);
    let size_bytes = desc.output_sizes[0];
    
    // Update learned policy stats
    {
        let mut stats = super::POLICY_STATS.lock();
        stats.update(desc.op_type, device, size_bytes, elapsed);
        let op_idx = (desc.op_type as usize).saturating_sub(1);
        let dev_idx = device as usize;
        let new_pred = stats.predicted_ticks_per_byte[op_idx][dev_idx];
        crate::println!(
            "[NEURAL_SCHED] Feedback loop: {:?} on {:?} took {} ticks ({} B). New predicted ticks/byte = {:.4}",
            desc.op_type, device, elapsed, size_bytes, new_pred
        );
    }

    crate::println!("[NEURAL_SIM]   [{:?} Core 0] Computation complete.", device);

    // 4. Complete node
    complete_node(desc.graph_id, desc.node_id);
}

pub fn complete_node(graph_id: usize, node_id: usize) {
    let mut pool = GRAPH_POOL.lock();
    if graph_id > 0 && graph_id <= pool.len() && pool[graph_id - 1].allocated {
        let graph = &mut pool[graph_id - 1];
        graph.nodes[node_id].state = NodeState::Completed;

        let target = graph.nodes[node_id].execution_target;
        if target == DeviceType::Cpu {
            crate::println!("[NEURAL_SCHED] CPU Node {} ({:?}) execution complete.", node_id, graph.nodes[node_id].op_type);
        } else {
            crate::println!("[NEURAL_SCHED] Interrupt received: {:?} Node {} ({:?}) completed.", target, node_id, graph.nodes[node_id].op_type);
        }

        let count = graph.successor_counts[node_id];
        for s in 0..count {
            let succ_id = graph.node_successors[node_id][s];
            if let Some(succ_idx) = (0..graph.num_nodes).find(|&i| graph.nodes[i].node_id == succ_id) {
                let succ_node = &mut graph.nodes[succ_idx];
                succ_node.remaining_dependencies -= 1;
                if succ_node.remaining_dependencies == 0 {
                    succ_node.state = NodeState::Ready;
                    crate::println!("[NEURAL_SCHED] Resolving graph dependencies: Node {} ({:?}) dependencies met. State -> READY.", succ_id, succ_node.op_type);
                    
                    let desc = graph.queue_descriptors[succ_idx];
                    let mut target_queue = succ_node.execution_target;
                    if target_queue == DeviceType::Auto {
                        let size_bytes = desc.output_sizes[0];
                        let best_dev = super::select_optimal_device(desc.op_type, size_bytes);
                        crate::println!("[NEURAL_SCHED] Dynamic Routing Decision: Node {} ({:?}) -> {:?}", succ_id, desc.op_type, best_dev);
                        succ_node.execution_target = best_dev;
                        target_queue = best_dev;
                    }
                    match target_queue {
                        DeviceType::Cpu => {
                            let mut q = CPU_QUEUE.lock();
                            let _ = q.enqueue(desc);
                        }
                        DeviceType::Gpu => {
                            let mut q = GPU_QUEUE.lock();
                            let index = q.head;
                            let _ = q.enqueue(desc);
                            crate::println!("[NEURAL_SCHED] Enqueued Node {} ({:?}) to GPU queue (Index: {}). Doorbell 0x89000000 triggered.", succ_id, desc.op_type, index);
                        }
                        DeviceType::Npu => {
                            let mut q = NPU_QUEUE.lock();
                            let index = q.head;
                            let _ = q.enqueue(desc);
                            crate::println!("[NEURAL_SCHED] Enqueued Node {} ({:?}) to NPU queue (Index: {}). Doorbell 0x89001000 triggered.", succ_id, desc.op_type, index);
                        }
                        _ => {}
                    }
                }
            }
        }

        let all_completed = graph.nodes.iter().take(graph.num_nodes).all(|n| n.state == NodeState::Completed);
        if all_completed {
            graph.active_execution = false;
            crate::println!("[NEURAL_SCHED] Graph {} execution finished. Waking up process PID {} blocked in sys_graph_wait.", graph.graph_id, graph.owner_pid);
            if let Some(tid) = graph.blocked_tid {
                crate::process::thread::wakeup_thread(tid);
                graph.blocked_tid = None;
            }
        }
    }
}

pub fn cpu_worker() -> ! {
    unsafe { crate::process::thread::release_lock(); }
    loop {
        let mut queue_guard = CPU_QUEUE.lock();
        if queue_guard.head != queue_guard.tail {
            let desc = queue_guard.ring[queue_guard.tail];
            queue_guard.tail = (queue_guard.tail + 1) % super::queue::QUEUE_RING_SIZE;
            drop(queue_guard);

            crate::println!("[NEURAL_SCHED] Dispatching Node {} ({:?}) to CPU Worker.", desc.node_id, desc.op_type);
            execute_node(desc, DeviceType::Cpu);
        } else {
            drop(queue_guard);
            crate::process::thread::schedule();
        }
    }
}

pub fn gpu_worker() -> ! {
    unsafe { crate::process::thread::release_lock(); }
    loop {
        let mut queue_guard = GPU_QUEUE.lock();
        if queue_guard.head != queue_guard.tail {
            let desc = queue_guard.ring[queue_guard.tail];
            queue_guard.tail = (queue_guard.tail + 1) % super::queue::QUEUE_RING_SIZE;
            drop(queue_guard);

            execute_node(desc, DeviceType::Gpu);
        } else {
            drop(queue_guard);
            crate::process::thread::schedule();
        }
    }
}

pub fn npu_worker() -> ! {
    unsafe { crate::process::thread::release_lock(); }
    loop {
        let mut queue_guard = NPU_QUEUE.lock();
        if queue_guard.head != queue_guard.tail {
            let desc = queue_guard.ring[queue_guard.tail];
            queue_guard.tail = (queue_guard.tail + 1) % super::queue::QUEUE_RING_SIZE;
            drop(queue_guard);

            execute_node(desc, DeviceType::Npu);
        } else {
            drop(queue_guard);
            crate::process::thread::schedule();
        }
    }
}
