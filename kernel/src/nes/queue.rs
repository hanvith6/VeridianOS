//! Heterogeneous Execution Queues for NES.

use super::types::{OpType, DeviceType};
use super::graph::{MAX_INPUTS, MAX_OUTPUTS};

use spin::Mutex;

pub const QUEUE_RING_SIZE: usize = 128;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct QueueDescriptor {
    pub graph_id: usize,
    pub node_id: usize,
    pub op_type: OpType,
    
    // Translated physical addresses (Kernel verifies VMO and populates these fields)
    pub num_inputs: usize,
    pub inputs_phys: [usize; MAX_INPUTS],
    pub input_sizes: [usize; MAX_INPUTS],
    
    pub num_outputs: usize,
    pub outputs_phys: [usize; MAX_OUTPUTS],
    pub output_sizes: [usize; MAX_OUTPUTS],
}

impl QueueDescriptor {
    pub const fn new_empty() -> Self {
        Self {
            graph_id: 0,
            node_id: 0,
            op_type: OpType::GEMM,
            num_inputs: 0,
            inputs_phys: [0; MAX_INPUTS],
            input_sizes: [0; MAX_INPUTS],
            num_outputs: 0,
            outputs_phys: [0; MAX_OUTPUTS],
            output_sizes: [0; MAX_OUTPUTS],
        }
    }
}


pub struct HeterogeneousQueue {
    pub device_type: DeviceType,
    pub ring: [QueueDescriptor; QUEUE_RING_SIZE],
    pub head: usize,        // Write index: updated by the kernel scheduler
    pub tail: usize,        // Read index: updated by the execution hardware/thread
    pub doorbell_reg: usize, // Physical or simulated MMIO doorbell register address
}

impl HeterogeneousQueue {
    pub const fn new(device_type: DeviceType, doorbell_reg: usize) -> Self {
        Self {
            device_type,
            ring: [QueueDescriptor::new_empty(); QUEUE_RING_SIZE],
            head: 0,
            tail: 0,
            doorbell_reg,
        }
    }

    /// Inserts a task descriptor into the ring buffer.
    pub fn enqueue(&mut self, desc: QueueDescriptor) -> Result<(), &'static str> {
        let next_head = (self.head + 1) % QUEUE_RING_SIZE;
        if next_head == self.tail {
            return Err("Queue ring buffer is full");
        }
        self.ring[self.head] = desc;
        self.head = next_head;
        self.trigger_doorbell();
        Ok(())
    }

    /// Triggers the doorbell to notify the execution core.
    fn trigger_doorbell(&self) {
        unsafe {
            let ptr = self.doorbell_reg as *mut u32;
            core::ptr::write_volatile(ptr, 1);
        }
    }
}

pub static CPU_QUEUE: Mutex<HeterogeneousQueue> = Mutex::new(HeterogeneousQueue::new(DeviceType::Cpu, 0x8900_2000));
pub static GPU_QUEUE: Mutex<HeterogeneousQueue> = Mutex::new(HeterogeneousQueue::new(DeviceType::Gpu, 0x8900_0000));
pub static NPU_QUEUE: Mutex<HeterogeneousQueue> = Mutex::new(HeterogeneousQueue::new(DeviceType::Npu, 0x8900_1000));
