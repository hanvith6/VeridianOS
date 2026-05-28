//! Process Management for VeridianOS
//!
//! A process is the execution namespace of a program.
//!
//! It contains:
//! - An address space (represented by its virtual page table).
//! - A private capability handle table.
//! - Process metadata (PID, state).
//!
//! References:
//! - RISC-V Privileged Architecture Manual v1.12
//! - seL4 Process Isolation Models
//! - Zircon (Fuchsia) process creation model

pub mod elf;
pub mod thread;

use crate::capability::{HandleTable, Handle, ObjectType, Rights};
use crate::memory::{alloc_page, PageTable, PageTableFlags, PAGE_SIZE};

/// Represents the execution state of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Exited(i32),
}

/// The main structure representing a process.
pub struct Process {
    pub pid: usize,
    pub state: ProcessState,
    pub page_table: PageTable,
    pub handle_table: HandleTable,
}

impl Process {
    /// Create a new process with a fresh page table.
    ///
    /// The new page table has all kernel mappings copied in so the process
    /// can take traps and execute kernel code on syscall/exception paths.
    pub fn new(pid: usize) -> Self {
        let mut pt = PageTable::new();
        pt.copy_kernel_mappings();
        
        let mut handle_table = HandleTable::new();
        
        // Pre-insert CPU, GPU, NPU queue capabilities into the handle table
        let cpu_q_ptr = &raw const crate::nes::queue::CPU_QUEUE as usize;
        let gpu_q_ptr = &raw const crate::nes::queue::GPU_QUEUE as usize;
        let npu_q_ptr = &raw const crate::nes::queue::NPU_QUEUE as usize;
        
        let _ = handle_table.insert(Handle::new(
            ObjectType::DeviceQueue,
            cpu_q_ptr,
            Rights::WRITE | Rights::READ | Rights::DUPLICATE,
        ));
        let _ = handle_table.insert(Handle::new(
            ObjectType::DeviceQueue,
            gpu_q_ptr,
            Rights::WRITE | Rights::READ | Rights::DUPLICATE,
        ));
        let _ = handle_table.insert(Handle::new(
            ObjectType::DeviceQueue,
            npu_q_ptr,
            Rights::WRITE | Rights::READ | Rights::DUPLICATE,
        ));

        Self {
            pid,
            state: ProcessState::Ready,
            page_table: pt,
            handle_table,
        }
    }
}

/// Spawn a new isolated user-space process from a raw ELF binary blob.
///
/// This function:
/// 1. Creates a new `Process` with a fresh Sv39 page table (kernel mappings copied in).
/// 2. Allocates and maps a 4KB user stack at virtual address `0x4000_2000`.
/// 3. Parses the ELF64 binary and maps all `PT_LOAD` segments into the new page table.
/// 4. Retrieves the `satp` value for the new address space.
/// 5. Stores the process as the active `CURRENT_PROCESS`.
/// 6. Spawns a kernel thread that transitions to U-mode at the ELF entry point.
///
/// # Arguments
/// * `name`     – Human-readable label for log output (e.g. "hello")
/// * `elf_data` – Slice of the raw ELF64 binary bytes
///
/// # Returns
/// `Ok(tid)` — the thread ID of the newly created thread, or `Err` on failure.
pub fn spawn(name: &str, elf_data: &'static [u8]) -> Result<usize, &'static str> {
    crate::println!("[PROCESS] Spawning process '{}' ({} bytes ELF)", name, elf_data.len());

    // 1. Create a new process with an isolated page table
    let mut process = Process::new(2); // PID 2 (PID 1 is the root process)

    // 2. Allocate and map the user stack
    //    Stack occupies one page at 0x4000_2000 → 0x4000_3000
    //    The stack pointer starts at the top: 0x4000_3000
    let stack_phys = alloc_page().ok_or("spawn: out of memory for user stack")?;
    let user_stack_virt = 0x4000_2000usize;
    let user_stack_top  = 0x4000_3000usize;

    unsafe {
        process.page_table.map(
            user_stack_virt,
            stack_phys,
            PageTableFlags::READ | PageTableFlags::WRITE | PageTableFlags::USER,
        )?;
    }

    if name == "neural_test" || name == "policy_test" {
        // Insert DeviceQueue capability at handle 4
        let queue_handle = crate::capability::Handle::new(
            crate::capability::ObjectType::DeviceQueue,
            0x9000_0000,
            crate::capability::Rights::WRITE | crate::capability::Rights::EXECUTE,
        );
        process.handle_table.set(4, queue_handle)?;

        // Insert and map VMO capabilities
        let vmo_configs = [
            (5, 0x4010_0000usize, 0x8610_0000usize),
            (6, 0x4011_0000usize, 0x8610_4000usize),
            (7, 0x4012_0000usize, 0x8610_8000usize),
            (8, 0x4013_0000usize, 0x8610_C000usize),
            (9, 0x4014_0000usize, 0x8611_0000usize),
            (10, 0x4015_0000usize, 0x8611_4000usize),
        ];

        for &(handle_id, virt_base, phys_base) in &vmo_configs {
            let handle = crate::capability::Handle::new(
                crate::capability::ObjectType::VirtualMemoryObject,
                virt_base,
                crate::capability::Rights::READ | crate::capability::Rights::WRITE | crate::capability::Rights::DUPLICATE,
            );
            process.handle_table.set(handle_id, handle)?;

            // Map 4 pages (16KB) for each VMO
            for page_idx in 0..4 {
                let vaddr = virt_base + page_idx * 4096;
                let paddr = phys_base + page_idx * 4096;
                unsafe {
                    process.page_table.map(
                        vaddr,
                        paddr,
                        PageTableFlags::READ | PageTableFlags::WRITE | PageTableFlags::USER,
                    )?;
                }
            }
        }
    }

    // Zero out the stack page so there's no stale kernel data visible in U-mode
    unsafe {
        core::ptr::write_bytes(stack_phys as *mut u8, 0, PAGE_SIZE);
    }

    // 3. Load the ELF binary — maps all PT_LOAD segments into the process page table
    let entry_point = elf::load_elf(elf_data, &mut process.page_table)?;
    crate::println!("[PROCESS] ELF loaded. Entry point: 0x{:X}", entry_point);

    // 4. Capture the satp value *before* moving the process into the global slot
    //    (taking the lock inside a function argument would deadlock)
    let satp = process.page_table.satp();

    // 5. Install this process as the current active process
    *crate::syscall::CURRENT_PROCESS.lock() = Some(process);

    // Debug: Check PTE for 0x80219120 in KERNEL_PAGE_TABLE
    {
        let mut kpt = crate::memory::KERNEL_PAGE_TABLE.lock();
        if let Some(entry) = kpt.get_entry_mut(0x80219120) {
            crate::println!("[DEBUG] PTE for 0x80219120: valid={}, address=0x{:X}, flags={:?}", entry.is_valid(), entry.physical_address(), entry.flags());
        } else {
            crate::println!("[DEBUG] PTE for 0x80219120: NOT FOUND in KERNEL_PAGE_TABLE!");
        }
    }

    // 6. Spawn the thread — it will flush the TLB and sret into U-mode
    let tid = thread::spawn_user_thread(entry_point, user_stack_top, satp)?;

    crate::println!("[PROCESS] Process '{}' spawned as thread tid={}", name, tid);

    Ok(tid)
}
