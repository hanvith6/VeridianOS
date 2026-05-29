//! Sv39 Virtual Memory Page Table Implementation for VeridianOS
//!
//! RISC-V Sv39 paging translates a 39-bit virtual address to a 56-bit physical address.
//! The translation uses a 3-level page tree:
//! - Level 2 (Root)
//! - Level 1
//! - Level 0 (Leaves, mapping 4KB pages)
//!
//! References:
//! - RISC-V Privileged Architecture Manual v1.12 §4.3 (Sv39 Paging)
//! - Asterinas memory safety page translation models (USENIX ATC'25)

use super::page_alloc;
use bitflags::bitflags;

bitflags! {
    /// Sv39 Page Table Entry (PTE) Flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PageTableFlags: u64 {
        const VALID = 1 << 0;
        const READ = 1 << 1;
        const WRITE = 1 << 2;
        const EXECUTE = 1 << 3;
        const USER = 1 << 4;
        const GLOBAL = 1 << 5;
        const ACCESSED = 1 << 6;
        const DIRTY = 1 << 7;
    }
}

/// An Sv39 Page Table Entry (PTE) representing a 64-bit value.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    /// Create a new empty (invalid) PTE.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Check if the entry is valid.
    pub fn is_valid(&self) -> bool {
        (self.0 & PageTableFlags::VALID.bits()) != 0
    }

    /// Check if the entry is a leaf page (contains read/write/execute permissions).
    pub fn is_leaf(&self) -> bool {
        (self.0
            & (PageTableFlags::READ.bits()
                | PageTableFlags::WRITE.bits()
                | PageTableFlags::EXECUTE.bits()))
            != 0
    }

    /// Extract the physical page number (PPN) and convert to a physical address.
    pub fn physical_address(&self) -> usize {
        // PPN occupies bits 10-53 in RISC-V Sv39 Page Table Entries.
        ((self.0 >> 10) & 0x003F_FFFF_FFFF) as usize * page_alloc::PAGE_SIZE
    }

    /// Set the entry's physical page address and flags.
    pub fn set(&mut self, phys_addr: usize, flags: PageTableFlags) {
        assert!(
            phys_addr.is_multiple_of(page_alloc::PAGE_SIZE),
            "Physical address must be page-aligned"
        );
        let ppn = (phys_addr / page_alloc::PAGE_SIZE) as u64;
        // PPN starts at bit 10. Combine with flags.
        self.0 = (ppn << 10) | flags.bits();
    }

    /// Clear the entry (mark invalid).
    pub fn clear(&mut self) {
        self.0 = 0;
    }

    /// Retrieve the flags currently set in this PTE.
    pub fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.0)
    }
}

/// An Sv39 Page Table consisting of 512 entries (occupies exactly one 4KB page).
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [PageTableEntry; 512],
}

impl Default for PageTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PageTable {
    /// Create a new, blank page table in memory.
    pub const fn new() -> Self {
        Self {
            entries: [PageTableEntry::empty(); 512],
        }
    }

    /// Helper to translate virtual address components into page table level indices.
    /// RISC-V Sv39 splits virtual addresses into three 9-bit indices:
    /// - Level 2 Index: bits 30-38
    /// - Level 1 Index: bits 21-29
    /// - Level 0 Index: bits 12-20
    const fn index(virt_addr: usize, level: usize) -> usize {
        (virt_addr >> (12 + level * 9)) & 0x1FF
    }

    /// Walk the page table hierarchy down to Level 0 to find or create the leaf entry.
    ///
    /// If intermediate page tables don't exist, they are allocated dynamically.
    ///
    /// Safety: This operates on raw physical addresses. We assume identity mapping is active
    /// for kernel space, so we can cast physical addresses directly to pointers.
    unsafe fn walk_mut(&mut self, virt_addr: usize, create: bool) -> Option<&mut PageTableEntry> {
        let mut table = self;

        // Walk down from Level 2 to Level 1, then Level 1 to Level 0
        for level in (1..=2).rev() {
            let idx = Self::index(virt_addr, level);
            let entry = &mut table.entries[idx];

            if entry.is_valid() {
                // Point to the next level page table in memory
                let next_table_ptr = entry.physical_address() as *mut PageTable;
                table = unsafe { &mut *next_table_ptr };
            } else {
                if !create {
                    return None;
                }
                // Allocate a new physical page for the next level page table
                let new_page_addr = page_alloc::alloc_page()?;
                // Zero the page table page to avoid using garbage mappings.
                unsafe {
                    core::ptr::write_bytes(new_page_addr as *mut u8, 0, page_alloc::PAGE_SIZE);
                }
                // Set the page table entry to point to this new table
                entry.set(new_page_addr, PageTableFlags::VALID);

                let next_table_ptr = new_page_addr as *mut PageTable;
                table = unsafe { &mut *next_table_ptr };
            }
        }

        let leaf_idx = Self::index(virt_addr, 0);
        Some(&mut table.entries[leaf_idx])
    }

    /// Map a 4KB virtual page to a physical frame.
    ///
    /// # Safety
    /// Operating on raw hardware page tables requires extreme caution. Invalid mappings
    /// can trigger page faults or page translation traps.
    pub unsafe fn map(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: PageTableFlags,
    ) -> Result<(), &'static str> {
        assert!(
            virt_addr.is_multiple_of(page_alloc::PAGE_SIZE),
            "Virtual address must be page-aligned"
        );
        assert!(
            phys_addr.is_multiple_of(page_alloc::PAGE_SIZE),
            "Physical address must be page-aligned"
        );

        let leaf_entry = unsafe { self.walk_mut(virt_addr, true) }
            .ok_or("Failed to allocate sub-level page table")?;

        if leaf_entry.is_valid() {
            return Err("Virtual address is already mapped");
        }

        leaf_entry.set(
            phys_addr,
            flags | PageTableFlags::VALID | PageTableFlags::ACCESSED | PageTableFlags::DIRTY,
        );
        Ok(())
    }

    /// Unmap a virtual page.
    ///
    /// # Safety
    /// Unmapping currently in-use kernel code or stack memory will result in immediate crash.
    pub unsafe fn unmap(&mut self, virt_addr: usize) -> Result<(), &'static str> {
        assert!(
            virt_addr.is_multiple_of(page_alloc::PAGE_SIZE),
            "Virtual address must be page-aligned"
        );

        let leaf_entry =
            unsafe { self.walk_mut(virt_addr, false) }.ok_or("Virtual address is not mapped")?;

        if !leaf_entry.is_valid() {
            return Err("Virtual address is not mapped");
        }

        leaf_entry.clear();
        Ok(())
    }

    /// Activate the page table on the CPU.
    ///
    /// Writes the address to the `satp` (Supervisor Address Translation and Protection) control register
    /// and flushes the TLB (Translation Lookaside Buffer).
    ///
    /// # Safety
    ///
    /// The caller must ensure that the page table maps valid kernel and user memory,
    /// and that the lifetime of this page table covers the execution duration.
    pub unsafe fn activate(&self) {
        let satp_val = self.satp();
        unsafe {
            // Write to satp register
            core::arch::asm!("csrw satp, {}", in(reg) satp_val);
            // sfence.vma instruction flushes the TLB to make the new page table mapping active immediately.
            core::arch::asm!("sfence.vma");
        }
    }

    /// Calculate the satp register value for this page table.
    pub fn satp(&self) -> usize {
        let phys_addr = self as *const PageTable as usize;
        debug_assert!(phys_addr < 0x8800_0000); // SAFETY: identity mapping assumed
        let ppn = phys_addr / page_alloc::PAGE_SIZE;
        // MODE: Sv39 (8)
        let mode_sv39 = 8usize << 60;
        mode_sv39 | ppn
    }

    /// Walk the page table to find the leaf entry for a virtual address, if it exists.
    pub fn get_entry_mut(&mut self, virt_addr: usize) -> Option<&mut PageTableEntry> {
        unsafe { self.walk_mut(virt_addr, false) }
    }

    /// Copy kernel mappings from the global kernel page table to this page table.
    /// Since RISC-V Sv39 paging uses disjoint top-level indices for kernel/MMIO vs user,
    /// we can copy all Level 2 entries of the kernel page table except index 1 (user space).
    pub fn copy_kernel_mappings(&mut self) {
        let kpt = crate::memory::KERNEL_PAGE_TABLE.lock();
        for i in 0..512 {
            if i != 1 {
                self.entries[i] = kpt.entries[i];
            }
        }
    }
}

