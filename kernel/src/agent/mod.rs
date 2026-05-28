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
use crate::syscall::CURRENT_PROCESS;
use crate::capability::channel::{CHANNELS, allocate_channel};

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
    let pid = CURRENT_PROCESS.lock().as_ref().map(|p| p.pid).unwrap_or(0);
    
    if intent_ptr == 0 || intent_len == 0 {
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

    match allocate_channel() {
        Ok(ch_id) => {
            println!("[AGENT] Created channel {} owned by agent {}", ch_id, owner_agent_id);
            ch_id as isize
        }
        Err(_) => -12, // -ENOMEM
    }
}

/// SYS_CHANNEL_SEND = 72
pub fn sys_channel_send(channel_id: usize, payload_ptr: usize, payload_len: usize) -> isize {
    if payload_ptr == 0 || payload_len != MSG_SIZE {
        return -22; // -EINVAL
    }

    let mut buf = [0u8; MSG_SIZE];
    unsafe {
        core::ptr::copy_nonoverlapping(payload_ptr as *const u8, buf.as_mut_ptr(), MSG_SIZE);
    }

    let mut pool = CHANNELS.lock();
    if channel_id >= pool.len() {
        return -22; // -EINVAL
    }

    if let Some(ref mut ch) = pool[channel_id] {
        match ch.write(&buf, None) {
            Ok(_) => 0,
            Err(_) => -11, // -EAGAIN
        }
    } else {
        -9 // -EBADF
    }
}

/// SYS_CHANNEL_RECV = 73
pub fn sys_channel_recv(channel_id: usize, out_buf_ptr: usize, out_len_ptr: usize) -> isize {
    if out_buf_ptr == 0 || out_len_ptr == 0 {
        return -14; // -EFAULT
    }

    let mut pool = CHANNELS.lock();
    if channel_id >= pool.len() {
        return -22; // -EINVAL
    }

    if let Some(ref mut ch) = pool[channel_id] {
        match ch.read() {
            Ok(msg) => {
                unsafe {
                    core::ptr::copy_nonoverlapping(msg.data.as_ptr(), out_buf_ptr as *mut u8, MSG_SIZE);
                    *(out_len_ptr as *mut usize) = MSG_SIZE;
                }
                let sender_agent_id = u32::from_le_bytes([
                    msg.data[60],
                    msg.data[61],
                    msg.data[62],
                    msg.data[63],
                ]);
                sender_agent_id as isize
            }
            Err(_) => -11, // -EAGAIN (empty)
        }
    } else {
        -9 // -EBADF
    }
}

/// SYS_AGENT_STATUS = 74
pub fn sys_agent_status(agent_id: usize, out_state_ptr: usize) -> isize {
    if out_state_ptr == 0 {
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
