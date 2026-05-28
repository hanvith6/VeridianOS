//! Channel IPC Communication for VeridianOS
//!
//! A Channel is a bidirectional, message-oriented IPC mechanism.
//! It consists of two endpoints. Sending a message on one endpoint delivers it
//! to the other endpoint. Messages can contain:
//! - An array of raw data bytes
//! - A transferred capability Handle (transferring ownership between processes!)
//!
//! References:
//! - Zircon Channel IPC design (Fuchsia)
//! - seL4 Endpoint IPC transfers

use crate::capability::Handle;
use spin::Mutex;

/// The maximum byte length of a single channel message.
pub const MAX_MESSAGE_SIZE: usize = 512;

/// A single message queued inside a channel.
#[derive(Clone, Copy)]
pub struct Message {
    pub data: [u8; MAX_MESSAGE_SIZE],
    pub len: usize,
    pub transferred_handle: Option<Handle>,
}

impl Message {
    pub const fn empty() -> Self {
        Self {
            data: [0; MAX_MESSAGE_SIZE],
            len: 0,
            transferred_handle: None,
        }
    }
}

/// The internal state of a Channel.
pub struct Channel {
    buffer: [Message; 8],
    write_idx: usize,
    read_idx: usize,
    count: usize,
    pub blocked_tid: Option<usize>,
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel {
    pub const fn new() -> Self {
        Self {
            buffer: [Message::empty(); 8],
            write_idx: 0,
            read_idx: 0,
            count: 0,
            blocked_tid: None,
        }
    }

    /// Write a message into the channel queue.
    pub fn write(&mut self, data: &[u8], handle: Option<Handle>) -> Result<(), &'static str> {
        if self.count >= 8 {
            return Err("Channel buffer is full");
        }
        if data.len() > MAX_MESSAGE_SIZE {
            return Err("Message exceeds maximum size");
        }

        let slot = &mut self.buffer[self.write_idx];
        slot.data[..data.len()].copy_from_slice(data);
        slot.len = data.len();
        slot.transferred_handle = handle;

        self.write_idx = (self.write_idx + 1) % 8;
        self.count += 1;

        // Wake up a blocked reader thread if any
        if let Some(tid) = self.blocked_tid {
            crate::process::thread::wakeup_thread(tid);
            self.blocked_tid = None;
        }

        Ok(())
    }

    /// Read a message from the channel queue.
    pub fn read(&mut self) -> Result<Message, &'static str> {
        if self.count == 0 {
            return Err("Channel is empty");
        }

        let msg = self.buffer[self.read_idx];
        self.read_idx = (self.read_idx + 1) % 8;
        self.count -= 1;
        Ok(msg)
    }
}

/// A static pool of channels in kernel memory.
/// This avoids the need for dynamic kernel heap allocation.
pub static CHANNELS: Mutex<[Option<Channel>; 64]> = Mutex::new([const { None }; 64]);

/// Create a new channel in the global pool.
/// Returns the index of the allocated channel.
pub fn allocate_channel() -> Result<usize, &'static str> {
    let mut pool = CHANNELS.lock();
    for (idx, slot) in pool.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(Channel::new());
            return Ok(idx);
        }
    }
    Err("Global channel pool exhausted")
}
