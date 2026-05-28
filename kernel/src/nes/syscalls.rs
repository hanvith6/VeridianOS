//! System call handlers for NES.

use crate::capability::{Handle, ObjectType, Rights};
use super::graph::{allocate_graph, deallocate_graph, GRAPH_POOL, MAX_NODES_PER_GRAPH, MAX_DEPENDENCIES, MAX_INPUTS, MAX_OUTPUTS, TaskNode, TaskGraph, NodeState};
use super::types::{OpType, DeviceType, NodeConfig};
use super::queue::{QueueDescriptor, CPU_QUEUE, GPU_QUEUE, NPU_QUEUE, QUEUE_RING_SIZE};
use crate::memory::PageTable;

// Helper to read RISC-V time
fn read_time() -> u64 {
    let r;
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) r);
    }
    r
}

// Helper to translate virtual user address to physical address
fn translate_user_address(pt: &mut PageTable, virt_addr: usize) -> Option<usize> {
    let page_offset = virt_addr % crate::memory::PAGE_SIZE;
    let page_base = virt_addr - page_offset;
    pt.get_entry_mut(page_base).map(|entry| entry.physical_address() + page_offset)
}

pub fn sys_graph_create() -> isize {
    crate::process::with_current_process(|proc| {
        let graph_id = match allocate_graph(proc.pid) {
            Ok(id) => id,
            Err(_) => return -12, // -ENOMEM
        };
        
        crate::println!("[NEURAL_SCHED] Creating new TaskGraph (Graph ID {}) for PID {}", graph_id, proc.pid);
        
        let handle = Handle::new(
            ObjectType::TaskGraph,
            graph_id,
            Rights::DEFAULT | Rights::READ | Rights::WRITE | Rights::EXECUTE,
        );
        
        match proc.handle_table.insert(handle) {
            Ok(handle_id) => handle_id as isize,
            Err(_) => {
                let _ = deallocate_graph(graph_id);
                -12 // -ENOMEM
            }
        }
    }).unwrap_or(-3) // -EPERM
}

pub fn sys_graph_add_node(
    graph_handle: usize,
    op_type: usize,
    config_ptr: usize,
    dependency_count: usize,
    dependency_array_ptr: usize,
) -> isize {
    // Validate config_ptr and dependency_array_ptr first
    let valid = crate::process::with_current_process(|proc| {
        if config_ptr == 0 || !proc.validate_user_buffer(config_ptr, core::mem::size_of::<NodeConfig>(), false) {
            return false;
        }
        if dependency_count > 0 && (dependency_array_ptr == 0 || !proc.validate_user_buffer(dependency_array_ptr, dependency_count * core::mem::size_of::<u32>(), false)) {
            return false;
        }
        true
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

    crate::process::with_current_process(|proc| {
        // 1. Retrieve and validate graph handle
        let handle = match proc.handle_table.get(graph_handle) {
            Ok(h) => h,
            Err(_) => return -9, // -EBADF
        };
        if handle.object_type != ObjectType::TaskGraph {
            return -9; // -EBADF
        }
        if !handle.rights.contains(Rights::WRITE) {
            return -13; // -EACCES
        }
        let graph_id = handle.object_ptr;

        // 2. Parse and validate OpType
        let op_type_enum = match op_type {
            1 => OpType::GEMM,
            2 => OpType::Convolution,
            3 => OpType::VectorAdd,
            4 => OpType::Activation,
            5 => OpType::LayerNorm,
            6 => OpType::Softmax,
            _ => return -22, // -EINVAL
        };

        // 3. Validate alignment
        if !config_ptr.is_multiple_of(core::mem::align_of::<NodeConfig>()) {
            return -14; // -EFAULT
        }
        let config = unsafe { &*(config_ptr as *const NodeConfig) };

        if dependency_count > MAX_DEPENDENCIES {
            return -22; // -EINVAL
        }
        if dependency_count > 0 && !dependency_array_ptr.is_multiple_of(core::mem::align_of::<u32>()) {
            return -14; // -EFAULT
        }

        // 4. Validate NodeConfig parameters
        let exec_target = match config.execution_target {
            0 => DeviceType::Cpu,
            1 => DeviceType::Gpu,
            2 => DeviceType::Npu,
            3 => DeviceType::Auto,
            _ => return -22, // -EINVAL
        };
        if config.num_inputs as usize > MAX_INPUTS || config.num_outputs as usize > MAX_OUTPUTS {
            return -22; // -EINVAL
        }

        // 5. Validate VMO handle rights
        for i in 0..config.num_inputs as usize {
            let vmo_h = config.inputs[i].vmo_handle;
            let vmo_handle = match proc.handle_table.get(vmo_h) {
                Ok(h) => h,
                Err(_) => return -9, // -EBADF
            };
            if vmo_handle.object_type != ObjectType::VirtualMemoryObject {
                return -9; // -EBADF
            }
            if !vmo_handle.rights.contains(Rights::READ) {
                return -13; // -EACCES
            }
        }
        for i in 0..config.num_outputs as usize {
            let vmo_h = config.outputs[i].vmo_handle;
            let vmo_handle = match proc.handle_table.get(vmo_h) {
                Ok(h) => h,
                Err(_) => return -9, // -EBADF
            };
            if vmo_handle.object_type != ObjectType::VirtualMemoryObject {
                return -9; // -EBADF
            }
            if !vmo_handle.rights.contains(Rights::WRITE) {
                return -13; // -EACCES
            }
        }

        // 6. Access graph mutably and validate/append
        let mut pool = GRAPH_POOL.lock();
        if graph_id > 0 && graph_id <= pool.len() && pool[graph_id - 1].allocated {
            let graph = &mut pool[graph_id - 1];
            if graph.num_nodes >= MAX_NODES_PER_GRAPH {
                return -22; // -EINVAL
            }
            let new_node_id = graph.num_nodes;

            // Read predecessor dependencies
            let mut deps = [0usize; MAX_DEPENDENCIES];
            if dependency_count > 0 {
                let deps_slice = unsafe { core::slice::from_raw_parts(dependency_array_ptr as *const u32, dependency_count) };
                for i in 0..dependency_count {
                    let dep = deps_slice[i] as usize;
                    if dep >= new_node_id {
                        return -22; // -EINVAL (Predecessor does not exist)
                    }
                    deps[i] = dep;
                }
            }

            // Fill the new TaskNode
            let mut node = TaskNode::new_empty();
            node.node_id = new_node_id;
            node.op_type = op_type_enum;
            node.execution_target = exec_target;
            node.state = NodeState::Pending;
            node.num_inputs = config.num_inputs as usize;
            for i in 0..node.num_inputs {
                node.inputs[i] = config.inputs[i];
            }
            node.num_outputs = config.num_outputs as usize;
            for i in 0..node.num_outputs {
                node.outputs[i] = config.outputs[i];
            }
            node.dependency_count = dependency_count;
            node.dependencies[..dependency_count].copy_from_slice(&deps[..dependency_count]);
            node.remaining_dependencies = dependency_count;

            // Setup successor links
            for &pred_id in deps[..dependency_count].iter() {
                let s_count = graph.successor_counts[pred_id];
                if s_count >= MAX_DEPENDENCIES {
                    return -22; // -EINVAL
                }
                graph.node_successors[pred_id][s_count] = new_node_id;
                graph.successor_counts[pred_id] += 1;
            }

            graph.nodes[new_node_id] = node;
            graph.num_nodes += 1;

            // Print node addition trace
            crate::print!("[NEURAL_SCHED] Node {} added to Graph {} ({:?}, Target: {:?}, Inputs: [", new_node_id, graph_id, node.op_type, node.execution_target);
            for i in 0..node.num_inputs {
                if i > 0 { crate::print!(", "); }
                crate::print!("VMO {}", node.inputs[i].vmo_handle);
            }
            crate::print!("], Outputs: [");
            for i in 0..node.num_outputs {
                if i > 0 { crate::print!(", "); }
                crate::print!("VMO {}", node.outputs[i].vmo_handle);
            }
            crate::print!("], Deps: [");
            for i in 0..node.dependency_count {
                if i > 0 { crate::print!(", "); }
                crate::print!("{}", node.dependencies[i]);
            }
            crate::println!("])");

            new_node_id as isize
        } else {
            -9 // -EBADF
        }
    }).unwrap_or(-3) // -EPERM
}

pub fn sys_graph_submit(graph_handle: usize, queue_handle: usize) -> isize {
    crate::process::with_current_process(|proc| {
        // 1. Retrieve handles
        let graph_h = match proc.handle_table.get(graph_handle) {
            Ok(h) => h,
            Err(_) => return -9, // -EBADF
        };
        if graph_h.object_type != ObjectType::TaskGraph {
            return -9; // -EBADF
        }
        if !graph_h.rights.contains(Rights::EXECUTE) {
            return -13; // -EACCES
        }
        let graph_id = graph_h.object_ptr;

        let queue_h = match proc.handle_table.get(queue_handle) {
            Ok(h) => h,
            Err(_) => return -9, // -EBADF
        };
        if queue_h.object_type != ObjectType::DeviceQueue {
            return -9; // -EBADF
        }
        if !queue_h.rights.contains(Rights::WRITE) {
            return -13; // -EACCES
        }

        // 2. Perform submission logic on graph
        let mut pool = GRAPH_POOL.lock();
        if graph_id > 0 && graph_id <= pool.len() && pool[graph_id - 1].allocated {
            let graph = &mut pool[graph_id - 1];
            if graph.num_nodes == 0 {
                return -22; // -EINVAL: empty graph
            }
            if graph.active_execution {
                return -22; // -EINVAL: already running
            }
            let all_completed = graph.nodes.iter().take(graph.num_nodes).all(|n| n.state == NodeState::Completed);
            if all_completed {
                return -22; // -EINVAL: already completed
            }

            crate::println!("[NEURAL_SCHED] Process PID {} submitted Graph {} to HeterogeneousQueue", proc.pid, graph_id);
            crate::println!("[NEURAL_SCHED] Validating Graph {} topology...", graph_id);

            // DAG check and topological sort
            let mut state = [0u8; MAX_NODES_PER_GRAPH]; // 0 = unvisited, 1 = visiting, 2 = visited
            let mut order = [0usize; MAX_NODES_PER_GRAPH];
            let mut idx = graph.num_nodes;
            
            fn dfs(node_idx: usize, graph: &TaskGraph, state: &mut [u8], order: &mut [usize], idx: &mut usize) -> Result<(), ()> {
                state[node_idx] = 1;
                let count = graph.successor_counts[node_idx];
                for s in 0..count {
                    let succ_id = graph.node_successors[node_idx][s];
                    if let Some(succ_idx) = (0..graph.num_nodes).find(|&i| graph.nodes[i].node_id == succ_id) {
                        if state[succ_idx] == 1 {
                            return Err(());
                        }
                        if state[succ_idx] == 0 {
                            dfs(succ_idx, graph, state, order, idx)?;
                        }
                    }
                }
                state[node_idx] = 2;
                *idx -= 1;
                order[*idx] = graph.nodes[node_idx].node_id;
                Ok(())
            }
            
            for i in 0..graph.num_nodes {
                if state[i] == 0
                    && dfs(i, graph, &mut state, &mut order, &mut idx).is_err() {
                        return -40; // -ELOOP
                    }
            }

            crate::print!("[NEURAL_SCHED] Topological sort: [");
            for (i, &node_idx) in order[..graph.num_nodes].iter().enumerate() {
                if i > 0 { crate::print!(" -> "); }
                crate::print!("Node {}", node_idx);
            }
            crate::println!("]. No cycles detected.");

            // 3. VMO handle translations
            crate::println!("[NEURAL_SCHED] Translating VMO handles to physical coordinates...");
            for i in 0..graph.num_nodes {
                let node = &graph.nodes[i];
                let mut desc = QueueDescriptor::new_empty();
                desc.graph_id = graph.graph_id;
                desc.node_id = node.node_id;
                desc.op_type = node.op_type;
                desc.num_inputs = node.num_inputs;
                for j in 0..node.num_inputs {
                    let tensor = &node.inputs[j];
                    let vmo_handle = match proc.handle_table.get(tensor.vmo_handle) {
                        Ok(h) => h,
                        Err(_) => return -9, // -EBADF
                    };
                    let base_virt = vmo_handle.object_ptr;
                    let virt_addr = base_virt + tensor.offset;
                    let phys_addr = match translate_user_address(&mut proc.page_table, virt_addr) {
                        Some(pa) => pa,
                        None => return -14, // -EFAULT
                    };
                    desc.inputs_phys[j] = phys_addr;
                    desc.input_sizes[j] = tensor.size;
                }
                desc.num_outputs = node.num_outputs;
                for j in 0..node.num_outputs {
                    let tensor = &node.outputs[j];
                    let vmo_handle = match proc.handle_table.get(tensor.vmo_handle) {
                        Ok(h) => h,
                        Err(_) => return -9, // -EBADF
                    };
                    let base_virt = vmo_handle.object_ptr;
                    let virt_addr = base_virt + tensor.offset;
                    let phys_addr = match translate_user_address(&mut proc.page_table, virt_addr) {
                        Some(pa) => pa,
                        None => return -14, // -EFAULT
                    };
                    desc.outputs_phys[j] = phys_addr;
                    desc.output_sizes[j] = tensor.size;
                }
                graph.queue_descriptors[i] = desc;
            }

            // Print the translation trace for outputs
            for i in 0..graph.num_nodes {
                let node = &graph.nodes[i];
                for j in 0..node.num_outputs {
                    let desc = &graph.queue_descriptors[i];
                    crate::println!("[NEURAL_SCHED]   Node {} Output (VMO {}) -> Phys Addr 0x{:X} (Size: {} bytes)", node.node_id, node.outputs[j].vmo_handle, desc.outputs_phys[j], desc.output_sizes[j]);
                }
            }

            // 4. Enqueue starting nodes and start execution
            crate::println!("[NEURAL_SCHED] Verification successful. Enqueuing starting nodes.");
            graph.active_execution = true;

            for idx in 0..graph.num_nodes {
                let node = &mut graph.nodes[idx];
                if node.dependency_count == 0 {
                    node.state = NodeState::Ready;
                     let desc = graph.queue_descriptors[idx];
                     let mut target_queue = node.execution_target;
                     if target_queue == DeviceType::Auto {
                          let size_bytes = desc.output_sizes[0];
                          let best_dev = super::select_optimal_device(desc.op_type, size_bytes);
                          crate::println!("[NEURAL_SCHED] Dynamic Routing Decision: Node {} ({:?}) -> {:?}", node.node_id, desc.op_type, best_dev);
                          node.execution_target = best_dev;
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
                              crate::println!("[NEURAL_SCHED] Enqueued Node {} ({:?}) to GPU queue (Index: {}). Doorbell 0x89000000 triggered.", node.node_id, desc.op_type, index);
                          }
                          DeviceType::Npu => {
                              let mut q = NPU_QUEUE.lock();
                              let index = q.head;
                              let _ = q.enqueue(desc);
                              crate::println!("[NEURAL_SCHED] Enqueued Node {} ({:?}) to NPU queue (Index: {}). Doorbell 0x89001000 triggered.", node.node_id, desc.op_type, index);
                          }
                          _ => {}
                     }
                }
            }

            0 // Success
        } else {
            -9 // -EBADF
        }
    }).unwrap_or(-3) // -EPERM
}

pub fn sys_graph_wait(graph_handle: usize, timeout_us: usize) -> isize {
    let res = crate::process::with_current_process(|proc| {
        // 1. Retrieve handle
        let handle = match proc.handle_table.get(graph_handle) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF
        };
        if handle.object_type != ObjectType::TaskGraph {
            return Err(-9); // -EBADF
        }
        if !handle.rights.contains(Rights::READ) {
            return Err(-13); // -EACCES
        }
        Ok(handle.object_ptr)
    });

    let graph_id = match res {
        Some(Ok(id)) => id,
        Some(Err(err)) => return err,
        None => return -3, // -EPERM
    };

    // 2. Poll/Wait loop
    let start_time = read_time();
    let timeout_ticks = if timeout_us == usize::MAX {
        u64::MAX
    } else {
        (timeout_us as u64) * 10
    };

    loop {
        let pool = GRAPH_POOL.lock();
        if graph_id > 0 && graph_id <= pool.len() && pool[graph_id - 1].allocated {
            let g = &pool[graph_id - 1];
            let all_completed = g.nodes.iter().take(g.num_nodes).all(|n| n.state == NodeState::Completed);
            if all_completed {
                return 0; // Success
            }
            // Check for failures
            for n in g.nodes.iter().take(g.num_nodes) {
                if let NodeState::Failed(err) = n.state {
                    return err;
                }
            }
        } else {
            return -9; // -EBADF
        }
        drop(pool);

        if timeout_us != usize::MAX && (read_time() - start_time) >= timeout_ticks {
            return -110; // -ETIMEDOUT
        }

        crate::process::thread::schedule();
    }
}

pub fn sys_policy_configure(op: usize, arg1: usize, arg2: usize) -> isize {
    match op {
        0 => { // GET_STATS
            let ptr = arg1;
            let size = arg2;
            if ptr == 0 || size < 72 {
                return -22; // -EINVAL
            }
            if ptr % 4 != 0 {
                return -22; // -EINVAL: unaligned pointer for stats copy
            }
            
            // Validate stats copy pointer
            let valid = crate::process::with_current_process(|proc| {
                proc.validate_user_buffer(ptr, 72, true)
            }).unwrap_or(false);

            if !valid {
                return -14; // -EFAULT
            }

            let stats = super::POLICY_STATS.lock();
            let src_ptr = stats.predicted_ticks_per_byte.as_ptr() as *const u8;
            unsafe {
                core::ptr::copy_nonoverlapping(src_ptr, ptr as *mut u8, 72);
            }
            0
        }
        1 => { // SET_EXPLORATION
            let rate_bits = arg1 as u32;
            let rate = f32::from_bits(rate_bits);
            if !(0.0..=1.0).contains(&rate) {
                return -22; // -EINVAL
            }
            let mut stats = super::POLICY_STATS.lock();
            stats.exploration_rate = rate;
            0
        }
        2 => { // RESET_STATS
            let mut stats = super::POLICY_STATS.lock();
            *stats = super::PolicyStats::new();
            0
        }
        _ => -22 // -EINVAL
    }
}
