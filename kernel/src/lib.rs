//! VeridianOS Kernel — library root for `cargo test --lib`
//!
//! This file exists solely to enable host-side unit tests for modules whose
//! logic is pure enough to run outside the RISC-V boot environment.
//!
//! The normal kernel build uses `main.rs` as the crate root (binary target).
//! This file is the *library* target declared in `Cargo.toml`; it is compiled
//! only when running `cargo test --lib` (or any build that links the lib).
//!
//! # no_std handling
//!
//! The kernel binary sets `#![no_std]` and `#![no_main]` in `main.rs`.  When
//! Cargo builds the *library* target for testing it does not inherit those
//! attributes from `main.rs`, so the standard library and the test harness are
//! available automatically.
//!
//! Any module re-exported here must not depend on kernel-only globals (UART,
//! ALLOCATOR spinlock initialised by `kinit`, etc.) inside `#[cfg(test)]`
//! blocks.  The `page_alloc` module satisfies this: its `PageAllocatorState`
//! is a plain struct that can be initialised with an arbitrary memory slice.

// When building for the kernel binary target the compiler sees main.rs and
// never compiles lib.rs. When building for tests, std is available and we do
// NOT want to set no_std here.
#![cfg_attr(not(test), no_std)]

pub mod memory {
    pub mod page_alloc;
}
