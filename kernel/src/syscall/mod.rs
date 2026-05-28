//! System Call Dispatcher for VeridianOS
//!
//! Handles and executes Supervisor system calls triggered by User-space via `ecall`.
//!
//! References:
//! - RISC-V Privileged Architecture Manual v1.12 §4.1.8 (Supervisor Cause Register)
//! - seL4 System Call API models

pub mod numbers;
use crate::dist;

use crate::capability::{Handle, Rights};
use crate::println;
use crate::process::Process;
use spin::Mutex;

// A simple static process instance representing the currently executing process has been removed
// in favor of process::PROCESS_TABLE and process::with_current_process.

/// Global entry point for system calls.
///
/// Dispatches the system call to its specific implementation based on the `id` argument.
///
/// Parameters:
/// - `id`: The system call number (normally passed in register `a7`).
/// - `arg0` - `arg4`: Arguments for the system call (normally in `a0` - `a4`).
///
/// Returns the result of the system call (negative values indicate errors).
pub fn syscall_handler(
    id: usize,
    arg0: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
) -> isize {
    #[cfg(debug_assertions)]
    println!("[SYSCALL Debug] id={}, args=(0x{:X}, 0x{:X}, 0x{:X}, 0x{:X}, 0x{:X})", id, arg0, arg1, arg2, arg3, arg4);
    match id {
        numbers::SYS_WRITE => sys_write(arg0, arg1),
        numbers::SYS_EXIT => sys_exit(arg0 as i32),
        numbers::SYS_HANDLE_CLOSE => sys_handle_close(arg0),
        numbers::SYS_HANDLE_DUPLICATE => sys_handle_duplicate(arg0, arg1),
        numbers::SYS_SPAWN => sys_spawn(arg0, arg1),
        numbers::SYS_YIELD => sys_yield(),
        numbers::SYS_WAIT => sys_wait(arg0),
        numbers::SYS_MAP => sys_map(arg0, arg1, arg2),
        numbers::SYS_UNMAP => sys_unmap(arg0, arg1),
        numbers::SYS_GRAPH_CREATE => crate::nes::sys_graph_create(),
        numbers::SYS_GRAPH_ADD_NODE => crate::nes::sys_graph_add_node(arg0, arg1, arg2, arg3, arg4),
        numbers::SYS_GRAPH_SUBMIT => crate::nes::sys_graph_submit(arg0, arg1),
        numbers::SYS_GRAPH_WAIT => crate::nes::sys_graph_wait(arg0, arg1),
        numbers::SYS_NODE_CREATE => crate::semantic_graph::sys_node_create(arg0 as u8, arg1, arg2),
        numbers::SYS_EDGE_ADD => crate::semantic_graph::sys_edge_add(arg0, arg1 as u16, arg2 as u64),
        numbers::SYS_NODE_WRITE => crate::semantic_graph::sys_node_write(arg0, arg1, arg2, arg3),
        numbers::SYS_GRAPH_QUERY => crate::semantic_graph::sys_graph_query(arg0, arg1, arg2),
        numbers::SYS_NODE_DELETE => crate::semantic_graph::sys_node_delete(arg0),
        numbers::SYS_AGENT_SPAWN => crate::agent::sys_agent_spawn(arg0, arg1, arg2),
        numbers::SYS_CHANNEL_CREATE => crate::agent::sys_channel_create(arg0),
        numbers::SYS_CHANNEL_SEND => crate::agent::sys_channel_send(arg0, arg1, arg2),
        numbers::SYS_CHANNEL_RECV => crate::agent::sys_channel_recv(arg0, arg1, arg2),
        numbers::SYS_AGENT_STATUS => crate::agent::sys_agent_status(arg0, arg1),
        numbers::SYS_POLICY_CONFIGURE => crate::nes::sys_policy_configure(arg0, arg1, arg2),
        numbers::SYS_DOMAIN_JOIN => dist::syscalls::sys_domain_join(arg0, arg1, arg2, arg3, arg4),
        numbers::SYS_DOMAIN_LIST => dist::syscalls::sys_domain_list(arg0, arg1, arg2),
        numbers::SYS_DOMAIN_STATUS => dist::syscalls::sys_domain_status(arg0, arg1),
        numbers::SYS_GRAPH_DISPATCH_REMOTE => dist::syscalls::sys_graph_dispatch_remote(arg0, arg1, arg2),
        numbers::SYS_GRAPH_WAIT_REMOTE => dist::syscalls::sys_graph_wait_remote(arg0, arg1, arg2),
        numbers::SYS_GRAPH_ABORT_REMOTE => dist::syscalls::sys_graph_abort_remote(arg0, arg1),
        numbers::SYS_CAP_EXPORT => dist::syscalls::sys_cap_export(arg0, arg1, arg2),
        numbers::SYS_CAP_IMPORT => dist::syscalls::sys_cap_import(arg0, arg1, arg2),
        numbers::SYS_CAP_REVOKE_REMOTE => dist::syscalls::sys_cap_revoke_remote(arg0, arg1),
        numbers::SYS_SGF_REPLICATE_ENABLE => dist::syscalls::sys_sgf_replicate_enable(arg0, arg1),
        numbers::SYS_SGF_REPLICATE_QUERY => dist::syscalls::sys_sgf_replicate_query(arg0, arg1, arg2),
        numbers::SYS_SGF_RAFT_STATUS => dist::syscalls::sys_sgf_raft_status(arg0, arg1),
        _ => {
            println!("[SYSCALL] Warning: Unknown system call ID: {}", id);
            -1 // ENOSYS: Function not implemented
        }
    }
}

/// System Call: Write a string to the UART console.
///
/// Parameters:
/// - `str_ptr`: Raw pointer to the string bytes.
/// - `len`: The length of the string in bytes.
fn sys_write(str_ptr: usize, len: usize) -> isize {
    // Validate that the user process buffer is valid and mapped
    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(str_ptr, len, false)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT: Bad address
    }

    let slice = unsafe { core::slice::from_raw_parts(str_ptr as *const u8, len) };

    if let Ok(s) = core::str::from_utf8(slice) {
        crate::print!("{}", s);
        len as isize
    } else {
        -1 // EFAULT: Bad address / invalid encoding
    }
}

/// System Call: Terminate the current process.
fn sys_exit(status: i32) -> ! {
    println!("[SYSCALL] Process exited with status code: {}", status);
    crate::process::with_current_process(|proc| {
        proc.state = crate::process::ProcessState::Exited(status);
    });
    crate::process::thread::exit_current_thread();
}

/// System Call: Close a capability handle.
fn sys_handle_close(handle_id: usize) -> isize {
    crate::process::with_current_process(|proc| {
        match proc.handle_table.remove(handle_id) {
            Ok(handle) => {
                println!(
                    "[SYSCALL] Closed handle {}: Type {:?} pointing to 0x{:X}",
                    handle_id, handle.object_type, handle.object_ptr
                );
                0 // Success
            }
            Err(e) => {
                println!("[SYSCALL] Close handle error: {}", e);
                -2 // EBADF: Bad file descriptor/handle
            }
        }
    }).unwrap_or(-3) // EPERM: No active process
}

/// System Call: Duplicate a capability handle.
fn sys_handle_duplicate(src_handle_id: usize, rights_mask: usize) -> isize {
    crate::process::with_current_process(|proc| {
        // 1. Retrieve the source handle to verify it exists and has DUPLICATE rights.
        let src_handle = match proc.handle_table.get(src_handle_id) {
            Ok(h) => h,
            Err(_) => return -2, // EBADF
        };

        if !src_handle.rights.contains(Rights::DUPLICATE) {
            println!("[SYSCALL] Error: Handle does not have DUPLICATE rights");
            return -13; // EACCES: Permission denied
        }

        // 2. Determine new rights (either inherit original rights or apply a subset).
        let new_rights = if rights_mask == 0 {
            src_handle.rights
        } else {
            let mask = Rights::from_bits_truncate(rights_mask as u32);
            // Ensure the new rights are only a subset of the original rights (cannot escalate rights!)
            src_handle.rights.intersection(mask)
        };

        // 3. Create the duplicated handle.
        let new_handle = Handle::new(src_handle.object_type, src_handle.object_ptr, new_rights);

        // 4. Insert into the process's handle table.
        match proc.handle_table.insert(new_handle) {
            Ok(new_id) => {
                println!(
                    "[SYSCALL] Duplicated handle {} -> {} (new rights: {:?})",
                    src_handle_id, new_id, new_rights
                );
                new_id as isize
            }
            Err(e) => {
                println!("[SYSCALL] Duplicate handle error: {}", e);
                -12 // ENOMEM: Out of memory/table slots
            }
        }
    }).unwrap_or(-3) // EPERM
}

/// System Call: Spawn a new process from a RAMFS binary name.
fn sys_spawn(name_ptr: usize, name_len: usize) -> isize {
    // 1. Validate the user-supplied string buffer
    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(name_ptr, name_len, false)
    }).unwrap_or(false);

    if !valid || name_len == 0 || name_len > 100 {
        return -14; // -EFAULT: Bad address
    }

    // 2. Copy the name string from user space
    let slice = unsafe { core::slice::from_raw_parts(name_ptr as *const u8, name_len) };
    let name_str = match core::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return -22, // -EINVAL: Invalid argument
    };

    // 3. Find the binary in RamFs
    let elf_data = match crate::fs::RamFs::find(name_str) {
        Some(data) => data,
        None => {
            println!("[SYS_SPAWN] Error: Binary '{}' not found in RamFs", name_str);
            return -2; // -ENOENT: No such file or directory
        }
    };

    // 4. Spawn the process
    match crate::process::spawn(name_str, elf_data) {
        Ok(tid) => tid as isize,
        Err(_) => -12, // -ENOMEM: Out of memory
    }
}

/// System Call: Yield CPU time to the next ready thread.
fn sys_yield() -> isize {
    crate::process::thread::schedule();
    0
}

/// System Call: Wait for a thread/process to exit.
fn sys_wait(tid: usize) -> isize {
    loop {
        let (found, exited) = crate::process::thread::check_thread_status(tid);

        if !found || exited {
            return 0; // Target exited or doesn't exist
        }

        crate::process::thread::schedule();
    }
}

/// System Call: Map memory dynamically.
fn sys_map(virt_addr: usize, len: usize, flags_raw: usize) -> isize {
    if len == 0 || len % crate::memory::PAGE_SIZE != 0 || virt_addr % crate::memory::PAGE_SIZE != 0 {
        return -22; // -EINVAL
    }
    if virt_addr >= 0x8000_0000 || virt_addr.checked_add(len).map_or(true, |end| end > 0x8000_0000) {
        return -14; // -EFAULT
    }

    let mut flags = crate::memory::PageTableFlags::USER;
    if flags_raw & 1 != 0 { flags |= crate::memory::PageTableFlags::READ; }
    if flags_raw & 2 != 0 { flags |= crate::memory::PageTableFlags::WRITE; }
    if flags_raw & 4 != 0 { flags |= crate::memory::PageTableFlags::EXECUTE; }

    let res = crate::process::with_current_process(|proc| {
        let num_pages = len / crate::memory::PAGE_SIZE;
        for i in 0..num_pages {
            let va = virt_addr + i * crate::memory::PAGE_SIZE;
            if let Some(entry) = proc.page_table.get_entry_mut(va) {
                if entry.is_valid() {
                    return Err(-22); // -EINVAL: already mapped
                }
            }

            let paddr = match crate::memory::alloc_page() {
                Some(pa) => pa,
                None => return Err(-12), // -ENOMEM
            };

            // Zero page
            unsafe { core::ptr::write_bytes(paddr as *mut u8, 0, crate::memory::PAGE_SIZE); }

            if unsafe { proc.page_table.map(va, paddr, flags) }.is_err() {
                unsafe { crate::memory::free_page(paddr); }
                return Err(-14); // -EFAULT
            }
        }
        Ok(0)
    });

    match res {
        Some(Ok(val)) => {
            // Flush TLB
            unsafe { core::arch::asm!("sfence.vma"); }
            val
        }
        Some(Err(err)) => err,
        None => -3, // -EPERM
    }
}

/// System Call: Unmap memory dynamically.
fn sys_unmap(virt_addr: usize, len: usize) -> isize {
    if len == 0 || len % crate::memory::PAGE_SIZE != 0 || virt_addr % crate::memory::PAGE_SIZE != 0 {
        return -22; // -EINVAL
    }
    if virt_addr >= 0x8000_0000 || virt_addr.checked_add(len).map_or(true, |end| end > 0x8000_0000) {
        return -14; // -EFAULT
    }

    let res = crate::process::with_current_process(|proc| {
        let num_pages = len / crate::memory::PAGE_SIZE;
        for i in 0..num_pages {
            let va = virt_addr + i * crate::memory::PAGE_SIZE;
            if let Some(entry) = proc.page_table.get_entry_mut(va) {
                if entry.is_valid() {
                    let paddr = entry.physical_address();
                    entry.clear();
                    unsafe { crate::memory::free_page(paddr); }
                } else {
                    return Err(-22); // -EINVAL: not mapped
                }
            } else {
                return Err(-22); // -EINVAL: not mapped
            }
        }
        Ok(0)
    });

    match res {
        Some(Ok(val)) => {
            // Flush TLB
            unsafe { core::arch::asm!("sfence.vma"); }
            val
        }
        Some(Err(err)) => err,
        None => -3, // -EPERM
    }
}
