//! TaskNode and TaskGraph representations for NES.

use super::types::{OpType, DeviceType, TensorDescriptor};
use super::queue::QueueDescriptor;
use spin::Mutex;

pub const MAX_NODES_PER_GRAPH: usize = 64;
pub const MAX_DEPENDENCIES: usize = 8;
pub const MAX_INPUTS: usize = 4;
pub const MAX_OUTPUTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Pending,
    Ready,
    Running,
    Completed,
    Failed(isize),
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TaskNode {
    pub node_id: usize,
    pub op_type: OpType,
    pub execution_target: DeviceType,
    pub state: NodeState,
    
    // Memory references mapped from user VMO handles
    pub num_inputs: usize,
    pub inputs: [TensorDescriptor; MAX_INPUTS],
    
    pub num_outputs: usize,
    pub outputs: [TensorDescriptor; MAX_OUTPUTS],
    
    // DAG Dependency trackers (Predecessor IDs)
    pub dependency_count: usize,
    pub dependencies: [usize; MAX_DEPENDENCIES],
    
    // Dynamic execution state tracks remaining unmet dependencies
    pub remaining_dependencies: usize,
}

impl TaskNode {
    pub const fn new_empty() -> Self {
        Self {
            node_id: 0,
            op_type: OpType::GEMM,
            execution_target: DeviceType::Cpu,
            state: NodeState::Pending,
            num_inputs: 0,
            inputs: [TensorDescriptor::new_empty(); MAX_INPUTS],
            num_outputs: 0,
            outputs: [TensorDescriptor::new_empty(); MAX_OUTPUTS],
            dependency_count: 0,
            dependencies: [0; MAX_DEPENDENCIES],
            remaining_dependencies: 0,
        }
    }
}

pub struct TaskGraph {
    pub graph_id: usize,
    pub owner_pid: usize,
    pub validated: bool,
    pub active_execution: bool,
    pub allocated: bool,
    pub blocked_tid: Option<usize>,
    
    pub num_nodes: usize,
    pub nodes: [TaskNode; MAX_NODES_PER_GRAPH],
    
    // Adjacency lists for graph traversal (Successors mapping)
    // node_successors[u] contains the list of nodes that depend on u
    pub node_successors: [[usize; MAX_DEPENDENCIES]; MAX_NODES_PER_GRAPH],
    pub successor_counts: [usize; MAX_NODES_PER_GRAPH],

    // Cache for translated physical descriptors populated during submit
    pub queue_descriptors: [QueueDescriptor; MAX_NODES_PER_GRAPH],
}

impl TaskGraph {
    pub const fn new_empty() -> Self {
        Self {
            graph_id: 0,
            owner_pid: 0,
            validated: false,
            active_execution: false,
            allocated: false,
            blocked_tid: None,
            num_nodes: 0,
            nodes: [TaskNode::new_empty(); MAX_NODES_PER_GRAPH],
            node_successors: [[0; MAX_DEPENDENCIES]; MAX_NODES_PER_GRAPH],
            successor_counts: [0; MAX_NODES_PER_GRAPH],
            queue_descriptors: [QueueDescriptor::new_empty(); MAX_NODES_PER_GRAPH],
        }
    }

    /// Creates a new TaskGraph instance.
    pub const fn new(graph_id: usize, owner_pid: usize) -> Self {
        Self {
            graph_id,
            owner_pid,
            validated: false,
            active_execution: false,
            allocated: true,
            blocked_tid: None,
            num_nodes: 0,
            nodes: [TaskNode::new_empty(); MAX_NODES_PER_GRAPH],
            node_successors: [[0; MAX_DEPENDENCIES]; MAX_NODES_PER_GRAPH],
            successor_counts: [0; MAX_NODES_PER_GRAPH],
            queue_descriptors: [QueueDescriptor::new_empty(); MAX_NODES_PER_GRAPH],
        }
    }

    /// Reset the task graph in-place to avoid stack copies.
    pub fn reset(&mut self, graph_id: usize, owner_pid: usize) {
        self.graph_id = graph_id;
        self.owner_pid = owner_pid;
        self.validated = false;
        self.active_execution = false;
        self.num_nodes = 0;
        self.allocated = true;
        self.blocked_tid = None;
        for val in self.successor_counts.iter_mut() {
            *val = 0;
        }
    }
}

pub static GRAPH_POOL: Mutex<[TaskGraph; 16]> = Mutex::new([const { TaskGraph::new_empty() }; 16]);

pub fn allocate_graph(owner_pid: usize) -> Result<usize, &'static str> {
    let mut pool = GRAPH_POOL.lock();
    for (i, graph) in pool.iter_mut().enumerate() {
        if !graph.allocated {
            let graph_id = i + 1;
            graph.reset(graph_id, owner_pid);
            return Ok(graph_id);
        }
    }
    Err("Graph pool is full")
}

pub fn deallocate_graph(graph_id: usize) -> Result<(), &'static str> {
    let mut pool = GRAPH_POOL.lock();
    if graph_id > 0 && graph_id <= pool.len() {
        pool[graph_id - 1].allocated = false;
        Ok(())
    } else {
        Err("Invalid graph ID")
    }
}

pub fn with_graph<F, R>(graph_id: usize, f: F) -> Result<R, &'static str>
where
    F: FnOnce(&TaskGraph) -> R,
{
    let pool = GRAPH_POOL.lock();
    if graph_id > 0 && graph_id <= pool.len() {
        let graph = &pool[graph_id - 1];
        if graph.allocated {
            Ok(f(graph))
        } else {
            Err("Graph is not allocated")
        }
    } else {
        Err("Invalid graph ID")
    }
}

pub fn with_graph_mut<F, R>(graph_id: usize, f: F) -> Result<R, &'static str>
where
    F: FnOnce(&mut TaskGraph) -> R,
{
    let mut pool = GRAPH_POOL.lock();
    if graph_id > 0 && graph_id <= pool.len() {
        let graph = &mut pool[graph_id - 1];
        if graph.allocated {
            Ok(f(graph))
        } else {
            Err("Graph is not allocated")
        }
    } else {
        Err("Invalid graph ID")
    }
}
