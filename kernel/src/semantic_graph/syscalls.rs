//! System Call Handlers for Phase 8 Semantic Graph Filesystem

use super::types::{
    ObjectType as SemObjectType, RelType, Property,
    QueryPredicate, PropertiesInit, MAX_PROPERTIES,
};
use super::store::{GRAPH_STORE, with_node_mut};
use crate::capability::{Handle, ObjectType, Rights};
use crate::syscall::CURRENT_PROCESS;
use crate::memory::{PageTableFlags, PAGE_SIZE};

/// Create a new semantic graph node (SYS_NODE_CREATE = 60)
pub fn sys_node_create(object_type: u8, blob_size: usize, properties_init_ptr: usize) -> isize {
    let mut proc_guard = CURRENT_PROCESS.lock();
    let proc = if let Some(ref mut p) = *proc_guard {
        p
    } else {
        return -3; // -EPERM
    };

    let sem_type = SemObjectType::from_u8(object_type);

    // 1. Allocate node in GraphStore
    let mut store = GRAPH_STORE.lock();
    let node_id = match store.alloc_node(sem_type, proc.pid as u32) {
        Ok(id) => id,
        Err(_) => return -12, // -ENOMEM
    };
    drop(store);

    // 2. Allocate and map VMO if blob_size > 0
    if blob_size > 0 {
        let num_pages = blob_size.div_ceil(PAGE_SIZE);
        
        // Dynamically assign virtual address region starting at 0x5000_0000
        static mut NEXT_VIRT_ADDR: usize = 0x5000_0000;
        let virt_base = unsafe {
            let addr = NEXT_VIRT_ADDR;
            NEXT_VIRT_ADDR += num_pages * PAGE_SIZE;
            addr
        };

        for page_idx in 0..num_pages {
            let paddr = match crate::memory::alloc_page() {
                Some(pa) => pa,
                None => return -12, // -ENOMEM
            };
            
            if unsafe {
                proc.page_table.map(
                    virt_base + page_idx * PAGE_SIZE,
                    paddr,
                    PageTableFlags::READ | PageTableFlags::WRITE | PageTableFlags::USER,
                )
            }.is_err() {
                return -14; // -EFAULT
            }
        }

        // Insert VMO handle
        let vmo_handle = Handle::new(
            ObjectType::VirtualMemoryObject,
            virt_base,
            Rights::READ | Rights::WRITE | Rights::DUPLICATE,
        );
        
        let vmo_handle_id = match proc.handle_table.insert(vmo_handle) {
            Ok(hid) => hid,
            Err(_) => return -12, // -ENOMEM
        };

        // Update node in store with VMO details
        with_node_mut(node_id, |node| {
            node.vmo_handle = vmo_handle_id;
            node.blob_size = blob_size;
        });
    }

    // 3. Initialize properties if provided
    if properties_init_ptr != 0 {
        let init = unsafe { &*(properties_init_ptr as *const PropertiesInit) };
        with_node_mut(node_id, |node| {
            let count = core::cmp::min(init.count, MAX_PROPERTIES);
            node.properties.count = count;
            for idx in 0..count {
                node.properties.store[idx] = Property {
                    key: init.keys[idx],
                    val: init.values[idx],
                };
            }
        });
    }

    // 4. Create capability handle for the GraphNode object itself
    let node_handle = Handle::new(
        ObjectType::GraphNode,
        node_id as usize,
        Rights::READ | Rights::WRITE | Rights::DUPLICATE,
    );

    match proc.handle_table.insert(node_handle) {
        Ok(hid) => hid as isize,
        Err(_) => -12, // -ENOMEM
    }
}

/// Add an edge between nodes (SYS_EDGE_ADD = 61)
pub fn sys_edge_add(src_node_handle: usize, rel_type: u16, target_id: u64) -> isize {
    let mut proc_guard = CURRENT_PROCESS.lock();
    let proc = if let Some(ref mut p) = *proc_guard {
        p
    } else {
        return -3; // -EPERM
    };

    // Retrieve and validate source node handle
    let handle = match proc.handle_table.get(src_node_handle) {
        Ok(h) => h,
        Err(_) => return -9, // -EBADF
    };
    if handle.object_type != ObjectType::GraphNode {
        return -9; // -EBADF
    }
    if !handle.rights.contains(Rights::WRITE) {
        return -13; // -EACCES
    }

    let src_id = handle.object_ptr as u64;
    let rel = RelType::from_u16(rel_type);

    let mut store = GRAPH_STORE.lock();
    match store.add_edge(src_id, rel, target_id) {
        Ok(_) => 0,
        Err(_) => -22, // -EINVAL
    }
}

/// Write data into node's VMO blob (SYS_NODE_WRITE = 62)
pub fn sys_node_write(node_handle: usize, src_ptr: usize, length: usize, offset: usize) -> isize {
    let proc_guard = CURRENT_PROCESS.lock();
    let proc = if let Some(ref p) = *proc_guard {
        p
    } else {
        return -3; // -EPERM
    };

    // 1. Get and validate GraphNode handle
    let handle = match proc.handle_table.get(node_handle) {
        Ok(h) => h,
        Err(_) => return -9, // -EBADF
    };
    if handle.object_type != ObjectType::GraphNode {
        return -9; // -EBADF
    }
    if !handle.rights.contains(Rights::WRITE) {
        return -13; // -EACCES
    }

    let node_id = handle.object_ptr as u64;

    // 2. Fetch VMO details from store
    let (vmo_handle_id, blob_size) = match super::store::with_node(node_id, |node| {
        (node.vmo_handle, node.blob_size)
    }) {
        Some(val) => val,
        None => return -9, // -EBADF
    };

    if vmo_handle_id == 0 {
        return -22; // -EINVAL: node has no VMO blob
    }

    if offset + length > blob_size {
        return -22; // -EINVAL: write out of bounds
    }

    // 3. Get VMO handle from process table
    let vmo_handle = match proc.handle_table.get(vmo_handle_id) {
        Ok(h) => h,
        Err(_) => return -9, // -EBADF
    };
    if vmo_handle.object_type != ObjectType::VirtualMemoryObject {
        return -9; // -EBADF
    }

    let vmo_virt_base = vmo_handle.object_ptr;

    // 4. Perform direct memory copy (SUM enabled allows reading user space)
    unsafe {
        let src = src_ptr as *const u8;
        let dst = (vmo_virt_base + offset) as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, length);
    }

    length as isize
}

/// Query the semantic graph (SYS_GRAPH_QUERY = 63)
pub fn sys_graph_query(predicate_ptr: usize, out_buf_ptr: usize, max_results: usize) -> isize {
    let proc_guard = CURRENT_PROCESS.lock();
    if proc_guard.is_none() {
        return -3; // -EPERM
    }
    drop(proc_guard);

    // Read QueryPredicate from user space
    let predicate = unsafe {
        let ptr = predicate_ptr as *const QueryPredicate;
        if ptr.is_null() {
            return -14; // -EFAULT
        }
        &*ptr
    };

    let mut results = [0u64; 64];
    let limit = core::cmp::min(max_results, results.len());

    let store = GRAPH_STORE.lock();
    let count = store.query(predicate, &mut results[..limit]);
    drop(store);

    // Copy result list back to user space buffer
    unsafe {
        let dst = out_buf_ptr as *mut u64;
        core::ptr::copy_nonoverlapping(results.as_ptr(), dst, count);
    }

    count as isize
}
