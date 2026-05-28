//! Neural Execution Subsystem (NES) for VeridianOS
//!
//! Provides the data structures, pools, and scheduling abstractions
//! for capability-secured accelerator operations.

pub mod graph;
pub mod types;
pub mod queue;
pub mod validator;
pub mod simulator;
pub mod syscalls;

pub use graph::{
    allocate_graph, deallocate_graph, with_graph, with_graph_mut, GRAPH_POOL, MAX_DEPENDENCIES,
    MAX_INPUTS, MAX_NODES_PER_GRAPH, MAX_OUTPUTS, NodeState, TaskGraph, TaskNode,
};
pub use types::{DataType, DeviceType, OpType, TensorDescriptor, NodeConfig};
pub use queue::{QueueDescriptor, CPU_QUEUE, GPU_QUEUE, NPU_QUEUE, HeterogeneousQueue, QUEUE_RING_SIZE};
pub use validator::validate_dag;
pub use simulator::{cpu_worker, gpu_worker, npu_worker};
pub use syscalls::{sys_graph_create, sys_graph_add_node, sys_graph_submit, sys_graph_wait, sys_policy_configure};

pub const NUM_OP_TYPES: usize = 6;
pub const NUM_DEVICES: usize = 3;

#[derive(Debug)]
pub struct PolicyStats {
    pub cumulative_ticks: [[u64; NUM_DEVICES]; NUM_OP_TYPES],
    pub completion_counts: [[u64; NUM_DEVICES]; NUM_OP_TYPES],
    pub predicted_ticks_per_byte: [[f32; NUM_DEVICES]; NUM_OP_TYPES],
    pub exploration_rate: f32,
}

impl PolicyStats {
    pub const fn new() -> Self {
        Self {
            cumulative_ticks: [[0; NUM_DEVICES]; NUM_OP_TYPES],
            completion_counts: [[0; NUM_DEVICES]; NUM_OP_TYPES],
            predicted_ticks_per_byte: [
                // GEMM (index 0): CPU=15.0, GPU=3.0, NPU=0.8
                [15.0, 3.0, 0.8],
                // Convolution (index 1): CPU=18.0, GPU=4.5, NPU=1.0
                [18.0, 4.5, 1.0],
                // VectorAdd (index 2): CPU=2.0, GPU=0.4, NPU=8.0
                [2.0, 0.4, 8.0],
                // Activation (index 3): CPU=1.5, GPU=0.3, NPU=6.0
                [1.5, 0.3, 6.0],
                // LayerNorm (index 4): CPU=3.0, GPU=0.8, NPU=4.0
                [3.0, 0.8, 4.0],
                // Softmax (index 5): CPU=5.0, GPU=1.2, NPU=5.0
                [5.0, 1.2, 5.0],
            ],
            exploration_rate: 0.1, // 10% exploration by default
        }
    }

    pub fn update(&mut self, op: OpType, device: DeviceType, size_bytes: usize, elapsed_ticks: u64) {
        let op_idx = (op as usize).saturating_sub(1);
        let dev_idx = device as usize;
        if op_idx >= NUM_OP_TYPES || dev_idx >= NUM_DEVICES || size_bytes == 0 {
            return;
        }

        self.cumulative_ticks[op_idx][dev_idx] += elapsed_ticks;
        self.completion_counts[op_idx][dev_idx] += 1;

        let new_sample = (elapsed_ticks as f32) / (size_bytes as f32);
        let old_pred = self.predicted_ticks_per_byte[op_idx][dev_idx];
        // Exponential moving average: 80% history, 20% new observation
        self.predicted_ticks_per_byte[op_idx][dev_idx] = 0.8 * old_pred + 0.2 * new_sample;
    }
}

impl Default for PolicyStats {
    fn default() -> Self {
        Self::new()
    }
}

pub static POLICY_STATS: spin::Mutex<PolicyStats> = spin::Mutex::new(PolicyStats::new());

pub fn select_optimal_device(op: OpType, size_bytes: usize) -> DeviceType {
    let stats_guard = POLICY_STATS.lock();
    let epsilon = stats_guard.exploration_rate;
    
    let r: u64;
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) r);
    }
    let rand_val = ((r % 1000) as f32) / 1000.0;
    
    if rand_val < epsilon {
        let dev_idx = (r % 3) as u32;
        let selected = match dev_idx {
            0 => DeviceType::Cpu,
            1 => DeviceType::Gpu,
            _ => DeviceType::Npu,
        };
        crate::println!("[NEURAL_SCHED] Exploration triggered (epsilon={:.2}). Randomly selected: {:?}", epsilon, selected);
        return selected;
    }

    let op_idx = (op as usize).saturating_sub(1);
    let mut best_device = DeviceType::Cpu;
    let mut lowest_cost = f32::MAX;

    for &dev in &[DeviceType::Cpu, DeviceType::Gpu, DeviceType::Npu] {
        let dev_idx = dev as usize;
        let ticks_per_byte = stats_guard.predicted_ticks_per_byte[op_idx][dev_idx];
        let predicted_exec_ticks = (size_bytes as f32) * ticks_per_byte;
        
        let queue = match dev {
            DeviceType::Cpu => &CPU_QUEUE,
            DeviceType::Gpu => &GPU_QUEUE,
            DeviceType::Npu => &NPU_QUEUE,
            _ => unreachable!(),
        };
        
        let q_lock = queue.lock();
        let mut wait_ticks: f32 = 0.0;
        let mut idx = q_lock.tail;
        while idx != q_lock.head {
            let desc = &q_lock.ring[idx];
            let q_op = desc.op_type;
            let q_size = desc.output_sizes[0];
            let q_op_idx = (q_op as usize).saturating_sub(1);
            let q_ticks_per_byte = stats_guard.predicted_ticks_per_byte[q_op_idx][dev_idx];
            wait_ticks += (q_size as f32) * q_ticks_per_byte;
            idx = (idx + 1) % QUEUE_RING_SIZE;
        }
        drop(q_lock);
        
        let total_cost = predicted_exec_ticks + wait_ticks;
        if total_cost < lowest_cost {
            lowest_cost = total_cost;
            best_device = dev;
        }
    }
    
    best_device
}

pub fn init() {
    crate::println!("[NEURAL_SCHED] Neural Execution Subsystem Initialized.");
    crate::println!("[NEURAL_SCHED] Self-Improving Policy Engine Active. Epsilon-Greedy Scheduler Loaded.");
}

