//! Memory Management Coordinator for VeridianOS
//!
//! Exposes physical page allocation and virtual page tables mapping.
//!
//! References:
//! - RISC-V Privileged Architecture Manual v1.12
//! - Asterinas memory safety page initialization (USENIX ATC'25)

pub mod page_alloc;
pub mod page_table;

pub use page_alloc::{PAGE_SIZE, alloc_page, free_page};
pub use page_table::{PageTable, PageTableFlags};
use spin::Mutex;

// Import symbols defined in our linker script (linker.ld)
unsafe extern "C" {
    fn _free_mem_start();
}

/// The root page table for the kernel.
pub static KERNEL_PAGE_TABLE: Mutex<PageTable> = Mutex::new(PageTable::new());

/// Initialize the memory subsystem.
///
/// Parameters:
/// - `_dtb_ptr`: A pointer to the device tree blob describing RAM size.
pub fn init(_dtb_ptr: usize) {
    // 1. Get the physical starting address for free memory from the linker script symbol.
    let free_mem_start = _free_mem_start as *const () as usize;
    crate::println!("[BOOT MEMORY] free_mem_start = 0x{:X}", free_mem_start);

    // 2. Set the end of RAM.
    // On the QEMU 'virt' board, default memory starts at 0x8000_0000 and is 128MB.
    // 0x8000_0000 + 128MB (0x0800_0000) = 0x8800_0000.
    let ram_end = 0x8800_0000;

    // 3. Initialize the physical page frame allocator.
    page_alloc::init(free_mem_start, ram_end);

    // Run buddy allocator tests to verify correctness
    page_alloc::test_page_alloc();

    // 4. Set up the initial virtual mappings for kernel space.
    let mut root_table = KERNEL_PAGE_TABLE.lock();

    // Map kernel sections and all physical RAM (from 0x8020_0000 up to ram_end) using identity mapping.
    // In identity mapping, virtual address == physical address.
    let kernel_start = 0x8020_0000;
    let mut addr = kernel_start;
    while addr < ram_end {
        unsafe {
            root_table
                .map(
                    addr,
                    addr,
                    PageTableFlags::READ | PageTableFlags::WRITE | PageTableFlags::EXECUTE,
                )
                .expect("Failed to map RAM memory");
        }
        addr += PAGE_SIZE;
    }

    // Map the UART MMIO space (0x1000_0000) to allow serial writes once paging is enabled.
    unsafe {
        root_table
            .map(
                0x1000_0000,
                0x1000_0000,
                PageTableFlags::READ | PageTableFlags::WRITE,
            )
            .expect("Failed to map UART MMIO space");
    }

    // Map the VirtIO MMIO region: QEMU's virt machine provides 8 VirtIO slots
    // at 0x1000_1000..0x1000_8FFF. We map 16 slots (0x1000_1000..0x1001_0FFF)
    // for diagnostic scanning with room to spare.
    // Reference: QEMU hw/riscv/virt.c — VIRT_VIRTIO region
    for slot in 0..16usize {
        let mmio_page = 0x1000_1000 + slot * PAGE_SIZE;
        unsafe {
            root_table
                .map(
                    mmio_page,
                    mmio_page,
                    PageTableFlags::READ | PageTableFlags::WRITE,
                )
                .expect("Failed to map VirtIO MMIO page");
        }
    }

    // Map MMIO Doorbell Registers for CPU, GPU and NPU (0x8900_0000, 0x8900_1000, 0x8900_2000)
    // Since these are simulated doorbells, we back them with real physical memory pages
    // so that writes/reads to them don't cause physical access faults on QEMU.
    for page in 0..3 {
        let virt_addr = 0x8900_0000 + page * PAGE_SIZE;
        let phys_frame = alloc_page().expect("Failed to allocate physical page for doorbell");
        unsafe {
            root_table
                .map(
                    virt_addr,
                    phys_frame,
                    PageTableFlags::READ | PageTableFlags::WRITE,
                )
                .expect("Failed to map Doorbell MMIO page");
        }
    }

    // 5. Enable paging by loading the root page table into the satp register.
    unsafe {
        root_table.activate();
    }
}

