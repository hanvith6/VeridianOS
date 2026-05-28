//! UART 16550 Serial Driver for VeridianOS
//!
//! On the QEMU 'virt' RISC-V platform, a serial port is simulated at physical address 0x1000_0000.
//! This driver uses Memory-Mapped I/O (MMIO), meaning we read and write to this memory address
//! as if it were RAM, but the hardware translates those writes into characters sent to our terminal.
//!
//! References:
//! - UART 16550 Specification
//! - RISC-V QEMU 'virt' board documentation

use core::fmt::{self, Write};
use spin::Mutex;

/// The physical base address of the UART registers on QEMU's RISC-V 'virt' board.
const UART0_BASE: usize = 0x1000_0000;

/// A simple structure representing our UART device.
pub struct Uart {
    base_address: usize,
}

impl Uart {
    /// Create a new instance of the UART driver pointing to a specific MMIO address.
    pub const fn new(base_address: usize) -> Self {
        Self { base_address }
    }

    /// Initialize the UART device.
    ///
    /// We configure:
    /// - Word length to 8 bits (LCR = 3)
    /// - Enable FIFO buffer (FCR = 1)
    /// - Enable receiver buffer interrupts (IER = 1)
    pub fn init(&self) {
        let ptr = self.base_address as *mut u8;
        unsafe {
            // 1. Disable all interrupts while configuring
            ptr.add(1).write_volatile(0x00);

            // 2. Set Baud Rate: Enable DLAB (Divisor Latch Access Bit)
            // This allows us to set the baud rate divisor.
            let lcr = ptr.add(3);
            lcr.write_volatile(0x80); // Set DLAB to 1

            // Set divisor to 3 (which gives 38.4K baud rate on a 1.8432 MHz clock)
            ptr.add(0).write_volatile(0x03); // Divisor Latch Low
            ptr.add(1).write_volatile(0x00); // Divisor Latch High

            // 3. Set word length to 8 bits, no parity, 1 stop bit
            // This also disables DLAB (writing 0x03) so registers 0 and 1 go back to normal mode.
            lcr.write_volatile(0x03);

            // 4. Enable FIFOs, clear TX and RX FIFOs
            ptr.add(2).write_volatile(0x07);

            // 5. Enable Receiver Buffer Interrupt (allows reading keys later)
            ptr.add(1).write_volatile(0x01);
        }
    }

    /// Send a single byte (character) over the serial port.
    pub fn putc(&self, c: u8) {
        let ptr = self.base_address as *mut u8;
        unsafe {
            // Line Status Register (LSR) is at offset 5.
            // Bit 5 (0x20) is the Transmit Holding Register Empty (THRE) flag.
            // We loop until the transmit buffer is empty and ready to accept a new character.
            while (ptr.add(5).read_volatile() & 0x20) == 0 {
                // Spin/wait
            }
            // Write the character to the Transmitter Holding Register (THR) at offset 0.
            ptr.add(0).write_volatile(c);
        }
    }

    /// Read a single byte from the serial port, blocking until one is available.
    pub fn getc(&self) -> u8 {
        let ptr = self.base_address as *mut u8;
        unsafe {
            // Line Status Register (LSR) offset 5.
            // Bit 0 (0x01) is the Data Ready (DR) flag.
            // We loop until data is ready to be read.
            while (ptr.add(5).read_volatile() & 0x01) == 0 {
                // Spin/wait
            }
            // Read from Receiver Buffer Register (RBR) at offset 0.
            ptr.add(0).read_volatile()
        }
    }
}

/// Implement the standard Rust `Write` trait so we can use formatting macros like `write!` and `print!`.
impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.putc(byte);
        }
        Ok(())
    }
}

/// A globally accessible, thread-safe instance of our UART serial port.
/// We use a Spinlock (`spin::Mutex`) to ensure that multiple cores or threads don't scramble print messages.
pub static WRITER: Mutex<Uart> = Mutex::new(Uart::new(UART0_BASE));

/// Standard print! macro for the kernel.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        {
            let mut writer = $crate::uart::WRITER.lock();
            use core::fmt::Write;
            let _ = write!(writer, $($arg)*);
        }
    };
}

/// Standard println! macro for the kernel.
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::print!("{}\n", format_args!($($arg)*))
    };
}
