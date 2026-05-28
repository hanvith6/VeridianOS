//! ELF64 Executable Loader for VeridianOS
//!
//! Parses standard 64-bit ELF executable files and maps their PT_LOAD segments
//! into a target process's page table.
//!
//! References:
//! - ELF-64 Object File Format Specification
//! - Asterinas ELF loading mechanisms (USENIX ATC'25)

use crate::memory::{PAGE_SIZE, PageTable, PageTableFlags};

/// The Magic number identifying ELF files.
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// ELF64 Header structure at the beginning of the file.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Header {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF64 Program Header entry describing memory segments.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// Parse and load an ELF64 binary into a PageTable.
///
/// Returns the entry point address (`e_entry`) of the loaded executable.
pub fn load_elf(elf_data: &[u8], page_table: &mut PageTable) -> Result<usize, &'static str> {
    // 1. Validate ELF data size fits header
    if elf_data.len() < core::mem::size_of::<Elf64Header>() {
        return Err("ELF data is too small to contain header");
    }

    // 2. Cast header pointer safely
    let header = unsafe { &*(elf_data.as_ptr() as *const Elf64Header) };

    // 3. Verify ELF Magic and architecture constraints
    if header.e_ident[0..4] != ELF_MAGIC {
        return Err("Invalid ELF magic");
    }
    if header.e_ident[4] != 2 {
        return Err("Unsupported ELF class (must be ELF64)");
    }
    if header.e_ident[5] != 1 {
        return Err("Unsupported ELF data format (must be Little Endian)");
    }
    if header.e_machine != 0xF3 {
        return Err("Unsupported architecture machine (must be RISC-V)");
    }

    // 4. Locate Program Headers
    let ph_offset = header.e_phoff as usize;
    let ph_num = header.e_phnum as usize;
    let ph_size = header.e_phentsize as usize;

    if ph_offset + ph_num * ph_size > elf_data.len() {
        return Err("Program headers segment out of bounds");
    }

    crate::println!(
        "[ELF] Loading ELF binary. Entry point: 0x{:X}, segments: {}",
        header.e_entry,
        ph_num
    );

    // 5. Parse and map each PT_LOAD segment
    for i in 0..ph_num {
        let ph_ptr = unsafe { elf_data.as_ptr().add(ph_offset + i * ph_size) as *const Elf64Phdr };
        let phdr = unsafe { &*ph_ptr };

        // We only care about PT_LOAD (type 1) segments
        if phdr.p_type == 1 {
            let start_vaddr = phdr.p_vaddr as usize;
            let end_vaddr = start_vaddr + phdr.p_memsz as usize;
            let file_end_vaddr = start_vaddr + phdr.p_filesz as usize;

            // Align start and end virtual addresses to 4KB page boundaries
            let start_page = start_vaddr & !(PAGE_SIZE - 1);
            let end_page = (end_vaddr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

            crate::println!(
                "  [PT_LOAD] Segment {}: vaddr [0x{:X} - 0x{:X}), memsz: 0x{:X}, filesz: 0x{:X}",
                i,
                start_vaddr,
                end_vaddr,
                phdr.p_memsz,
                phdr.p_filesz
            );

            for page_vaddr in (start_page..end_page).step_by(PAGE_SIZE) {
                // Determine permissions for this segment
                let mut segment_flags = PageTableFlags::USER | PageTableFlags::VALID;
                if (phdr.p_flags & 1) != 0 {
                    segment_flags |= PageTableFlags::EXECUTE;
                }
                if (phdr.p_flags & 2) != 0 {
                    segment_flags |= PageTableFlags::WRITE;
                }
                if (phdr.p_flags & 4) != 0 {
                    segment_flags |= PageTableFlags::READ;
                }

                let phys_frame = if let Some(entry) = page_table.get_entry_mut(page_vaddr) {
                    if entry.is_valid() {
                        // Page is already mapped!
                        // Merge new permissions with the existing ones (bitwise OR)
                        let merged_flags = entry.flags() | segment_flags;
                        let phys = entry.physical_address();
                        entry.set(phys, merged_flags);
                        phys
                    } else {
                        // Leaf slot exists but is not mapped yet. Allocate and map it.
                        let new_frame = crate::memory::alloc_page()
                            .ok_or("Out of memory during ELF segment loading")?;
                        entry.set(
                            new_frame,
                            segment_flags
                                | PageTableFlags::VALID
                                | PageTableFlags::ACCESSED
                                | PageTableFlags::DIRTY,
                        );
                        new_frame
                    }
                } else {
                    // Allocate a new physical frame for this page (leaf table doesn't exist)
                    let new_frame = crate::memory::alloc_page()
                        .ok_or("Out of memory during ELF segment loading")?;

                    // Map virtual page to physical frame
                    unsafe {
                        page_table.map(page_vaddr, new_frame, segment_flags)?;
                    }
                    new_frame
                };

                // Determine file data intersection with this page and copy
                let copy_start = core::cmp::max(page_vaddr, start_vaddr);
                let copy_end = core::cmp::min(page_vaddr + PAGE_SIZE, file_end_vaddr);

                if copy_start < copy_end {
                    let offset_in_page = copy_start - page_vaddr;
                    let offset_in_file = copy_start - start_vaddr + phdr.p_offset as usize;
                    let len = copy_end - copy_start;

                    unsafe {
                        let dest = (phys_frame + offset_in_page) as *mut u8;
                        let src = elf_data.as_ptr().add(offset_in_file);
                        core::ptr::copy_nonoverlapping(src, dest, len);
                    }
                }
            }
        }
    }

    Ok(header.e_entry as usize)
}
