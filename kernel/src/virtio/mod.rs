//! VirtIO MMIO Transport Layer for VeridianOS
//!
//! Provides the low-level interface to VirtIO devices connected via MMIO
//! (Memory-Mapped I/O), as exposed by QEMU's `virt` machine.
//!
//! VirtIO is the de-facto standard paravirtualized I/O bus for virtual machines.
//! It allows a guest OS to communicate with emulated hardware (block devices,
//! network cards, etc.) through a well-defined ABI that avoids expensive
//! hardware emulation.
//!
//! References:
//! - [VirtIO Specification v1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/)
//! - QEMU virt machine memory map (hw/riscv/virt.c in QEMU source)

pub mod blk;
pub mod net;

// ---------------------------------------------------------------------------
// VirtIO MMIO Register Offsets (relative to device base address)
// VirtIO v1.2, Section 4.2.2
// ---------------------------------------------------------------------------

/// Magic value: must read 0x74726976 ("virt" in little-endian ASCII)
pub const VIRTIO_MMIO_MAGIC: usize = 0x000;
/// VirtIO device version (must be 2 for modern non-legacy interface)
pub const VIRTIO_MMIO_VERSION: usize = 0x004;
/// Device ID: identifies what type of device this is (1 = network, 2 = block, etc.)
pub const VIRTIO_MMIO_DEVICE_ID: usize = 0x008;
/// Vendor ID
pub const VIRTIO_MMIO_VENDOR_ID: usize = 0x00C;
/// Host (device) features bitmask — read-only
pub const VIRTIO_MMIO_DEVICE_FEATURES: usize = 0x010;
/// Selector for which 32-bit word of device features to read
pub const VIRTIO_MMIO_DEVICE_FEATURES_SEL: usize = 0x014;
/// Guest (driver) features bitmask — write-only
pub const VIRTIO_MMIO_DRIVER_FEATURES: usize = 0x020;
/// Selector for which 32-bit word of driver features to write
pub const VIRTIO_MMIO_DRIVER_FEATURES_SEL: usize = 0x024;
/// Legacy: guest page size in bytes — write-only
pub const VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE: usize = 0x028;
/// Index of the queue being accessed (write to select, then read/write other regs)
pub const VIRTIO_MMIO_QUEUE_SEL: usize = 0x030;
/// Maximum number of elements in the selected queue
pub const VIRTIO_MMIO_QUEUE_NUM_MAX: usize = 0x034;
/// Number of elements in the selected queue (write to configure)
pub const VIRTIO_MMIO_QUEUE_NUM: usize = 0x038;
/// Guest-physical page number of the descriptor table (lower 32 bits)
pub const VIRTIO_MMIO_QUEUE_DESC_LOW: usize = 0x080;
/// Guest-physical page number of the descriptor table (upper 32 bits)
pub const VIRTIO_MMIO_QUEUE_DESC_HIGH: usize = 0x084;
/// Guest-physical page number of the driver (available) ring (lower 32 bits)
pub const VIRTIO_MMIO_QUEUE_DRIVER_LOW: usize = 0x090;
/// Guest-physical page number of the driver (available) ring (upper 32 bits)
pub const VIRTIO_MMIO_QUEUE_DRIVER_HIGH: usize = 0x094;
/// Guest-physical page number of the device (used) ring (lower 32 bits)
pub const VIRTIO_MMIO_QUEUE_DEVICE_LOW: usize = 0x0A0;
/// Guest-physical page number of the device (used) ring (upper 32 bits)
pub const VIRTIO_MMIO_QUEUE_DEVICE_HIGH: usize = 0x0A4;
/// Write any value to notify the device that queue index N has new requests
pub const VIRTIO_MMIO_QUEUE_NOTIFY: usize = 0x050;
/// Write 1 to mark the selected queue as ready
pub const VIRTIO_MMIO_QUEUE_READY: usize = 0x044;

// Legacy (version 1) queue setup registers
/// Legacy: queue alignment in bytes (write 4096 for 4KB alignment)
pub const VIRTIO_MMIO_LEGACY_QUEUE_ALIGN: usize = 0x03C;
/// Legacy: page frame number of the queue (physical address / 4096)
pub const VIRTIO_MMIO_LEGACY_QUEUE_PFN: usize = 0x040;

/// Device status register — driver writes bits to progress through initialization
pub const VIRTIO_MMIO_STATUS: usize = 0x070;
/// Device interrupt status (read-only)
pub const VIRTIO_MMIO_INTERRUPT_STATUS: usize = 0x060;
/// Write to acknowledge and clear interrupts
pub const VIRTIO_MMIO_INTERRUPT_ACK: usize = 0x064;
/// Device-specific configuration space starts here
pub const VIRTIO_MMIO_CONFIG: usize = 0x100;

// ---------------------------------------------------------------------------
// VirtIO Device Status Bits (VirtIO Spec §2.1)
// ---------------------------------------------------------------------------
/// OS has acknowledged the device
pub const VIRTIO_STATUS_ACKNOWLEDGE: u32 = 1;
/// OS knows how to drive this device
pub const VIRTIO_STATUS_DRIVER: u32 = 2;
/// Driver is set up and ready to drive the device
pub const VIRTIO_STATUS_DRIVER_OK: u32 = 4;
/// Driver has acknowledged all the features it understands
pub const VIRTIO_STATUS_FEATURES_OK: u32 = 8;
/// Something went wrong in the guest (driver bug or misconfiguration)
pub const VIRTIO_STATUS_FAILED: u32 = 128;

/// The MMIO base address of the first VirtIO device on QEMU's `virt` machine.
/// QEMU places virtio-blk at 0x10001000 by default (second virtio slot).
pub const VIRTIO_BLK_MMIO_BASE: usize = 0x10001000;

/// Expected magic number for a valid VirtIO MMIO device.
pub const VIRTIO_MAGIC: u32 = 0x7472_6976; // "virt"

/// VirtIO Device ID for block devices.
pub const VIRTIO_DEVICE_ID_BLOCK: u32 = 2;

// ---------------------------------------------------------------------------
// VirtQueue Descriptor Flags (VirtIO Spec §2.7.5)
// ---------------------------------------------------------------------------
/// This descriptor's buffer continues in the next descriptor (chaining)
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Device writes to this buffer (not driver)
pub const VIRTQ_DESC_F_WRITE: u16 = 2;

// ---------------------------------------------------------------------------
// VirtQueue Structures (VirtIO Spec §2.7)
// ---------------------------------------------------------------------------

/// Number of descriptors in the VirtQueue ring. Must be a power of two.
pub const QUEUE_SIZE: usize = 8;

/// A VirtQueue Descriptor Table entry.
///
/// Each descriptor points to a physical buffer of data, with a length,
/// flags, and an optional chain to the next descriptor.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct VirtqDesc {
    /// Guest-physical address of the buffer
    pub addr: u64,
    /// Buffer length in bytes
    pub len: u32,
    /// Descriptor flags (NEXT, WRITE, INDIRECT)
    pub flags: u16,
    /// Index of the next descriptor in the chain (if NEXT flag is set)
    pub next: u16,
}

/// The VirtQueue Available Ring (driver → device).
///
/// The driver writes descriptor chain head indices here to submit requests.
#[repr(C)]
pub struct VirtqAvail {
    pub flags: u16,
    /// Index into the ring where the next available descriptor will be written
    pub idx: u16,
    /// Ring of descriptor chain head indices
    pub ring: [u16; QUEUE_SIZE],
    pub used_event: u16,
}

/// One entry in the Used Ring — the device's response.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct VirtqUsedElem {
    /// Index of the start of the descriptor chain that was processed
    pub id: u32,
    /// Total bytes written into the device-writable buffers
    pub len: u32,
}

/// The VirtQueue Used Ring (device → driver).
///
/// The device writes completed requests here.
#[repr(C)]
pub struct VirtqUsed {
    pub flags: u16,
    /// Index into the ring where the next used entry will be written
    pub idx: u16,
    pub ring: [VirtqUsedElem; QUEUE_SIZE],
    pub avail_event: u16,
}

// ---------------------------------------------------------------------------
// Low-level MMIO read/write helpers
// ---------------------------------------------------------------------------

/// Read a 32-bit value from a VirtIO MMIO register.
///
/// # Safety
/// The base address must be a valid VirtIO MMIO device region.
#[inline]
pub unsafe fn mmio_read(base: usize, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}

/// Write a 32-bit value to a VirtIO MMIO register.
///
/// # Safety
/// The base address must be a valid VirtIO MMIO device region.
#[inline]
pub unsafe fn mmio_write(base: usize, offset: usize, val: u32) {
    unsafe { core::ptr::write_volatile((base + offset) as *mut u32, val) }
}
