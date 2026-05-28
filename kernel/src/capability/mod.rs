//! Capability Security Model for VeridianOS
//!
//! Unlike Unix, which relies on ambient authority and access lists (UID/GID),
//! VeridianOS uses **Capabilities**.
//!
//! A capability is a token of authority. If a process holds a capability to a resource
//! (a memory block, a device driver, or a communication channel), it can access it.
//! If it doesn't hold the capability, access is physically impossible.
//!
//! Features:
//! - **Kernel Objects**: Abstract traits representing resources (VMOs, Channels, Processes).
//! - **Handles**: Secure, unforgeable integers indexable into a process's private Handle Table.
//! - **Rights**: Granular permission bitflags associated with each handle.
//!
//! References:
//! - Zircon Microkernel Concepts (Fuchsia)
//! - seL4 Microkernel Reference Manual
//! - LithOS capability enclaves (SOSP '25)

pub mod channel;
pub mod rights;

pub use rights::Rights;

/// The maximum number of handles a single process can hold at once.
pub const MAX_HANDLES: usize = 64;


/// Types of kernel objects that can be governed by capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    None,
    Process,
    Thread,
    Channel,
    VirtualMemoryObject,
    TaskGraph,           // Representing a DAG computation
    DeviceQueue,         // Representing a heterogeneous device command ring buffer
    GraphNode,           // Representing a semantic graph node (Phase 8)
    AgentProcess,        // Representing an AI agent process (Phase 9)
    AgentChannel,        // Representing an IPC channel between agents (Phase 9)
}

/// A representation of a Capability Handle.
///
/// Under the hood, this is a secure reference containing:
/// - A pointer to the physical kernel object in memory
/// - The type of the object
/// - The permissions (rights) granted through this specific handle
#[derive(Debug, Clone, Copy)]
pub struct Handle {
    pub object_type: ObjectType,
    pub object_ptr: usize,
    pub rights: Rights,
}

impl Handle {
    /// Create a new capability handle.
    pub const fn new(object_type: ObjectType, object_ptr: usize, rights: Rights) -> Self {
        Self {
            object_type,
            object_ptr,
            rights,
        }
    }
}

/// A Process-Local Handle Table.
///
/// Each process has a private handle table. The indices of this table are the actual
/// "handle numbers" (integers) returned to user-space.
pub struct HandleTable {
    slots: [Option<Handle>; MAX_HANDLES],
}

impl Default for HandleTable {
    fn default() -> Self {
        Self::new()
    }
}

impl HandleTable {
    /// Create a new, empty handle table.
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_HANDLES],
        }
    }

    /// Set a handle at a specific slot in the table.
    pub fn set(&mut self, handle_id: usize, handle: Handle) -> Result<(), &'static str> {
        if handle_id >= MAX_HANDLES {
            return Err("Invalid handle ID");
        }
        self.slots[handle_id] = Some(handle);
        Ok(())
    }

    /// Insert a handle into the table, returning the handle ID (index).
    /// Returns `Err` if the table is full.
    pub fn insert(&mut self, handle: Handle) -> Result<usize, &'static str> {
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(handle);
                return Ok(idx);
            }
        }
        Err("Handle table is full")
    }

    /// Retrieve a handle by its ID.
    pub fn get(&self, handle_id: usize) -> Result<Handle, &'static str> {
        if handle_id >= MAX_HANDLES {
            return Err("Invalid handle ID");
        }
        self.slots[handle_id].ok_or("No handle at requested slot")
    }

    /// Retrieve a mutable reference to a handle slot.
    pub fn get_mut(&mut self, handle_id: usize) -> Result<&mut Handle, &'static str> {
        if handle_id >= MAX_HANDLES {
            return Err("Invalid handle ID");
        }
        self.slots[handle_id]
            .as_mut()
            .ok_or("No handle at requested slot")
    }

    /// Remove a handle from the table (closing it).
    pub fn remove(&mut self, handle_id: usize) -> Result<Handle, &'static str> {
        if handle_id >= MAX_HANDLES {
            return Err("Invalid handle ID");
        }
        self.slots[handle_id]
            .take()
            .ok_or("No handle at requested slot")
    }
}
