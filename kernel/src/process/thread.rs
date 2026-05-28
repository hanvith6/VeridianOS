//! Thread Context and Scheduler for VeridianOS
//!
//! A thread is the basic unit of execution.
//!
//! Note: Because the Thread struct contains its own Stack array, moving the Thread
//! struct in memory changes the physical address of the stack. Therefore, we must
//! initialize the stack pointer context *after* the thread is moved to its permanent
//! slot in the scheduler array.
//!
//! References:
//! - RISC-V Privileged Architecture Manual v1.12
//! - "OS in 1000 Lines" (context switching)

use spin::Mutex;
use core::sync::atomic::{AtomicUsize, Ordering};

/// The callee-saved registers that must be preserved during a context switch in RISC-V.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadContext {
    pub ra: usize,      // Return address
    pub sp: usize,      // Stack pointer
    pub s: [usize; 12], // Saved registers s0 - s11
}

/// Represents the execution state of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running(usize), // Running on hart_id
    Blocked,
    Exited,
}

/// A 16-byte aligned stack block for kernel execution.
/// RISC-V requires 16-byte stack alignment.
#[repr(align(16))]
pub struct Stack(pub [u8; 16384]);

/// The main Thread structure.
#[repr(C)]
#[repr(align(16))]
pub struct Thread {
    pub tid: usize,
    pub pid: usize,
    pub state: ThreadState,
    pub context: ThreadContext,
    pub stack: Option<alloc::boxed::Box<Stack>>,
    pub satp: usize,
    pub user_entry: Option<usize>,
    pub user_sp: Option<usize>,
    pub saved_user_context: Option<crate::trap::TrapFrame>,
}

impl Thread {
    /// Create a new thread.
    pub fn new(tid: usize, satp: usize, pid: usize) -> Self {
        Self {
            tid,
            pid,
            state: ThreadState::Ready,
            context: ThreadContext::default(),
            stack: Some(alloc::boxed::Box::new(Stack([0; 16384]))),
            satp,
            user_entry: None,
            user_sp: None,
            saved_user_context: None,
        }
    }

    /// Initialize the execution context for this thread.
    ///
    /// This must be called *after* the Thread struct is in its final static memory location.
    pub fn init_context(&mut self, entry: fn() -> !) {
        if let Some(ref mut stack) = self.stack {
            let stack_top = &**stack as *const Stack as usize + 16384;
            assert!(
                stack_top.is_multiple_of(16),
                "Stack top must be 16-byte aligned"
            );

            self.context.sp = stack_top;
            self.context.ra = entry as usize;
        }
    }
}

// External assembly function to switch CPU execution context between two threads.
unsafe extern "C" {
    fn switch_context(current: *mut ThreadContext, next: *const ThreadContext);
}

/// The maximum number of threads the scheduler can manage.
pub const MAX_THREADS: usize = 16;

/// Read the current hart ID from the thread pointer register (tp)
pub fn get_hart_id() -> usize {
    let id: usize;
    unsafe {
        core::arch::asm!("mv {}, tp", out(reg) id);
    }
    id
}

/// Scheduler state.
struct SchedulerState {
    threads: [Option<Thread>; MAX_THREADS],
    current_idx: [usize; 4],
}

impl SchedulerState {
    const fn new() -> Self {
        Self {
            threads: [const { None }; MAX_THREADS],
            current_idx: [0; 4],
        }
    }

    /// Add a thread to the scheduler queue and initialize its entry context.
    fn spawn(&mut self, thread: Thread, entry: fn() -> !) -> Result<usize, &'static str> {
        for (idx, slot) in self.threads.iter_mut().enumerate() {
            if slot.is_none() {
                // Place in the slot first
                *slot = Some(thread);
                // Now get a mutable reference to the moved struct to initialize its context
                if let Some(t) = slot {
                    t.init_context(entry);
                    crate::println!(
                        "[BOOT] Thread context initialized: tid={}, sp=0x{:X}, ra=0x{:X}",
                        t.tid,
                        t.context.sp,
                        t.context.ra
                    );
                }
                return Ok(idx);
            }
        }
        Err("Thread queue is full")
    }

    /// Try to switch the current hart to the next ready thread.
    /// Returns `true` if a thread is running, `false` if the current thread is blocked/exited and no other ready threads exist.
    fn try_schedule(&mut self) -> bool {
        let hart_id = get_hart_id();
        let curr_idx = self.current_idx[hart_id];
        let mut next_idx = curr_idx;

        // Find the next thread that is in the Ready state
        let found = loop {
            next_idx = (next_idx + 1) % MAX_THREADS;
            if let Some(ref t) = self.threads[next_idx] {
                if t.state == ThreadState::Ready {
                    break true;
                }
            }
            if next_idx == curr_idx {
                break false;
            }
        };

        if found {
            let current_ptr = &mut self.threads[curr_idx] as *mut Option<Thread>;
            let next_ptr = &mut self.threads[next_idx] as *mut Option<Thread>;

            unsafe {
                let current_opt = &mut *current_ptr;
                let next_opt = &mut *next_ptr;

                if let (Some(curr), Some(next)) = (current_opt, next_opt) {
                    if let ThreadState::Running(h) = curr.state {
                        if h == hart_id {
                            curr.state = ThreadState::Ready;
                        }
                    }
                    next.state = ThreadState::Running(hart_id);
                    self.current_idx[hart_id] = next_idx;

                    // Switch page tables to the target thread's address space
                    let next_satp = next.satp;
                    core::arch::asm!("csrw satp, {}", in(reg) next_satp);
                    core::arch::asm!("sfence.vma");

                    // Perform the physical register swap
                    switch_context(&mut curr.context, &next.context);
                    return true;
                }
            }
        } else {
            // Keep running the current thread if it is still Running on this hart
            if let Some(ref mut curr) = self.threads[curr_idx] {
                if curr.state == ThreadState::Running(hart_id) {
                    return true;
                }
            }
        }
        false
    }
}

static SCHEDULER: Mutex<SchedulerState> = Mutex::new(SchedulerState::new());

/// Spawn a new thread with default kernel page table.
pub fn spawn_thread(entry: fn() -> !) -> Result<usize, &'static str> {
    let satp = crate::memory::KERNEL_PAGE_TABLE.lock().satp();
    spawn_thread_with_satp(entry, satp)
}

/// Spawn a new thread with a custom satp register value.
pub fn spawn_thread_with_satp(entry: fn() -> !, satp: usize) -> Result<usize, &'static str> {
    static NEXT_TID: AtomicUsize = AtomicUsize::new(1);
    let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);

    let thread = Thread::new(tid, satp, 0); // Default PID 0 for kernel threads
    SCHEDULER.lock().spawn(thread, entry)
}

fn user_mode_trampoline() -> ! {
    unsafe {
        SCHEDULER.force_unlock();
    }

    let (entry, user_sp) = {
        let sched = SCHEDULER.lock();
        let hart_id = get_hart_id();
        let current_idx = sched.current_idx[hart_id];
        let thread = sched.threads[current_idx].as_ref().unwrap();
        (thread.user_entry.unwrap_or(0), thread.user_sp.unwrap_or(0))
    };

    let kernel_sp: usize;
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) kernel_sp);
        core::arch::asm!("sfence.vma");
    }
    crate::println!(
        "[THREAD] Entering U-mode: entry=0x{:X} user_sp=0x{:X} kernel_sp=0x{:X}",
        entry, user_sp, kernel_sp
    );
    unsafe {
        crate::trap::enter_user_mode(entry, user_sp, kernel_sp);
    }
}

/// Spawn a kernel thread that transitions to user-mode at `entry_point`.
///
/// Used by `process::spawn()` to launch a new user process. The thread
/// runs a small kernel trampoline that:
/// 1. Releases the scheduler lock inherited from the spawner
/// 2. Flushes the TLB
/// 3. Calls `trap::enter_user_mode(entry, user_sp, kernel_sp)`
pub fn spawn_user_thread(entry_point: usize, stack_top: usize, satp: usize, pid: usize) -> Result<usize, &'static str> {
    static NEXT_TID: AtomicUsize = AtomicUsize::new(1);
    let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);

    let mut thread = Thread::new(tid, satp, pid);
    thread.user_entry = Some(entry_point);
    thread.user_sp = Some(stack_top);

    SCHEDULER.lock().spawn(thread, user_mode_trampoline)
}


/// Yield execution to the next thread.
pub fn schedule() {
    loop {
        let mut sched = SCHEDULER.lock();
        if sched.try_schedule() {
            break;
        }
        drop(sched);
        unsafe {
            // Enable supervisor interrupts (SIE bit in sstatus is bit 1)
            core::arch::asm!("csrs sstatus, {}", in(reg) 0x2usize);
            core::arch::asm!("wfi");
            // Disable supervisor interrupts so scheduling operations are atomic
            core::arch::asm!("csrc sstatus, {}", in(reg) 0x2usize);
        }
    }
}

/// Block the current thread and yield execution.
pub fn block_current_thread() {
    {
        let mut sched = SCHEDULER.lock();
        let hart_id = get_hart_id();
        let current_idx = sched.current_idx[hart_id];
        if let Some(ref mut curr) = sched.threads[current_idx] {
            curr.state = ThreadState::Blocked;
        }
    }
    schedule();
}

/// Wake up a thread by its ID, marking it ready.
pub fn wakeup_thread(tid: usize) {
    let mut sched = SCHEDULER.lock();
    for t in sched.threads.iter_mut().flatten() {
        if t.tid == tid && t.state == ThreadState::Blocked {
            t.state = ThreadState::Ready;
            break;
        }
    }
}

/// Get the TID of the currently running thread.
pub fn current_tid() -> usize {
    let sched = SCHEDULER.lock();
    let hart_id = get_hart_id();
    let current_idx = sched.current_idx[hart_id];
    if let Some(ref curr) = sched.threads[current_idx] {
        curr.tid
    } else {
        0
    }
}

/// Get the PID of the currently running thread's process.
pub fn current_pid() -> usize {
    let sched = SCHEDULER.lock();
    let hart_id = get_hart_id();
    let current_idx = sched.current_idx[hart_id];
    if let Some(ref curr) = sched.threads[current_idx] {
        curr.pid
    } else {
        0
    }
}

/// Execute a closure with a mutable reference to the currently running thread.
pub fn with_current_thread_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Thread) -> R,
{
    let mut sched = SCHEDULER.lock();
    let hart_id = get_hart_id();
    let current_idx = sched.current_idx[hart_id];
    sched.threads[current_idx].as_mut().map(f)
}

/// Check if a thread exists and if it has exited.
/// Returns (found, exited).
pub fn check_thread_status(tid: usize) -> (bool, bool) {
    let sched = SCHEDULER.lock();
    for t in sched.threads.iter().flatten() {
        if t.tid == tid {
            return (true, t.state == ThreadState::Exited);
        }
    }
    (false, false)
}

/// Initialize the scheduler with the main thread.
pub fn init() {
    let mut sched = SCHEDULER.lock();
    crate::println!("[DEBUG] SCHEDULER address: 0x{:X}", &raw const SCHEDULER as usize);
    crate::println!("[DEBUG] SCHEDULER.threads address: 0x{:X}", &raw const sched.threads as usize);
    crate::println!("[DEBUG] SCHEDULER.threads[0] address: 0x{:X}", &raw const sched.threads[0] as usize);
    crate::println!("[DEBUG] SCHEDULER.threads[1] address: 0x{:X}", &raw const sched.threads[1] as usize);
    // Register the current boot context as thread 0 (Running on hart 0).
    let boot_thread = Thread {
        tid: 0,
        pid: 1, // boot thread runs the root process (PID 1)
        state: ThreadState::Running(0),
        context: ThreadContext::default(),
        stack: None, // Dummy stack since we are already using the boot stack
        satp: crate::memory::KERNEL_PAGE_TABLE.lock().satp(),
        user_entry: None,
        user_sp: None,
        saved_user_context: None,
    };
    sched.threads[0] = Some(boot_thread);
    sched.current_idx[0] = 0;

    // Register dummy threads for secondary harts 1, 2, 3 in slots 1, 2, 3
    for hart_id in 1..4 {
        let dummy_thread = Thread {
            tid: 0x100 + hart_id,
            pid: 0,
            state: ThreadState::Running(hart_id),
            context: ThreadContext::default(),
            stack: None,
            satp: crate::memory::KERNEL_PAGE_TABLE.lock().satp(),
            user_entry: None,
            user_sp: None,
            saved_user_context: None,
        };
        sched.threads[hart_id] = Some(dummy_thread);
        sched.current_idx[hart_id] = hart_id;
    }
}

/// Forcefully release the scheduler lock.
/// This must be called at the start of every newly spawned thread.
///
/// # Safety
///
/// This is unsafe because it forcefully unlocks a spinlock, which should only be done
/// once when spawning a new thread context to release the lock held by the spawning thread.
pub unsafe fn release_lock() {
    unsafe {
        SCHEDULER.force_unlock();
    }
}

/// Terminate execution of the current thread and schedule the next one.
pub fn exit_current_thread() -> ! {
    {
        let mut sched = SCHEDULER.lock();
        let hart_id = get_hart_id();
        let current_idx = sched.current_idx[hart_id];
        if let Some(ref mut curr) = sched.threads[current_idx] {
            curr.state = ThreadState::Exited;
            crate::println!("[SCHED] Thread {} exited.", curr.tid);
        }
    }
    // Yield execution to the next ready thread
    schedule();
    // If there are no other threads, halt
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}
