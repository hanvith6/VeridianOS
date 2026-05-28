//! VeridianOS Neural Execution Subsystem Verification Program
//!
//! Constructs a 3-node DAG: NPU GEMM -> CPU ReLU -> GPU VectorAdd.
//! Executes it and verifies the mathematical output correctness.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[inline(always)]
pub fn syscall5(id: usize, arg0: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) -> isize {
    let ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") id,
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            in("a3") arg3,
            in("a4") arg4,
            lateout("a0") ret,
        );
    }
    ret
}

const SYS_WRITE: usize = 1;
const SYS_EXIT: usize = 2;
const SYS_GRAPH_CREATE: usize = 50;
const SYS_GRAPH_ADD_NODE: usize = 51;
const SYS_GRAPH_SUBMIT: usize = 52;
const SYS_GRAPH_WAIT: usize = 53;

fn print(s: &str) {
    syscall5(SYS_WRITE, s.as_ptr() as usize, s.len(), 0, 0, 0);
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    F32 = 1,
    F16 = 2,
    Int8 = 3,
    Int32 = 4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TensorDescriptor {
    pub vmo_handle: usize,
    pub offset: usize,
    pub size: usize,
    pub shape: [usize; 4],
    pub strides: [usize; 4],
    pub data_type: DataType,
}

impl Default for TensorDescriptor {
    fn default() -> Self {
        Self {
            vmo_handle: 0,
            offset: 0,
            size: 0,
            shape: [0; 4],
            strides: [0; 4],
            data_type: DataType::F32,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NodeConfig {
    pub execution_target: u32, // 0 = CPU, 1 = GPU, 2 = NPU
    pub num_inputs: u32,
    pub inputs: [TensorDescriptor; 4],
    pub num_outputs: u32,
    pub outputs: [TensorDescriptor; 2],
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("[USER] Starting Neural Execution Subsystem Verification program...\n");

    // 1. Initialize input matrices A & B (VMO 5 & VMO 6)
    // A: 64x64 f32 populated with 1.0f32
    // B: 64x64 f32 populated with 2.0f32
    let ptr_a = 0x4010_0000 as *mut f32;
    let ptr_b = 0x4011_0000 as *mut f32;
    for i in 0..4096 {
        unsafe {
            *ptr_a.add(i) = 1.0;
            *ptr_b.add(i) = 2.0;
        }
    }

    // 2. Initialize secondary vector input (VMO 9)
    // Vector: 4096 elements populated with 3.0f32
    let ptr_v = 0x4014_0000 as *mut f32;
    for i in 0..4096 {
        unsafe {
            *ptr_v.add(i) = 3.0;
        }
    }

    // Clear output buffers
    let ptr_c = 0x4012_0000 as *mut f32;
    let ptr_act = 0x4013_0000 as *mut f32;
    let ptr_out = 0x4015_0000 as *mut f32;
    for i in 0..4096 {
        unsafe {
            *ptr_c.add(i) = 0.0;
            *ptr_act.add(i) = 0.0;
            *ptr_out.add(i) = 0.0;
        }
    }

    // 3. Create compute task graph
    let graph_handle = syscall5(SYS_GRAPH_CREATE, 0, 0, 0, 0, 0);
    if graph_handle < 0 {
        print("[USER] Error creating task graph!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    let graph_handle = graph_handle as usize;
    print("[USER] Created Task Graph capability successfully.\n");

    // 4. Add Node 0: GEMM (NPU)
    // Inputs: VMO 5, VMO 6 -> Output: VMO 7
    let mut gemm_config = NodeConfig {
        execution_target: 2, // NPU
        num_inputs: 2,
        inputs: [TensorDescriptor::default(); 4],
        num_outputs: 1,
        outputs: [TensorDescriptor::default(); 2],
    };
    gemm_config.inputs[0] = TensorDescriptor {
        vmo_handle: 5,
        offset: 0,
        size: 16384,
        shape: [64, 64, 1, 1],
        strides: [256, 4, 4, 4],
        data_type: DataType::F32,
    };
    gemm_config.inputs[1] = TensorDescriptor {
        vmo_handle: 6,
        offset: 0,
        size: 16384,
        shape: [64, 64, 1, 1],
        strides: [256, 4, 4, 4],
        data_type: DataType::F32,
    };
    gemm_config.outputs[0] = TensorDescriptor {
        vmo_handle: 7,
        offset: 0,
        size: 16384,
        shape: [64, 64, 1, 1],
        strides: [256, 4, 4, 4],
        data_type: DataType::F32,
    };

    let node0_id = syscall5(
        SYS_GRAPH_ADD_NODE,
        graph_handle,
        1, // OpType::GEMM
        &gemm_config as *const NodeConfig as usize,
        0, // Dependency count
        0, // Dependency array pointer
    );
    if node0_id < 0 {
        print("[USER] Error adding GEMM node!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }

    // 5. Add Node 1: Activation ReLU (CPU)
    // Inputs: VMO 7 -> Output: VMO 8
    let mut act_config = NodeConfig {
        execution_target: 0, // CPU
        num_inputs: 1,
        inputs: [TensorDescriptor::default(); 4],
        num_outputs: 1,
        outputs: [TensorDescriptor::default(); 2],
    };
    act_config.inputs[0] = TensorDescriptor {
        vmo_handle: 7,
        offset: 0,
        size: 16384,
        shape: [4096, 1, 1, 1],
        strides: [4, 4, 4, 4],
        data_type: DataType::F32,
    };
    act_config.outputs[0] = TensorDescriptor {
        vmo_handle: 8,
        offset: 0,
        size: 16384,
        shape: [4096, 1, 1, 1],
        strides: [4, 4, 4, 4],
        data_type: DataType::F32,
    };

    let deps_node1 = [0usize];
    let node1_id = syscall5(
        SYS_GRAPH_ADD_NODE,
        graph_handle,
        4, // OpType::Activation
        &act_config as *const NodeConfig as usize,
        1, // Dependency count
        deps_node1.as_ptr() as usize,
    );
    if node1_id < 0 {
        print("[USER] Error adding Activation node!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }

    // 6. Add Node 2: VectorAdd (GPU)
    // Inputs: VMO 8, VMO 9 -> Output: VMO 10
    let mut add_config = NodeConfig {
        execution_target: 1, // GPU
        num_inputs: 2,
        inputs: [TensorDescriptor::default(); 4],
        num_outputs: 1,
        outputs: [TensorDescriptor::default(); 2],
    };
    add_config.inputs[0] = TensorDescriptor {
        vmo_handle: 8,
        offset: 0,
        size: 16384,
        shape: [4096, 1, 1, 1],
        strides: [4, 4, 4, 4],
        data_type: DataType::F32,
    };
    add_config.inputs[1] = TensorDescriptor {
        vmo_handle: 9,
        offset: 0,
        size: 16384,
        shape: [4096, 1, 1, 1],
        strides: [4, 4, 4, 4],
        data_type: DataType::F32,
    };
    add_config.outputs[0] = TensorDescriptor {
        vmo_handle: 10,
        offset: 0,
        size: 16384,
        shape: [4096, 1, 1, 1],
        strides: [4, 4, 4, 4],
        data_type: DataType::F32,
    };

    let deps_node2 = [1usize];
    let node2_id = syscall5(
        SYS_GRAPH_ADD_NODE,
        graph_handle,
        3, // OpType::VectorAdd
        &add_config as *const NodeConfig as usize,
        1, // Dependency count
        deps_node2.as_ptr() as usize,
    );
    if node2_id < 0 {
        print("[USER] Error adding VectorAdd node!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }

    // 7. Submit the computation graph
    // Queue capability is pre-inserted at handle 4
    let submit_ret = syscall5(SYS_GRAPH_SUBMIT, graph_handle, 4, 0, 0, 0);
    if submit_ret < 0 {
        print("[USER] Graph submission failed!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    print("[USER] Task Graph submitted to DeviceQueue.\n");

    // 8. Wait for completion
    let wait_ret = syscall5(SYS_GRAPH_WAIT, graph_handle, usize::MAX, 0, 0, 0);
    if wait_ret < 0 {
        print("[USER] Graph execution wait failed!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    print("[USER] Execution completed successfully. Verifying mathematical outputs...\n");

    // 9. Verify result in VMO 10:
    // C_ij = sum_k (A_ik * B_kj) = sum_k (1.0 * 2.0) = 64 * 2.0 = 128.0
    // Activation: ReLU(128.0) = 128.0
    // VectorAdd: 128.0 + 3.0 = 131.0
    let mut verified = true;
    for i in 0..4096 {
        let val = unsafe { *ptr_out.add(i) };
        if (val - 131.0).abs() > 0.0001 {
            verified = false;
            break;
        }
    }

    if verified {
        print("[USER] Result Verification SUCCESS: All elements are exactly 131.0!\n");
        syscall5(SYS_EXIT, 0, 0, 0, 0, 0);
    } else {
        print("[USER] Result Verification FAILURE: Output elements mismatch!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    loop {}
}

// Absolute value function since core::f32::abs requires std
#[allow(dead_code)]
trait AbsExt {
    fn abs(self) -> Self;
}
impl AbsExt for f32 {
    fn abs(self) -> Self {
        if self < 0.0 { -self } else { self }
    }
}

