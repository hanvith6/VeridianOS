//! Panic Handler for VeridianOS
//!
//! Because our kernel runs on bare metal without the standard library (`no_std`),
//! we must define our own handler for when the code crashes (panics).
//! This code prints the crash information to the screen and halts the computer.

use crate::println;
use core::panic::PanicInfo;

/// The panic handler entry point.
///
/// When the Rust compiler detects a panic, it redirects execution here.
/// We print the error message, the file name, and the line number where it happened,
/// and then halt the CPU core.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n==================================================");
    println!("🚨 KERNEL PANIC DETECTED 🚨");
    println!("==================================================");

    if let Some(location) = info.location() {
        println!(
            "Location: file '{}' at line {}",
            location.file(),
            location.line()
        );
    } else {
        println!("Location: unknown");
    }

    // Print the panic message itself. In modern nightly Rust, PanicInfo::message()
    // returns a structure that implements Display directly.
    println!("Message:  {}", info.message());

    println!("==================================================");
    println!("System halted. Reboot required.");

    // Loop forever to halt the CPU core.
    loop {
        // Use the assembly "wfi" (Wait For Interrupt) instruction to put the CPU into a low-power state.
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}
