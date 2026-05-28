//! Capability Rights definitions for VeridianOS
//!
//! A capability handle doesn't just reference a kernel object; it is also associated
//! with a set of `Rights` that control which operations can be performed on the object
//! via that specific handle.
//!
//! References:
//! - Google Fuchsia Zircon Capability Rights Model
//! - seL4 microkernel Access Control Lists

use bitflags::bitflags;

bitflags! {
    /// Rights associated with a capability handle.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Rights: u32 {
        /// Allows reading the object's contents or state.
        const READ = 1 << 0;

        /// Allows modifying the object's contents or state.
        const WRITE = 1 << 1;

        /// Allows executing memory mapped from the object (e.g. for VMOs).
        const EXECUTE = 1 << 2;

        /// Allows duplicating the handle to create a new one (optionally reducing rights).
        const DUPLICATE = 1 << 3;

        /// Allows transferring the handle to another process over a communication channel.
        const TRANSFER = 1 << 4;

        /// Full/Administrative rights.
        const DEFAULT = Self::READ.bits() | Self::WRITE.bits() | Self::EXECUTE.bits() | Self::DUPLICATE.bits() | Self::TRANSFER.bits();
    }
}
