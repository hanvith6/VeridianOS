//! System Call Handlers for Phase 8 Semantic Graph Filesystem

use super::types::{
    ObjectType as SemObjectType, RelType, Property,
    QueryPredicate, PropertiesInit, MAX_PROPERTIES,
};
use super::store::{GRAPH_STORE, with_node_mut};
use crate::capability::{Handle, ObjectType, Rights};
use crate::memory::{PageTableFlags, PAGE_SIZE};

/// Create a new semantic graph node (SYS_NODE_CREATE = 60)
pub fn sys_node_create(object_type: u8, blob_size: usize, properties_init_ptr: usize) -> isize {
    let sem_type = SemObjectType::from_u8(object_type);

    // 1. Allocate node in GraphStore
    let mut store = GRAPH_STORE.lock();
    let pid = crate::process::thread::current_pid();
    let node_id = match store.alloc_node(sem_type, pid as u32) {
        Ok(id) => id,
        Err(_) => return -12, // -ENOMEM
    };
    drop(store);

    // 2. Allocate and map VMO if blob_size > 0
    if blob_size > 0 {
        let num_pages = blob_size.div_ceil(PAGE_SIZE);
        
        // Dynamically assign virtual address region starting at 0x5000_0000
        static NEXT_VIRT_ADDR: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0x5000_0000);
        let virt_base = NEXT_VIRT_ADDR.fetch_add(num_pages * PAGE_SIZE, core::sync::atomic::Ordering::Relaxed);

        let map_res = crate::process::with_current_process(|proc| {
            for page_idx in 0..num_pages {
                let paddr = match crate::memory::alloc_page() {
                    Some(pa) => pa,
                    None => return Err(-12), // -ENOMEM
                };
                
                if unsafe {
                    proc.page_table.map(
                        virt_base + page_idx * PAGE_SIZE,
                        paddr,
                        PageTableFlags::READ | PageTableFlags::WRITE | PageTableFlags::USER,
                    )
                }.is_err() {
                    return Err(-14); // -EFAULT
                }
            }
            Ok(())
        });

        match map_res {
            Some(Ok(())) => {}
            Some(Err(err_code)) => return err_code,
            None => return -3, // -EPERM
        }

        // Insert VMO handle
        let vmo_handle = Handle::new(
            ObjectType::VirtualMemoryObject,
            virt_base,
            Rights::READ | Rights::WRITE | Rights::DUPLICATE,
        );
        
        let vmo_handle_id = match crate::process::with_current_process(|proc| {
            proc.handle_table.insert(vmo_handle)
        }) {
            Some(Ok(hid)) => hid,
            Some(Err(_)) => return -12, // -ENOMEM
            None => return -3, // -EPERM
        };

        // Update node in store with VMO details
        with_node_mut(node_id, |node| {
            node.vmo_handle = vmo_handle_id;
            node.blob_size = blob_size;
        });
    }

    // 3. Initialize properties if provided
    if properties_init_ptr != 0 {
        // Validate properties_init_ptr before read
        let valid = crate::process::with_current_process(|proc| {
            proc.validate_user_buffer(properties_init_ptr, core::mem::size_of::<PropertiesInit>(), false)
        }).unwrap_or(false);

        if !valid {
            return -14; // -EFAULT
        }

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

    match crate::process::with_current_process(|proc| {
        proc.handle_table.insert(node_handle)
    }) {
        Some(Ok(hid)) => hid as isize,
        Some(Err(_)) => -12, // -ENOMEM
        None => -3, // -EPERM
    }
}

/// Add an edge between nodes (SYS_EDGE_ADD = 61)
pub fn sys_edge_add(src_node_handle: usize, rel_type: u16, target_id: u64) -> isize {
    let src_id = match crate::process::with_current_process(|proc| {
        // Retrieve and validate source node handle
        let handle = match proc.handle_table.get(src_node_handle) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF
        };
        if handle.object_type != ObjectType::GraphNode {
            return Err(-9); // -EBADF
        }
        if !handle.rights.contains(Rights::WRITE) {
            return Err(-13); // -EACCES
        }
        Ok(handle.object_ptr as u64)
    }) {
        Some(Ok(id)) => id,
        Some(Err(err)) => return err,
        None => return -3, // -EPERM
    };

    let rel = RelType::from_u16(rel_type);

    let mut store = GRAPH_STORE.lock();
    match store.add_edge(src_id, rel, target_id) {
        Ok(_) => 0,
        Err(_) => -22, // -EINVAL
    }
}

/// Write data into node's VMO blob (SYS_NODE_WRITE = 62)
pub fn sys_node_write(node_handle: usize, src_ptr: usize, length: usize, offset: usize) -> isize {
    // Validate that src_ptr points to a valid user memory region of length before copy
    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(src_ptr, length, false)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

    let res = crate::process::with_current_process(|proc| {
        // 1. Get and validate GraphNode handle
        let handle = match proc.handle_table.get(node_handle) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF
        };
        if handle.object_type != ObjectType::GraphNode {
            return Err(-9); // -EBADF
        }
        if !handle.rights.contains(Rights::WRITE) {
            return Err(-13); // -EACCES
        }

        let node_id = handle.object_ptr as u64;

        // 2. Fetch VMO details from store
        let (vmo_handle_id, blob_size) = match super::store::with_node(node_id, |node| {
            (node.vmo_handle, node.blob_size)
        }) {
            Some(val) => val,
            None => return Err(-9), // -EBADF
        };

        if vmo_handle_id == 0 {
            return Err(-22); // -EINVAL: node has no VMO blob
        }

        if offset + length > blob_size {
            return Err(-22); // -EINVAL: write out of bounds
        }

        // 3. Get VMO handle from process table
        let vmo_handle = match proc.handle_table.get(vmo_handle_id) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF
        };
        if vmo_handle.object_type != ObjectType::VirtualMemoryObject {
            return Err(-9); // -EBADF
        }

        Ok(vmo_handle.object_ptr)
    });

    let vmo_virt_base = match res {
        Some(Ok(base)) => base,
        Some(Err(err)) => return err,
        None => return -3, // -EPERM
    };

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
    // Validate predicate_ptr and out_buf_ptr
    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(predicate_ptr, core::mem::size_of::<QueryPredicate>(), false)
            && proc.validate_user_buffer(out_buf_ptr, max_results * core::mem::size_of::<u64>(), true)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

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

/// Delete a semantic graph node and reclaim its VMO memory (SYS_NODE_DELETE = 64)
pub fn sys_node_delete(node_handle: usize) -> isize {
    let res = crate::process::with_current_process(|proc| {
        // 1. Retrieve the GraphNode handle to verify it exists and has WRITE rights.
        let handle = match proc.handle_table.get(node_handle) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF: Bad file descriptor/handle
        };

        if handle.object_type != ObjectType::GraphNode {
            return Err(-9); // -EBADF
        }

        if !handle.rights.contains(Rights::WRITE) {
            return Err(-13); // -EACCES: Permission denied
        }

        let node_id = handle.object_ptr as u64;

        // 2. Fetch VMO details from store
        let (vmo_handle_id, blob_size) = match super::store::with_node(node_id, |node| {
            (node.vmo_handle, node.blob_size)
        }) {
            Some(val) => val,
            None => return Err(-9), // -EBADF
        };

        // 3. If there is an associated VMO, unmap its pages and free physical frames
        if vmo_handle_id > 0 && blob_size > 0 {
            let vmo_handle = match proc.handle_table.get(vmo_handle_id) {
                Ok(h) => h,
                Err(_) => return Err(-9), // -EBADF
            };
            if vmo_handle.object_type != ObjectType::VirtualMemoryObject {
                return Err(-9); // -EBADF
            }
            let virt_base = vmo_handle.object_ptr;
            let num_pages = blob_size.div_ceil(PAGE_SIZE);

            for page_idx in 0..num_pages {
                let va = virt_base + page_idx * PAGE_SIZE;
                if let Some(entry) = proc.page_table.get_entry_mut(va) {
                    if entry.is_valid() {
                        let paddr = entry.physical_address();
                        entry.clear();
                        unsafe { crate::memory::free_page(paddr); }
                    }
                }
            }
            // Remove VMO handle from process table
            let _ = proc.handle_table.remove(vmo_handle_id);

            // Flush TLB
            unsafe {
                core::arch::asm!("sfence.vma");
            }
        }

        // 4. Remove GraphNode handle from process table
        let _ = proc.handle_table.remove(node_handle);

        // 5. Mark node in GraphStore as unallocated
        let mut store = super::store::GRAPH_STORE.lock();
        for slot in store.nodes.iter_mut() {
            if slot.allocated && slot.id == node_id {
                slot.allocated = false;
                slot.id = super::types::OBJECT_ID_NULL;
                slot.vmo_handle = 0;
                slot.blob_size = 0;
                slot.ref_count = 0;
                slot.owner_pid = 0;
                break;
            }
        }

        // 6. Clean up references pointing to this node in other nodes' edge lists
        for slot in store.nodes.iter_mut() {
            if slot.allocated {
                let mut write_idx = 0;
                let count = slot.edges.count;
                for i in 0..count {
                    if slot.edges.store[i].target != node_id {
                        slot.edges.store[write_idx] = slot.edges.store[i];
                        write_idx += 1;
                    }
                }
                slot.edges.count = write_idx;
            }
        }

        Ok(0)
    });

    match res {
        Some(Ok(val)) => val,
        Some(Err(err)) => err,
        None => -3, // -EPERM
    }
}
