//! VeridianOS Phase 9 — Agent Runtime
//!
//! Agents are first-class kernel citizens: isolated processes with:
//! - A unique AgentId (u32, monotonically allocated)
//! - An intent string (32-byte fixed-size label)
//! - A parent AgentId (0 = root/kernel)
//! - State: Idle | Running | WaitingForMessage | Dead
//!
//! IPC: Agents communicate via capability-secured Channels.
//!
//! References:
//! - AIOS: LLM Agent Operating System (Mei et al., 2024)

use spin::Mutex;
use crate::println;
use crate::capability::channel::{CHANNELS, allocate_channel};
use crate::capability::{Handle, ObjectType, Rights};

pub type AgentId = u32;
pub const AGENT_ID_NULL: AgentId = 0;
pub const MAX_AGENTS: usize = 16;
pub const MSG_SIZE: usize = 64;
pub const MAX_INTENT_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentState {
    Idle = 0,
    Running = 1,
    WaitingForMessage = 2,
    Dead = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct AgentRecord {
    pub id: AgentId,
    pub parent_id: AgentId,
    pub state: AgentState,
    pub intent: [u8; MAX_INTENT_LEN],
    pub pid: usize,
    pub valid: bool,
    /// Phase 12: enclave_id is `Some(id)` when this agent runs inside a
    /// hardware-isolated TEE enclave managed by the M-mode monitor.
    /// `None` means the agent runs in ordinary S-mode kernel-managed memory.
    pub enclave_id: Option<u8>,
}

pub struct AgentPool {
    records: [AgentRecord; MAX_AGENTS],
    next_id: u32,
}

impl AgentPool {
    const fn new() -> Self {
        let empty_record = AgentRecord {
            id: 0,
            parent_id: 0,
            state: AgentState::Idle,
            intent: [0; MAX_INTENT_LEN],
            pid: 0,
            valid: false,
            enclave_id: None,
        };
        Self {
            records: [
                empty_record, empty_record, empty_record, empty_record,
                empty_record, empty_record, empty_record, empty_record,
                empty_record, empty_record, empty_record, empty_record,
                empty_record, empty_record, empty_record, empty_record,
            ],
            next_id: 1,
        }
    }

    pub fn alloc(&mut self, parent_id: AgentId, intent: &[u8], pid: usize) -> Result<AgentId, &'static str> {
        for record in self.records.iter_mut() {
            if !record.valid {
                let id = self.next_id;
                self.next_id += 1;
                record.id = id;
                record.parent_id = parent_id;
                record.state = AgentState::Running;
                let copy_len = core::cmp::min(intent.len(), MAX_INTENT_LEN);
                record.intent[..copy_len].copy_from_slice(&intent[..copy_len]);
                record.pid = pid;
                record.valid = true;
                return Ok(id);
            }
        }
        Err("Agent pool exhausted")
    }

    pub fn get(&self, id: AgentId) -> Option<&AgentRecord> {
        self.records.iter().find(|record| record.valid && record.id == id)
    }

    pub fn get_mut(&mut self, id: AgentId) -> Option<&mut AgentRecord> {
        self.records.iter_mut().find(|record| record.valid && record.id == id)
    }
}

pub static AGENT_TABLE: Mutex<AgentPool> = Mutex::new(AgentPool::new());

pub fn init() {
    println!("[AGENT] Agent Runtime initialized. Max agents: 16, Max channels: 16");
    println!("[AGENT] Syscalls registered: SYS_AGENT_SPAWN(70)..SYS_AGENT_STATUS(74)");
}

/// SYS_AGENT_SPAWN = 70
pub fn sys_agent_spawn(parent_id: usize, intent_ptr: usize, intent_len: usize) -> isize {
    let pid = crate::process::thread::current_pid();
    
    if intent_ptr == 0 || intent_len == 0 {
        return -14; // -EFAULT
    }

    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(intent_ptr, intent_len, false)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }
    
    let copy_len = core::cmp::min(intent_len, MAX_INTENT_LEN);
    let mut intent_buf = [0u8; MAX_INTENT_LEN];
    unsafe {
        let src = intent_ptr as *const u8;
        core::ptr::copy_nonoverlapping(src, intent_buf.as_mut_ptr(), copy_len);
    }

    let mut table = AGENT_TABLE.lock();
    match table.alloc(parent_id as u32, &intent_buf[..copy_len], pid) {
        Ok(id) => {
            println!("[AGENT] Spawned agent {} with parent {}", id, parent_id);
            id as isize
        }
        Err(_) => -12, // -ENOMEM
    }
}

/// SYS_CHANNEL_CREATE = 71
pub fn sys_channel_create(owner_agent_id: usize) -> isize {
    let table = AGENT_TABLE.lock();
    if owner_agent_id != 0 && table.get(owner_agent_id as u32).is_none() {
        return -22; // -EINVAL
    }
    drop(table);

    let ch_id = match allocate_channel() {
        Ok(ch_id) => ch_id,
        Err(_) => return -12, // -ENOMEM
    };

    println!("[AGENT] Created channel {} owned by agent {}", ch_id, owner_agent_id);

    let handle = Handle::new(
        ObjectType::Channel,
        ch_id,
        Rights::READ | Rights::WRITE | Rights::DUPLICATE,
    );

    let res = crate::process::with_current_process(|proc| {
        proc.handle_table.insert(handle)
    });

    match res {
        Some(Ok(hid)) => hid as isize,
        Some(Err(_)) => -12, // -ENOMEM
        None => -3, // -EPERM
    }
}

/// SYS_CHANNEL_SEND = 72
pub fn sys_channel_send(channel_handle_id: usize, payload_ptr: usize, payload_len: usize) -> isize {
    if payload_ptr == 0 || payload_len > 512 {
        return -22; // -EINVAL
    }

    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(payload_ptr, payload_len, false)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

    let mut buf = [0u8; 512];
    unsafe {
        core::ptr::copy_nonoverlapping(payload_ptr as *const u8, buf.as_mut_ptr(), payload_len);
    }

    let res = crate::process::with_current_process(|proc| {
        let handle = match proc.handle_table.get(channel_handle_id) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF
        };
        if handle.object_type != ObjectType::Channel {
            return Err(-9); // -EBADF
        }
        if !handle.rights.contains(Rights::WRITE) {
            return Err(-13); // -EACCES
        }
        Ok(handle.object_ptr)
    });

    let channel_id = match res {
        Some(Ok(cid)) => cid,
        Some(Err(err)) => return err,
        None => return -3, // -EPERM
    };

    let mut pool = CHANNELS.lock();
    if channel_id >= pool.len() {
        return -22; // -EINVAL
    }

    if let Some(ref mut ch) = pool[channel_id] {
        match ch.write(&buf[..payload_len], None) {
            Ok(_) => 0,
            Err(_) => -11, // -EAGAIN
        }
    } else {
        -9 // -EBADF
    }
}

/// SYS_CHANNEL_RECV = 73
pub fn sys_channel_recv(channel_handle_id: usize, out_buf_ptr: usize, out_len_ptr: usize) -> isize {
    if out_buf_ptr == 0 || out_len_ptr == 0 {
        return -14; // -EFAULT
    }

    let len_ptr_valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(out_len_ptr, core::mem::size_of::<usize>(), true)
    }).unwrap_or(false);

    if !len_ptr_valid {
        return -14; // -EFAULT
    }

    let res = crate::process::with_current_process(|proc| {
        let handle = match proc.handle_table.get(channel_handle_id) {
            Ok(h) => h,
            Err(_) => return Err(-9), // -EBADF
        };
        if handle.object_type != ObjectType::Channel {
            return Err(-9); // -EBADF
        }
        if !handle.rights.contains(Rights::READ) {
            return Err(-13); // -EACCES
        }
        Ok(handle.object_ptr)
    });

    let channel_id = match res {
        Some(Ok(cid)) => cid,
        Some(Err(err)) => return err,
        None => return -3, // -EPERM
    };

    let mut sie = 0;
    loop {
        let mut pool = CHANNELS.lock();
        if channel_id >= pool.len() {
            if sie != 0 { unsafe { core::arch::asm!("csrs sstatus, {}", in(reg) 2usize); } }
            return -22; // -EINVAL
        }

        if let Some(ref mut ch) = pool[channel_id] {
            match ch.read() {
                Ok(msg) => {
                    let buf_valid = crate::process::with_current_process(|proc| {
                        proc.validate_user_buffer(out_buf_ptr, msg.len, true)
                    }).unwrap_or(false);

                    if !buf_valid {
                        if sie != 0 { unsafe { core::arch::asm!("csrs sstatus, {}", in(reg) 2usize); } }
                        return -14; // -EFAULT
                    }

                    if sie != 0 {
                        unsafe { core::arch::asm!("csrs sstatus, {}", in(reg) 2usize); }
                    }

                    unsafe {
                        core::ptr::copy_nonoverlapping(msg.data.as_ptr(), out_buf_ptr as *mut u8, msg.len);
                        *(out_len_ptr as *mut usize) = msg.len;
                    }
                    let sender_agent_id = if msg.len >= 4 {
                        u32::from_le_bytes([
                            msg.data[msg.len - 4],
                            msg.data[msg.len - 3],
                            msg.data[msg.len - 2],
                            msg.data[msg.len - 1],
                        ])
                    } else {
                        0
                    };
                    return sender_agent_id as isize;
                }
                Err(_) => {
                    let tid = crate::process::thread::current_tid();
                    ch.blocked_tid = Some(tid);
                    
                    sie = unsafe {
                        let mut sstatus: usize;
                        core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
                        core::arch::asm!("csrc sstatus, {}", in(reg) 2usize);
                        sstatus & 2
                    };
                    
                    drop(pool);

                    crate::process::thread::block_current_thread();
                }
            }
        } else {
            if sie != 0 { unsafe { core::arch::asm!("csrs sstatus, {}", in(reg) 2usize); } }
            return -9; // -EBADF
        }
    }
}

/// SYS_AGENT_STATUS = 74
pub fn sys_agent_status(agent_id: usize, out_state_ptr: usize) -> isize {
    if out_state_ptr == 0 {
        return -14; // -EFAULT
    }

    let valid = crate::process::with_current_process(|proc| {
        proc.validate_user_buffer(out_state_ptr, 1, true)
    }).unwrap_or(false);

    if !valid {
        return -14; // -EFAULT
    }

    let table = AGENT_TABLE.lock();
    if let Some(record) = table.get(agent_id as u32) {
        let state_val = record.state as u8;
        unsafe {
            *(out_state_ptr as *mut u8) = state_val;
        }
        0
    } else {
        -22 // -EINVAL
    }
}
