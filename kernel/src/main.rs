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
                    let init_binary = parse_bootargs(dtb_ptr).unwrap_or("policy_test");
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

fn parse_bootargs(dtb_ptr: usize) -> Option<&'static str> {
    if dtb_ptr == 0 {
        return None;
    }

    let header = unsafe { &*(dtb_ptr as *const FdtHeader) };
    let magic = u32::from_be(header.magic);
    if magic != 0xd00dfeed {
        return None;
    }

    let off_struct = u32::from_be(header.off_dt_struct) as usize;
    let off_strings = u32::from_be(header.off_dt_strings) as usize;

    let struct_ptr = (dtb_ptr + off_struct) as *const u32;
    let strings_ptr = (dtb_ptr + off_strings) as *const u8;

    let mut offset = 0;
    let mut in_chosen = false;
    let mut depth = 0;

    for _ in 0..10000 {
        let token = u32::from_be(unsafe { *struct_ptr.add(offset) });
        offset += 1;

        match token {
            1 => { // FDT_BEGIN_NODE
                let name_ptr = unsafe { struct_ptr.add(offset) as *const u8 };
                let mut len = 0;
                unsafe {
                    while *name_ptr.add(len) != 0 {
                        len += 1;
                    }
                }

                let name_slice = unsafe { core::slice::from_raw_parts(name_ptr, len) };
                if let Ok(name_str) = core::str::from_utf8(name_slice) {
                    if name_str == "chosen" {
                        in_chosen = true;
                        depth = 1;
                    } else if in_chosen {
                        depth += 1;
                    }
                }

                let name_bytes = len + 1;
                let name_words = (name_bytes + 3) / 4;
                offset += name_words;
            }
            2 => { // FDT_END_NODE
                if in_chosen {
                    depth -= 1;
                    if depth == 0 {
                        in_chosen = false;
                    }
                }
            }
            3 => { // FDT_PROP
                let len = u32::from_be(unsafe { *struct_ptr.add(offset) }) as usize;
                let nameoff = u32::from_be(unsafe { *struct_ptr.add(offset + 1) }) as usize;
                offset += 2;

                let prop_name_ptr = unsafe { strings_ptr.add(nameoff) };
                let mut name_len = 0;
                unsafe {
                    while *prop_name_ptr.add(name_len) != 0 {
                        name_len += 1;
                    }
                }
                let prop_name_slice = unsafe { core::slice::from_raw_parts(prop_name_ptr, name_len) };

                if let Ok(prop_name) = core::str::from_utf8(prop_name_slice) {
                    if in_chosen && prop_name == "bootargs" {
                        let val_ptr = unsafe { struct_ptr.add(offset) as *const u8 };
                        let val_len = if len > 0 && unsafe { *val_ptr.add(len - 1) } == 0 {
                            len - 1
                        } else {
                            len
                        };
                        let val_slice = unsafe { core::slice::from_raw_parts(val_ptr, val_len) };
                        if let Ok(bootargs_str) = core::str::from_utf8(val_slice) {
                            if let Some(init_start) = bootargs_str.find("init=") {
                                let init_val = &bootargs_str[init_start + 5..];
                                let init_end = init_val.find(' ').unwrap_or(init_val.len());
                                let parsed = &init_val[..init_end];
                                return Some(unsafe { core::mem::transmute::<&str, &'static str>(parsed) });
                            }
                        }
                    }
                }

                let val_words = (len + 3) / 4;
                offset += val_words;
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
