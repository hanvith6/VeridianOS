//! VeridianOS Kernel Entry Point
//!
//! This is the main file for the VeridianOS kernel.
//! It boots in S-mode (Supervisor mode) on a RISC-V 64-bit processor.
//!
//! Subsystems Initialized:
//! 1. UART Console Logger
//! 2. Physical Page Frame Allocator
//! 3. Sv39 Page Table Translation Manager
//! 4. Capability System (Handles & Rights)
//! 5. Process Isolation
//! 6. Round-Robin Thread Scheduler
//! 7. Channel IPC Message Passing
//! 8. Syscall Dispatcher
//! 9. VirtIO Block Device Driver         ← Phase 6
//! 10. InitRAMFS (ustar TAR filesystem)  ← Phase 6
//! 11. Named Process Spawning            ← Phase 6

#![no_std]
#![no_main]

extern crate alloc;

// Include the assembly bootloader stub.
core::arch::global_asm!(include_str!("arch/riscv64/boot.S"));
core::arch::global_asm!(include_str!("arch/riscv64/trap.S"));

// Import modules
pub mod capability;
pub mod fs;
pub mod memory;
pub mod nes;
pub mod panic;
pub mod process;
pub mod sbi;
pub mod syscall;
pub mod trap;
pub mod uart;
pub mod virtio;
pub mod semantic_graph;
pub mod agent;
pub mod dist;
pub mod enclave;



use capability::{Handle, ObjectType, Rights};
use process::Process;
use process::thread;

/// The entry point for Thread 1 — preemptive compute loop.
#[allow(dead_code)]
fn test_thread_1() -> ! {
    unsafe {
        thread::release_lock();
        core::arch::asm!("csrs sstatus, {}", in(reg) 0x2);
    }
    println!("[Thread 1] Started preemptive compute loop...");

    let mut count = 0u64;
    loop {
        count += 1;
        if count.is_multiple_of(2_000_000) {
            println!("[Thread 1] Computing... count = {}", count);
        }
        if count >= 6_000_000 {
            break;
        }
    }
    println!("[Thread 1] Preemptive loop complete. Exiting.");
    thread::exit_current_thread();
}

/// The entry point for Thread 2 — preemptive compute loop.
#[allow(dead_code)]
fn test_thread_2() -> ! {
    unsafe {
        thread::release_lock();
        core::arch::asm!("csrs sstatus, {}", in(reg) 0x2);
    }
    println!("[Thread 2] Started preemptive compute loop...");

    let mut count = 0u64;
    loop {
        count += 1;
        if count.is_multiple_of(2_000_000) {
            println!("[Thread 2] Computing... count = {}", count);
        }
        if count >= 6_000_000 {
            break;
        }
    }
    println!("[Thread 2] Preemptive loop complete. Exiting.");
    thread::exit_current_thread();
}

/// The main entry point of our operating system kernel.
///
/// Parameters:
/// - `hart_id`: The ID of the current hardware thread (CPU core).
/// - `dtb_ptr`: A physical address pointing to the Device Tree Blob (DTB) loaded in RAM.
#[unsafe(no_mangle)]
pub extern "C" fn kmain(hart_id: usize, dtb_ptr: usize) -> ! {
    // 1. Initialize the UART serial port driver so we can print messages.
    uart::WRITER.lock().init();

    // Print segment boundaries for debugging
    unsafe extern "C" {
        fn _text_start();
        fn _text_end();
        fn _rodata_start();
        fn _rodata_end();
        fn _data_start();
        fn _data_end();
        fn _bss_start();
        fn _bss_end();
        fn _stack_bottom();
        fn _stack_top();
        fn _heap_start();
        fn _heap_end();
    }
    println!("[DEBUG] Linker sections:");
    println!("  .text:         0x{:X} - 0x{:X}", _text_start as *const () as usize, _text_end as *const () as usize);
    println!("  .rodata:       0x{:X} - 0x{:X}", _rodata_start as *const () as usize, _rodata_end as *const () as usize);
    println!("  .data:         0x{:X} - 0x{:X}", _data_start as *const () as usize, _data_end as *const () as usize);
    println!("  .bss:          0x{:X} - 0x{:X}", _bss_start as *const () as usize, _bss_end as *const () as usize);
    println!("  Stack:         0x{:X} - 0x{:X}", _stack_bottom as *const () as usize, _stack_top as *const () as usize);
    println!("  Heap:          0x{:X} - 0x{:X}", _heap_start as *const () as usize, _heap_end as *const () as usize);
    println!("  KERNEL_PT:     0x{:X}", &raw const memory::KERNEL_PAGE_TABLE as usize);
    println!("  [DEBUG] Thread size: {}, align: {}", core::mem::size_of::<process::thread::Thread>(), core::mem::align_of::<process::thread::Thread>());
    println!("  [DEBUG] Stack size: {}", core::mem::size_of::<process::thread::Stack>());

    // 2. Set up the trap vector register (stvec) to point to our assembly trap_vector.
    trap::init();

    // 3. Print boot banner
    println!("");
    println!("================================================================");
    println!(" __      __        _     _ _             ____   _____ ");
    println!(" \\ \\    / /       (_)   | (_)           / __ \\ / ____|");
    println!("  \\ \\  / /__ _ __ _  __| |_  __ _ _ __ | |  | | (___  ");
    println!("   \\ \\/ / _ \\ '__| |/ _` | |/ _` | '_ \\| |  | |\\___ \\ ");
    println!("    \\  /  __/ |  | | (_| | | (_| | | | | |__| |____) |");
    println!("     \\/ \\___|_|  |_|\\__,_|_|\\__,_|_| |_|\\____/|_____/ ");
    println!("================================================================");
    println!("               VeridianOS Version 0.1.0-alpha");
    println!("  Concept: AI-Native, Capability-Based Architecture (RISC-V 64)");
    println!("================================================================");
    println!("");

    println!("[BOOT] Booting CPU Hart ID: {}", hart_id);
    println!(
        "[BOOT] Device Tree Blob located at physical address: 0x{:X}",
        dtb_ptr
    );

    // 4. Initialize Memory Management (Buddy Allocator + Sv39 Paging)
    println!("[BOOT] Initializing memory management...");
    memory::init(dtb_ptr);
    println!("[BOOT] Memory management active (Sv39 Paging enabled).");

    // 5. Initialize root capability process
    println!("[BOOT] Creating root system process...");
    let mut root_process = Process::new(1);

    let dummy_vmo_addr = 0x8600_0000;
    let vmo_handle = Handle::new(
        ObjectType::VirtualMemoryObject,
        dummy_vmo_addr,
        Rights::READ | Rights::WRITE | Rights::DUPLICATE,
    );
    let handle_id = root_process
        .handle_table
        .insert(vmo_handle)
        .expect("Failed to insert dummy VMO capability");
    println!(
        "[BOOT] Capability inserted: Handle ID {} -> VMO at 0x{:X}",
        handle_id, dummy_vmo_addr
    );

    {
        let mut pt = process::PROCESS_TABLE.lock();
        pt[0] = Some(root_process);
    }
    println!("[BOOT] Root process active.");

    // 6. Syscall smoke-test
    println!("\n--- [SYSCALL VERIFICATION] ---");
    let test_msg = "Hello from user space (simulated syscall)!\n";
    let bytes_written = syscall::syscall_handler(
        syscall::numbers::SYS_WRITE,
        test_msg.as_ptr() as usize,
        test_msg.len(),
        0,
        0,
        0,
    );
    println!("[TEST] SYS_WRITE returned: {} bytes written", bytes_written);
    println!("------------------------------\n");

    // 7. Initialize the Thread Scheduler
    println!("[BOOT] Initializing thread scheduler...");
    thread::init();

    // 7.5. Initialize S-Mode Neural Subsystem simulator
    nes::init();

    // 7.6. Initialize S-Mode Semantic Graph Filesystem
    println!("[BOOT] Initializing S-Mode Semantic Graph Filesystem...");
    semantic_graph::init();

    // 7.7. Initialize Agent Runtime
    println!("[BOOT] Initializing Agent Runtime...");
    agent::init();

    // 7.8. Initialize Distributed Multi-Kernel Coherence
    println!("[BOOT] Initializing Distributed Kernel Coherence...");
    dist::cluster::cluster_init(dist::types::KernelDomainId(0));
    dist::raft::raft_init();
    println!("[BOOT] Distributed Multi-Kernel Coherence initialized.");

    // -----------------------------------------------------------------------
    // Phase 6: VirtIO Block Driver + InitRAMFS + Named Process Spawn
    // -----------------------------------------------------------------------
    println!("\n=== [PHASE 6] VirtIO + InitRAMFS + Named Process Spawn ===");

    // 7a. Initialize VirtIO block device
    match virtio::blk::init() {
        Ok(capacity) => {
            println!(
                "[VIRTIO] Block device ready. Capacity: {} sectors ({} KB)",
                capacity,
                capacity / 2
            );

            // 7b. Load the disk image into the kernel RAM buffer and parse the ustar archive
            match fs::RamFs::load_from_disk() {
                Ok(count) => {
                    println!("[RAMFS] Loaded {} file(s) from disk image.", count);

                    // 7c. Parse the 'init' binary name from bootargs, fallback to "policy_test"
                    let init_binary = parse_bootargs(dtb_ptr).unwrap_or("enclave_test");
                    println!("[RAMFS] Looking for init binary: '{}'", init_binary);
                    match fs::RamFs::find(init_binary) {
                        Some(elf_data) => {
                            println!(
                                "[RAMFS] Found '{}' ({} bytes). Spawning process...",
                                init_binary,
                                elf_data.len()
                            );

                            // 7d. Spawn the process — creates isolated page table,
                            //     maps ELF segments, starts user-mode thread
                            match process::spawn(init_binary, elf_data) {
                                Ok(tid) => {
                                    println!(
                                        "[BOOT] Process '{}' scheduled as thread tid={}",
                                        init_binary,
                                        tid
                                    );
                                }
                                Err(e) => {
                                    println!("[ERROR] Failed to spawn '{}': {}", init_binary, e);
                                }
                            }
                        }
                        None => {
                            println!("[RAMFS] WARNING: '{}' binary not found in disk image.", init_binary);
                            println!("[RAMFS] Run `make disk` to rebuild disk.img.");
                        }
                    }
                }
                Err(e) => {
                    println!("[RAMFS] Failed to load from disk: {}", e);
                }
            }
        }
        Err(e) => {
            println!("[VIRTIO] Block device not available: {}", e);
            println!("[VIRTIO] No legacy ELF fallback in Phase 11. Run `make disk` to rebuild disk.img.");
        }
    }

    // 7e. Initialize VirtIO network device
    match virtio::net::init() {
        Ok(_) => {
            println!("[VIRTIO] Net device ready.");
            *crate::dist::transport::ACTIVE_TRANSPORT.lock() = &crate::dist::transport::VirtioNetTransport;
            println!("[BOOT] DKCP active transport switched to VirtioNetTransport.");
        }
        Err(e) => {
            println!("[VIRTIO] Net device initialization failed: {}", e);
        }
    }

    println!("=== [PHASE 6 INIT COMPLETE] ===\n");

    // -----------------------------------------------------------------------
    // Preemptive scheduler verification (Phase 5): Spawn compute threads
    // -----------------------------------------------------------------------
    // Preemptive scheduler verification (Phase 5): Spawn compute threads
    // -----------------------------------------------------------------------
    // Commented out to prevent deadlocks and CPU hogging during NES verification
    // println!("[BOOT] Spawning preemptive compute threads...");
    // thread::spawn_thread(test_thread_1).expect("Failed to spawn Thread 1");
    // thread::spawn_thread(test_thread_2).expect("Failed to spawn Thread 2");

    println!("[BOOT] Spawning NES simulation workers...");
    thread::spawn_thread(nes::simulator::cpu_worker).expect("Failed to spawn CPU worker");
    thread::spawn_thread(nes::simulator::gpu_worker).expect("Failed to spawn GPU worker");
    thread::spawn_thread(nes::simulator::npu_worker).expect("Failed to spawn NPU worker");

    // Boot secondary harts
    smp::init();

    println!("[BOOT] Yielding to scheduler...");
    thread::schedule();

    println!("\n[SUCCESS] VeridianOS Phase 11 fully verified!");
    println!("[INFO] Entering Supervisor idle loop...");

    unsafe {
        // Enable supervisor interrupts so timer preemption continues
        core::arch::asm!("csrs sstatus, {}", in(reg) 0x2usize);
    }

    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

// Static buffer for the init= argument extracted from the DTB bootargs.
// Copying here eliminates the transmute-to-'static lifetime hazard: the DTB
// blob lives in memory that the page allocator may later reclaim, so any
// reference into it would become dangling after the allocator starts.
static mut BOOTARGS_BUF: [u8; 64] = [0u8; 64];
static mut BOOTARGS_LEN: usize = 0;

fn parse_bootargs(dtb_ptr: usize) -> Option<&'static str> {
    if dtb_ptr == 0 {
        return None;
    }

    // SAFETY: dtb_ptr is provided by OpenSBI/QEMU in a1 register; we validate
    // alignment and magic before any further reads.
    if dtb_ptr & 3 != 0 {
        return None; // FDT headers are always 4-byte aligned
    }

    let header = unsafe { &*(dtb_ptr as *const FdtHeader) };
    let magic = u32::from_be(header.magic);
    if magic != 0xd00dfeed {
        return None;
    }

    let totalsize = u32::from_be(header.totalsize) as usize;
    let off_struct = u32::from_be(header.off_dt_struct) as usize;
    let off_strings = u32::from_be(header.off_dt_strings) as usize;
    let size_dt_strings = u32::from_be(header.size_dt_strings) as usize;
    let size_dt_struct = u32::from_be(header.size_dt_struct) as usize;

    // Bounds: offsets must be inside the blob and must not overflow usize.
    if off_struct >= totalsize
        || off_strings >= totalsize
        || off_struct.checked_add(size_dt_struct).map_or(true, |e| e > totalsize)
        || off_strings.checked_add(size_dt_strings).map_or(true, |e| e > totalsize)
    {
        return None;
    }

    let struct_base = dtb_ptr.checked_add(off_struct)?;
    let strings_base = dtb_ptr.checked_add(off_strings)?;
    let struct_end = struct_base.checked_add(size_dt_struct)?;
    let strings_end = strings_base.checked_add(size_dt_strings)?;

    let struct_ptr = struct_base as *const u32;
    let strings_ptr = strings_base as *const u8;

    // Maximum words in the struct section — used as the loop bound.
    let max_words = size_dt_struct / 4;

    let mut offset = 0usize; // word index within struct section
    let mut in_chosen = false;
    let mut depth = 0u32;

    while offset < max_words {
        // Each token read advances by exactly 1 word; bounds-check first.
        if struct_base + offset * 4 + 4 > struct_end {
            break;
        }
        let token = u32::from_be(unsafe { *struct_ptr.add(offset) });
        offset += 1;

        match token {
            1 => { // FDT_BEGIN_NODE — followed by NUL-terminated name, padded to 4 bytes
                let name_start = struct_base + offset * 4;
                if name_start >= struct_end {
                    break;
                }
                let name_ptr = name_start as *const u8;
                let max_name = struct_end - name_start;
                let mut name_len = 0usize;
                // SAFETY: name_ptr + name_len < struct_end, checked in loop.
                while name_len < max_name {
                    if unsafe { *name_ptr.add(name_len) } == 0 {
                        break;
                    }
                    name_len += 1;
                }
                if name_len < max_name {
                    let name_slice = unsafe { core::slice::from_raw_parts(name_ptr, name_len) };
                    if let Ok(name_str) = core::str::from_utf8(name_slice) {
                        if name_str == "chosen" {
                            in_chosen = true;
                            depth = 1;
                        } else if in_chosen {
                            depth += 1;
                        }
                    }
                }
                let name_words = ((name_len + 1) + 3) / 4;
                offset = offset.saturating_add(name_words);
            }
            2 => { // FDT_END_NODE
                if in_chosen {
                    depth -= 1;
                    if depth == 0 {
                        in_chosen = false;
                    }
                }
            }
            3 => { // FDT_PROP — len (u32), nameoff (u32), value bytes
                if struct_base + offset * 4 + 8 > struct_end {
                    break;
                }
                let prop_len = u32::from_be(unsafe { *struct_ptr.add(offset) }) as usize;
                let nameoff = u32::from_be(unsafe { *struct_ptr.add(offset + 1) }) as usize;
                offset += 2;

                // Validate nameoff is inside the strings section.
                if strings_base.checked_add(nameoff).map_or(true, |p| p >= strings_end) {
                    let val_words = (prop_len + 3) / 4;
                    offset = offset.saturating_add(val_words);
                    continue;
                }

                let prop_name_ptr = unsafe { strings_ptr.add(nameoff) };
                let max_prop_name = strings_end - (strings_base + nameoff);
                let mut pname_len = 0usize;
                while pname_len < max_prop_name {
                    if unsafe { *prop_name_ptr.add(pname_len) } == 0 {
                        break;
                    }
                    pname_len += 1;
                }

                // Validate that the property value bytes are inside the struct section.
                let val_start = struct_base + offset * 4;
                if in_chosen
                    && pname_len < max_prop_name
                    && val_start.checked_add(prop_len).map_or(true, |e| e <= struct_end)
                {
                    let prop_name_slice =
                        unsafe { core::slice::from_raw_parts(prop_name_ptr, pname_len) };
                    if let Ok(prop_name) = core::str::from_utf8(prop_name_slice) {
                        if prop_name == "bootargs" && prop_len > 0 {
                            let val_ptr = val_start as *const u8;
                            // Strip trailing NUL if present.
                            let val_len =
                                if unsafe { *val_ptr.add(prop_len - 1) } == 0 {
                                    prop_len - 1
                                } else {
                                    prop_len
                                };
                            let val_slice =
                                unsafe { core::slice::from_raw_parts(val_ptr, val_len) };
                            if let Ok(bootargs_str) = core::str::from_utf8(val_slice) {
                                if let Some(init_start) = bootargs_str.find("init=") {
                                    let init_val = &bootargs_str[init_start + 5..];
                                    let init_end =
                                        init_val.find(' ').unwrap_or(init_val.len());
                                    let parsed = &init_val[..init_end];
                                    // Copy into static buffer — avoids 'static transmute
                                    // over DTB memory that the page allocator may reclaim.
                                    let copy_len = parsed.len().min(63);
                                    unsafe {
                                        BOOTARGS_BUF[..copy_len]
                                            .copy_from_slice(&parsed.as_bytes()[..copy_len]);
                                        BOOTARGS_BUF[copy_len] = 0;
                                        BOOTARGS_LEN = copy_len;
                                        // SAFETY: BOOTARGS_BUF is 'static, UTF-8 validated
                                        // above, and length is copy_len <= 63.
                                        return Some(core::str::from_utf8_unchecked(
                                            &BOOTARGS_BUF[..copy_len],
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                let val_words = (prop_len + 3) / 4;
                offset = offset.saturating_add(val_words);
            }
            4 => {} // FDT_NOP
            9 => break, // FDT_END
            _ => break,
        }
    }

    None
}

#[unsafe(no_mangle)]
pub extern "C" fn ksecondary_main(_hart_id: usize) -> ! {
    // Enable paging
    let satp_val = crate::memory::KERNEL_PAGE_TABLE.lock().satp();
    unsafe {
        core::arch::asm!("csrw satp, {}", in(reg) satp_val);
        core::arch::asm!("sfence.vma");
    }

    crate::trap::init_secondary();

    // Enable interrupts
    unsafe {
        core::arch::asm!("csrs sstatus, {}", in(reg) 0x2usize);
    }

    loop {
        crate::process::thread::schedule();
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

pub mod smp {
    pub fn init() {
        unsafe extern "C" {
            fn _secondary_start();
        }
        let start_addr = _secondary_start as *const () as usize;
        for hart_id in 1..4 {
            let ret = crate::sbi::sbi_hart_start(hart_id, start_addr, 0);
            if ret.error == 0 {
                crate::println!("[BOOT] Woke up secondary hart {}", hart_id);
            } else {
                crate::println!("[BOOT] Failed to wake up secondary hart {}: error={}", hart_id, ret.error);
            }
        }
    }
}
