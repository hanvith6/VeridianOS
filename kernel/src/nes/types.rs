//! Core types and descriptors for the Neural Execution Subsystem (NES).

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    F32 = 0,
    F16 = 1,
    BF16 = 2,
    Int8 = 3,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpType {
    GEMM = 1,
    Convolution = 2,
    VectorAdd = 3,
    Activation = 4, // ReLU, GeLU, etc.
    LayerNorm = 5,
    Softmax = 6,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Cpu = 0,
    Gpu = 1,
    Npu = 2,
    Auto = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TensorDescriptor {
    pub vmo_handle: usize,     // Local process handle ID pointing to a VMO
    pub offset: usize,         // Byte offset inside the physical memory object
    pub size: usize,           // Total buffer size in bytes
    pub shape: [usize; 4],     // Up to 4D tensor dimension tracking
    pub strides: [usize; 4],   // Dimension stride values for memory layout
    pub data_type: DataType,   // Element data type representation
}

impl TensorDescriptor {
    /// Creates a blank TensorDescriptor.
    pub const fn new_empty() -> Self {
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
    pub execution_target: u32,
    pub num_inputs: u32,
    pub inputs: [TensorDescriptor; 4],
    pub num_outputs: u32,
    pub outputs: [TensorDescriptor; 2],
}


