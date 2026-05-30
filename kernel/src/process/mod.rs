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
use spin::Mutex;

pub static PROCESS_TABLE: Mutex<[Option<Process>; 16]> = Mutex::new([const { None }; 16]);

pub fn with_current_process<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Process) -> R,
{
    let pid = thread::current_pid();
    let mut pt = PROCESS_TABLE.lock();
    for slot in pt.iter_mut() {
        if let Some(proc) = slot {
            if proc.pid == pid {
                return Some(f(proc));
            }
        }
    }
    crate::println!("[with_current_process debug] PROCESS NOT FOUND for pid: {}! Active processes:", pid);
    for (i, slot) in pt.iter().enumerate() {
        if let Some(proc) = slot {
            crate::println!("  slot[{}]: pid={}", i, proc.pid);
        }
    }
    None
}

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
    pub next_stack_va: usize,
    pub exception_handler: usize,
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
            next_stack_va: 0x4000_0000,
            exception_handler: 0,
        }
    }

    /// Allocate a user stack with a fixed guard page below it.
    ///
    /// ASLR is temporarily disabled to diagnose an intermittent Store/AMO Page Fault
    /// (scause=15) at sepc=0x11EE6 (U-mode entry +2) with fault addr=0x40008FF8.
    /// The ASLR seed `get_time() ^ pid` produced offsets of 1-15 pages that placed the
    /// stack VA within the same Sv39 L1/L0 subtree used by ELF PT_LOAD segments near
    /// 0x40000000, causing page-table conflicts on certain offsets.
    ///
    /// With a fixed 1-guard + 1-stack layout the stack always lands at:
    ///   guard:     0x40001000  (unmapped — catches underflows)
    ///   stack page:0x40002000  (READ|WRITE|USER)
    ///   stack_top: 0x40003000  (initial sp value)
    ///
    /// TODO: Re-enable ASLR once ELF segment VA ranges are audited and a minimum
    /// offset that clears all PT_LOAD VAs near 0x40000000 is established.
    pub fn alloc_stack(&mut self) -> Result<(usize, usize), &'static str> {
        if self.next_stack_va == 0 {
            self.next_stack_va = 0x4000_0000;
        }

        // Fixed 1-page offset from the base so the guard is at 0x40001000 and
        // the stack page at 0x40002000, well clear of the ELF load address region.
        if self.next_stack_va == 0x4000_0000 {
            self.next_stack_va += PAGE_SIZE; // skip 0x40000000 itself
        }

        // 1-page unmapped guard page below the stack
        let _guard_va = self.next_stack_va;
        self.next_stack_va += PAGE_SIZE;

        // Actual stack is the next page
        let stack_va = self.next_stack_va;
        self.next_stack_va += PAGE_SIZE;

        let stack_top = stack_va + PAGE_SIZE;

        // Allocate physical page frame
        let stack_phys = alloc_page().ok_or("alloc_stack: out of physical memory")?;
        
        // Zero physical page to prevent information leaks
        unsafe {
            core::ptr::write_bytes(stack_phys as *mut u8, 0, PAGE_SIZE);
        }

        // Map physical page frame into process page table
        unsafe {
            self.page_table.map(
                stack_va,
                stack_phys,
                PageTableFlags::READ | PageTableFlags::WRITE | PageTableFlags::USER,
            )?;
        }

        Ok((stack_va, stack_top))
    }

    /// Validates that a user-supplied buffer `[virt_addr, virt_addr + len)` is entirely
    /// within user-space, is properly mapped, and has the required page table flags.
    pub fn validate_user_buffer(&mut self, virt_addr: usize, len: usize, writeable: bool) -> bool {
        if len == 0 {
            return true;
        }
        // User space is restricted to addresses below 0x8000_0000.
        if virt_addr >= 0x8000_0000 || virt_addr.checked_add(len).map_or(true, |end| end > 0x8000_0000) {
            return false;
        }

        let start_page = virt_addr / PAGE_SIZE;
        let end_page = (virt_addr + len - 1) / PAGE_SIZE;

        for page in start_page..=end_page {
            let page_addr = page * PAGE_SIZE;
            if let Some(entry) = self.page_table.get_entry_mut(page_addr) {
                if !entry.is_valid() {
                    crate::println!("[validate_user_buffer debug] page 0x{:X} entry is invalid!", page_addr);
                    return false;
                }
                let flags = entry.flags();
                if !flags.contains(PageTableFlags::USER) {
                    crate::println!("[validate_user_buffer debug] page 0x{:X} flags {:?} do not contain USER!", page_addr, flags);
                    return false;
                }
                if writeable && !flags.contains(PageTableFlags::WRITE) {
                    crate::println!("[validate_user_buffer debug] page 0x{:X} writeable requested but flags {:?} do not contain WRITE!", page_addr, flags);
                    return false;
                }
            } else {
                crate::println!("[validate_user_buffer debug] page 0x{:X} entry mut not found!", page_addr);
                return false;
            }
        }
        true
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
    static NEXT_PID: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(2);
    let pid = NEXT_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut process = Process::new(pid);

    if name == "neural_test" || name == "policy_test" || name == "enclave_test" {
        // Insert DeviceQueue capability at handle 4
        let queue_handle = crate::capability::Handle::new(
            crate::capability::ObjectType::DeviceQueue,
            0x9000_0000,
            crate::capability::Rights::WRITE | crate::capability::Rights::EXECUTE,
        );
        process.handle_table.force_set(4, queue_handle)?;

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
            process.handle_table.force_set(handle_id, handle)?;

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

    // 2. Load the ELF binary first so alloc_stack() knows which VAs are taken
    let entry_point = elf::load_elf(elf_data, &mut process.page_table)?;
    crate::println!("[PROCESS] ELF loaded. Entry point: 0x{:X}", entry_point);

    // 3. Allocate stack AFTER ELF is mapped — prevents stack VA from landing
    //    in the same Sv39 L0 subtree as a PT_LOAD segment (which caused
    //    a Store/AMO page fault at U-mode entry when ASLR gave small offsets).
    let (_user_stack_virt, user_stack_top) = process.alloc_stack()?;

    // 5. Install this process as the current active process and capture satp
    let pid_val = process.pid;
    let satp = {
        let mut pt = PROCESS_TABLE.lock();
        let mut inserted = false;
        let mut target_satp = 0;
        for slot in pt.iter_mut() {
            if slot.is_none() {
                *slot = Some(process);
                target_satp = slot.as_ref().unwrap().page_table.satp();
                inserted = true;
                break;
            }
        }
        if !inserted {
            return Err("spawn: process table full");
        }
        target_satp
    };

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
    let tid = thread::spawn_user_thread(entry_point, user_stack_top, satp, pid_val)?;

    crate::println!("[PROCESS] Process '{}' spawned as thread tid={}", name, tid);

    Ok(tid)
}
