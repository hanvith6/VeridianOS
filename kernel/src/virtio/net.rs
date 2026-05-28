//! VirtIO Network Device Driver for VeridianOS
//!
//! Implements a polling (non-interrupt-driven) VirtIO network device driver.
//! This driver can send and receive raw Ethernet frames via a QEMU-emulated
//! virtio-net device using the VirtQueue mechanism.
//!
//! # Architecture
//!
//! VirtIO net I/O uses two queues:
//!   Queue 0 — Receive (RX): Device writes packets here
//!   Queue 1 — Transmit (TX): Driver writes packets here
//!
//! Each TX submission is a 2-descriptor chain:
//! ```text
//! [Desc 0: VirtioNetHdr (10 bytes, WRITE by driver)] → NEXT
//! [Desc 1: Packet data  (≤1514 bytes, WRITE by driver)]
//! ```
//!
//! Each RX descriptor is pre-populated at init and re-filled after each
//! receive so the device always has buffers to deliver into.
//!
//! References:
//! - VirtIO Specification v1.2, Section 5.1 (Network Device)
//! - [OSDev VirtIO wiki](https://wiki.osdev.org/Virtio)

use super::{
    mmio_read, mmio_write, VirtqAvail, VirtqDesc, VirtqUsed, QUEUE_SIZE,
    VIRTIO_MAGIC,
    VIRTIO_MMIO_DEVICE_ID, VIRTIO_MMIO_DEVICE_FEATURES, VIRTIO_MMIO_DEVICE_FEATURES_SEL,
    VIRTIO_MMIO_DRIVER_FEATURES, VIRTIO_MMIO_DRIVER_FEATURES_SEL, VIRTIO_MMIO_MAGIC,
    VIRTIO_MMIO_QUEUE_DESC_HIGH, VIRTIO_MMIO_QUEUE_DESC_LOW,
    VIRTIO_MMIO_QUEUE_DEVICE_HIGH, VIRTIO_MMIO_QUEUE_DEVICE_LOW,
    VIRTIO_MMIO_QUEUE_DRIVER_HIGH, VIRTIO_MMIO_QUEUE_DRIVER_LOW,
    VIRTIO_MMIO_QUEUE_NUM, VIRTIO_MMIO_QUEUE_NUM_MAX, VIRTIO_MMIO_QUEUE_NOTIFY,
    VIRTIO_MMIO_QUEUE_READY, VIRTIO_MMIO_QUEUE_SEL, VIRTIO_MMIO_STATUS,
    VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER, VIRTIO_STATUS_DRIVER_OK,
    VIRTIO_STATUS_FEATURES_OK, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE,
    VIRTIO_MMIO_LEGACY_QUEUE_ALIGN, VIRTIO_MMIO_LEGACY_QUEUE_PFN,
    VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE,
};
use spin::Mutex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Base address of VirtIO slot 0 on the QEMU virt machine.
const VIRTIO_MMIO_SLOT_BASE: usize = 0x10001000;
/// Stride between consecutive VirtIO MMIO slots.
const VIRTIO_MMIO_SLOT_STRIDE: usize = 0x1000;
/// Total number of VirtIO MMIO slots on QEMU virt.
const VIRTIO_MMIO_SLOT_COUNT: usize = 8;

/// VirtIO Device ID for network devices (VirtIO Spec §5.1).
pub const VIRTIO_DEVICE_ID_NET: u32 = 1;

/// Maximum Ethernet frame payload (MTU without FCS).
pub const MAX_PACKET_SIZE: usize = 1514;

/// VirtIO net feature bits we negotiate (none — we want the minimal baseline).
/// VIRTIO_NET_F_MAC (bit 5) — device has a MAC address in config space.
/// We read it but do not require it, so we negotiate 0 optional features.
const NET_FEATURES_NONE: u32 = 0;

// ---------------------------------------------------------------------------
// VirtIO Net Header (VirtIO Spec §5.1.6)
// ---------------------------------------------------------------------------

/// Required header prepended to every transmitted or received packet.
///
/// For a basic driver (no GSO / checksum offload) all fields except
/// `num_buffers` remain zero.  The device fills `num_buffers` on receive;
/// we write all zeros on transmit.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VirtioNetHdr {
    flags: u8,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
    num_buffers: u16,
}

// ---------------------------------------------------------------------------
// Per-queue VirtQueue memory (page-aligned static buffers)
// ---------------------------------------------------------------------------
//
// VirtIO Legacy uses a single contiguous region per queue identified by its
// page-frame number.  We allocate an 8 KiB (2 × 4 KiB) buffer per queue:
//
//   [0    .. 128)  — descriptor table  (QUEUE_SIZE=8 × 16 bytes = 128 bytes)
//   [128  .. 4096) — available ring    (6 + QUEUE_SIZE×2 bytes, within page 0)
//   [4096 .. 8192) — used ring         (starts at page boundary, page 1)
//
// This matches the layout expected by VirtIO legacy PFN-based setup.

#[repr(C, align(4096))]
struct LegacyVirtQueue {
    data: core::cell::UnsafeCell<[u8; 8192]>,
}

unsafe impl Sync for LegacyVirtQueue {}

/// Page-aligned VirtQueue buffer for the RX queue (queue 0).
static VQ_BUF_RX: LegacyVirtQueue =
    LegacyVirtQueue { data: core::cell::UnsafeCell::new([0u8; 8192]) };

/// Page-aligned VirtQueue buffer for the TX queue (queue 1).
static VQ_BUF_TX: LegacyVirtQueue =
    LegacyVirtQueue { data: core::cell::UnsafeCell::new([0u8; 8192]) };

// Descriptor table is at offset 0; available ring at offset 128 (8×16);
// used ring at offset 4096 (next page boundary).
const DESC_OFFSET: usize = 0;
const AVAIL_OFFSET: usize = 128;
const USED_OFFSET: usize = 4096;

unsafe fn rx_desc_table() -> *mut VirtqDesc {
    (VQ_BUF_RX.data.get() as usize + DESC_OFFSET) as *mut VirtqDesc
}
unsafe fn rx_avail_ring() -> *mut VirtqAvail {
    (VQ_BUF_RX.data.get() as usize + AVAIL_OFFSET) as *mut VirtqAvail
}
unsafe fn rx_used_ring() -> *mut VirtqUsed {
    (VQ_BUF_RX.data.get() as usize + USED_OFFSET) as *mut VirtqUsed
}

unsafe fn tx_desc_table() -> *mut VirtqDesc {
    (VQ_BUF_TX.data.get() as usize + DESC_OFFSET) as *mut VirtqDesc
}
unsafe fn tx_avail_ring() -> *mut VirtqAvail {
    (VQ_BUF_TX.data.get() as usize + AVAIL_OFFSET) as *mut VirtqAvail
}
unsafe fn tx_used_ring() -> *mut VirtqUsed {
    (VQ_BUF_TX.data.get() as usize + USED_OFFSET) as *mut VirtqUsed
}

// ---------------------------------------------------------------------------
// Static packet data buffers (no heap)
// ---------------------------------------------------------------------------

/// Static RX packet data storage: one slot per queue descriptor.
/// Each slot holds the VirtioNetHdr + packet body (up to MAX_PACKET_SIZE).
#[repr(C, align(4096))]
struct RxDataBuf {
    data: core::cell::UnsafeCell<[[u8; MAX_PACKET_SIZE + core::mem::size_of::<VirtioNetHdr>()]; QUEUE_SIZE]>,
}

unsafe impl Sync for RxDataBuf {}

/// Static RX data buffers — one per descriptor slot.
static RX_DATA: RxDataBuf = RxDataBuf {
    data: core::cell::UnsafeCell::new(
        [[0u8; MAX_PACKET_SIZE + core::mem::size_of::<VirtioNetHdr>()]; QUEUE_SIZE],
    ),
};

/// Wrapper so we can implement `Sync` for the static TX net header.
struct TxNetHdrBuf {
    data: core::cell::UnsafeCell<VirtioNetHdr>,
}
unsafe impl Sync for TxNetHdrBuf {}

/// TX net header (we only ever have one in-flight TX at a time).
static TX_NET_HDR: TxNetHdrBuf = TxNetHdrBuf {
    data: core::cell::UnsafeCell::new(VirtioNetHdr {
        flags: 0,
        gso_type: 0,
        hdr_len: 0,
        gso_size: 0,
        csum_start: 0,
        csum_offset: 0,
        num_buffers: 0,
    }),
};

// ---------------------------------------------------------------------------
// Driver state
// ---------------------------------------------------------------------------

struct VirtioNetState {
    initialized: bool,
    mmio_base: usize,

    // TX queue bookkeeping
    tx_avail_idx: u16,
    tx_last_used_idx: u16,

    // RX queue bookkeeping
    rx_avail_idx: u16,
    rx_last_used_idx: u16,
}

impl VirtioNetState {
    const fn new() -> Self {
        Self {
            initialized: false,
            mmio_base: 0,
            tx_avail_idx: 0,
            tx_last_used_idx: 0,
            rx_avail_idx: 0,
            rx_last_used_idx: 0,
        }
    }
}

static VIRTIO_NET: Mutex<VirtioNetState> = Mutex::new(VirtioNetState::new());

// ---------------------------------------------------------------------------
// Public query
// ---------------------------------------------------------------------------

/// Returns true once `init()` has succeeded.
pub fn is_initialized() -> bool {
    VIRTIO_NET.lock().initialized
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the VirtIO network device.
///
/// Scans MMIO slots for device ID 1 (network), negotiates features, sets up
/// both RX and TX queues, pre-populates RX descriptors, and marks the device
/// as ready.
///
/// Returns `Ok(())` on success, `Err` if no device is found or init fails.
pub fn init() -> Result<(), &'static str> {
    let mut state = VIRTIO_NET.lock();

    // ── Step 1: Discover the network device ─────────────────────────────────
    let base = unsafe {
        let mut found = None;
        for slot in 0..VIRTIO_MMIO_SLOT_COUNT {
            let addr = VIRTIO_MMIO_SLOT_BASE + slot * VIRTIO_MMIO_SLOT_STRIDE;
            let magic = mmio_read(addr, VIRTIO_MMIO_MAGIC);
            let dev_id = mmio_read(addr, VIRTIO_MMIO_DEVICE_ID);
            if magic == VIRTIO_MAGIC && dev_id == VIRTIO_DEVICE_ID_NET {
                crate::println!(
                    "[VIRTIO-NET] Network device found at slot {} (0x{:X})",
                    slot, addr
                );
                found = Some(addr);
                break;
            }
        }
        match found {
            Some(addr) => addr,
            None => return Err("VirtIO: No network device found in any MMIO slot"),
        }
    };
    state.mmio_base = base;

    unsafe {
        let version = mmio_read(base, super::VIRTIO_MMIO_VERSION);
        crate::println!("[VIRTIO-NET] Device version: {}", version);

        // ── Step 2: Reset ─────────────────────────────────────────────────
        mmio_write(base, VIRTIO_MMIO_STATUS, 0);

        // ── Step 3: ACKNOWLEDGE ───────────────────────────────────────────
        mmio_write(base, VIRTIO_MMIO_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);

        // ── Step 4: DRIVER ────────────────────────────────────────────────
        mmio_write(base, VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

        if version == 1 {
            // ── VirtIO Legacy MMIO ─────────────────────────────────────────
            // Step 5: Feature negotiation (no SEL, no FEATURES_OK)
            let _dev_features = mmio_read(base, VIRTIO_MMIO_DEVICE_FEATURES);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES, NET_FEATURES_NONE);

            // Tell device the guest page size
            mmio_write(base, VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE, 4096);

            // ── Queue 0: RX ────────────────────────────────────────────────
            mmio_write(base, VIRTIO_MMIO_QUEUE_SEL, 0);
            let max_q = mmio_read(base, VIRTIO_MMIO_QUEUE_NUM_MAX);
            if max_q == 0 {
                return Err("VirtIO-net legacy: RX queue (0) not available");
            }
            let qnum = QUEUE_SIZE.min(max_q as usize) as u32;
            mmio_write(base, VIRTIO_MMIO_QUEUE_NUM, qnum);
            mmio_write(base, VIRTIO_MMIO_LEGACY_QUEUE_ALIGN, 4096);
            let rx_pfn = (VQ_BUF_RX.data.get() as u64 / 4096) as u32;
            mmio_write(base, VIRTIO_MMIO_LEGACY_QUEUE_PFN, rx_pfn);
            crate::println!("[VIRTIO-NET] RX queue legacy PFN: 0x{:X}", rx_pfn);

            // ── Queue 1: TX ────────────────────────────────────────────────
            mmio_write(base, VIRTIO_MMIO_QUEUE_SEL, 1);
            let max_q = mmio_read(base, VIRTIO_MMIO_QUEUE_NUM_MAX);
            if max_q == 0 {
                return Err("VirtIO-net legacy: TX queue (1) not available");
            }
            let qnum = QUEUE_SIZE.min(max_q as usize) as u32;
            mmio_write(base, VIRTIO_MMIO_QUEUE_NUM, qnum);
            mmio_write(base, VIRTIO_MMIO_LEGACY_QUEUE_ALIGN, 4096);
            let tx_pfn = (VQ_BUF_TX.data.get() as u64 / 4096) as u32;
            mmio_write(base, VIRTIO_MMIO_LEGACY_QUEUE_PFN, tx_pfn);
            crate::println!("[VIRTIO-NET] TX queue legacy PFN: 0x{:X}", tx_pfn);

        } else {
            // ── VirtIO Modern MMIO (version 2) ─────────────────────────────
            // Step 5: Feature negotiation with SEL registers
            mmio_write(base, VIRTIO_MMIO_DEVICE_FEATURES_SEL, 0);
            let _dev_features = mmio_read(base, VIRTIO_MMIO_DEVICE_FEATURES);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES_SEL, 0);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES, NET_FEATURES_NONE);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES_SEL, 1);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES, 0);

            // Step 6: Confirm feature negotiation
            mmio_write(base, VIRTIO_MMIO_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK);
            let st = mmio_read(base, VIRTIO_MMIO_STATUS);
            if st & VIRTIO_STATUS_FEATURES_OK == 0 {
                return Err("VirtIO-net modern: device rejected feature negotiation");
            }

            // ── Queue 0: RX ────────────────────────────────────────────────
            mmio_write(base, VIRTIO_MMIO_QUEUE_SEL, 0);
            let max_q = mmio_read(base, VIRTIO_MMIO_QUEUE_NUM_MAX);
            if max_q == 0 {
                return Err("VirtIO-net modern: RX queue (0) not available");
            }
            let qnum = QUEUE_SIZE.min(max_q as usize) as u32;
            mmio_write(base, VIRTIO_MMIO_QUEUE_NUM, qnum);
            let rx_desc = rx_desc_table() as u64;
            let rx_avail = rx_avail_ring() as u64;
            let rx_used = rx_used_ring() as u64;
            mmio_write(base, VIRTIO_MMIO_QUEUE_DESC_LOW,   rx_desc as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DESC_HIGH,  (rx_desc >> 32) as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DRIVER_LOW,  rx_avail as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DRIVER_HIGH, (rx_avail >> 32) as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DEVICE_LOW,  rx_used as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DEVICE_HIGH, (rx_used >> 32) as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_READY, 1);

            // ── Queue 1: TX ────────────────────────────────────────────────
            mmio_write(base, VIRTIO_MMIO_QUEUE_SEL, 1);
            let max_q = mmio_read(base, VIRTIO_MMIO_QUEUE_NUM_MAX);
            if max_q == 0 {
                return Err("VirtIO-net modern: TX queue (1) not available");
            }
            let qnum = QUEUE_SIZE.min(max_q as usize) as u32;
            mmio_write(base, VIRTIO_MMIO_QUEUE_NUM, qnum);
            let tx_desc = tx_desc_table() as u64;
            let tx_avail = tx_avail_ring() as u64;
            let tx_used = tx_used_ring() as u64;
            mmio_write(base, VIRTIO_MMIO_QUEUE_DESC_LOW,   tx_desc as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DESC_HIGH,  (tx_desc >> 32) as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DRIVER_LOW,  tx_avail as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DRIVER_HIGH, (tx_avail >> 32) as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DEVICE_LOW,  tx_used as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DEVICE_HIGH, (tx_used >> 32) as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_READY, 1);
        }

        // ── Step 8: Pre-populate RX descriptors ───────────────────────────
        //
        // For each descriptor slot in the RX queue we install a single
        // WRITE descriptor pointing at RX_DATA[i] (NetHdr + packet body)
        // and add it to the available ring so the device can deliver into it.
        let slot_size = (MAX_PACKET_SIZE + core::mem::size_of::<VirtioNetHdr>()) as u32;
        let rx_desc = rx_desc_table();
        let rx_avail = rx_avail_ring();
        let rx_data_base = (*RX_DATA.data.get()).as_ptr() as usize;

        for i in 0..QUEUE_SIZE {
            let buf_phys = (rx_data_base
                + i * (MAX_PACKET_SIZE + core::mem::size_of::<VirtioNetHdr>())) as u64;
            core::ptr::write_volatile(rx_desc.add(i), VirtqDesc {
                addr: buf_phys,
                len: slot_size,
                flags: VIRTQ_DESC_F_WRITE, // device writes into this buffer
                next: 0,
            });
            // Place descriptor index i into available ring slot i
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!((*rx_avail).ring[i]),
                i as u16,
            );
        }
        // Publish all QUEUE_SIZE descriptors at once
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        core::ptr::write_volatile(
            core::ptr::addr_of_mut!((*rx_avail).idx),
            QUEUE_SIZE as u16,
        );
        state.rx_avail_idx = QUEUE_SIZE as u16;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        // Notify device of the pre-filled RX queue
        mmio_write(base, VIRTIO_MMIO_QUEUE_NOTIFY, 0); // queue 0 = RX

        // ── Step 9: DRIVER_OK ─────────────────────────────────────────────
        mmio_write(base, VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK);

        state.initialized = true;
    }

    crate::println!("[VIRTIO-NET] Network device ready at MMIO 0x{:X}", base);
    Ok(())
}

// ---------------------------------------------------------------------------
// Transmit
// ---------------------------------------------------------------------------

/// Send a raw Ethernet frame (without FCS).
///
/// Builds a 2-descriptor chain in the TX queue:
///   Desc[0]: VirtioNetHdr (10 bytes) — driver writes, device reads
///   Desc[1]: `data`                  — driver writes, device reads
///
/// Polls the TX used ring until the device acknowledges completion.
/// Returns `Err` if `data` exceeds `MAX_PACKET_SIZE` or the driver is not
/// initialized.
pub fn send_packet(data: &[u8]) -> Result<(), &'static str> {
    if data.len() > MAX_PACKET_SIZE {
        return Err("VirtIO-net: packet exceeds MAX_PACKET_SIZE (1514)");
    }

    let mut state = VIRTIO_NET.lock();
    if !state.initialized {
        return Err("VirtIO-net: driver not initialized");
    }

    let base = state.mmio_base;

    unsafe {
        // Zero the net header (no GSO / checksum offload)
        core::ptr::write_volatile(
            TX_NET_HDR.data.get(),
            VirtioNetHdr::default(),
        );

        let hdr_phys = TX_NET_HDR.data.get() as u64;
        let pkt_phys = data.as_ptr() as u64;

        let desc = tx_desc_table();
        let avail = tx_avail_ring();
        let used = tx_used_ring();

        // We use two fixed descriptor indices (0 and 1) since we enforce
        // one in-flight TX at a time via the Mutex.

        // Descriptor 0: net header (device reads)
        core::ptr::write_volatile(desc.add(0), VirtqDesc {
            addr: hdr_phys,
            len: core::mem::size_of::<VirtioNetHdr>() as u32,
            flags: VIRTQ_DESC_F_NEXT, // no WRITE — driver → device
            next: 1,
        });

        // Descriptor 1: packet data (device reads)
        core::ptr::write_volatile(desc.add(1), VirtqDesc {
            addr: pkt_phys,
            len: data.len() as u32,
            flags: 0, // no WRITE, no NEXT — end of chain
            next: 0,
        });

        // Add chain head (descriptor 0) to the available ring
        let avail_slot = (state.tx_avail_idx as usize) % QUEUE_SIZE;
        core::ptr::write_volatile(
            core::ptr::addr_of_mut!((*avail).ring[avail_slot]),
            0u16, // chain head is always descriptor 0
        );
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let new_idx = core::ptr::read_volatile(core::ptr::addr_of!((*avail).idx))
            .wrapping_add(1);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*avail).idx), new_idx);
        state.tx_avail_idx = new_idx;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        // Notify device that TX queue (queue 1) has a new entry
        mmio_write(base, VIRTIO_MMIO_QUEUE_NOTIFY, 1);

        // Poll TX used ring for completion
        let target = state.tx_last_used_idx.wrapping_add(1);
        let mut spins = 0u32;
        loop {
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            let cur = core::ptr::read_volatile(core::ptr::addr_of!((*used).idx));
            if cur == target {
                break;
            }
            spins += 1;
            if spins % 10000 == 0 {
                crate::process::thread::schedule();
            }
            if spins > 50_000_000 {
                return Err("VirtIO-net: TX timed out");
            }
        }
        state.tx_last_used_idx = target;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Receive
// ---------------------------------------------------------------------------

/// Poll the RX queue for a completed packet.
///
/// If the device has delivered a packet, copies the payload (stripping the
/// `VirtioNetHdr`) into `buf` and returns `Some(len)` where `len` is the
/// number of bytes written.  Returns `None` if no packet is available.
///
/// After consuming a completed descriptor the function re-posts it to the
/// available ring so the device can refill the slot.
///
/// # Buffer
/// `buf` must be `[u8; 1514]`.  Only `len` bytes are valid on return.
pub fn try_recv_packet(buf: &mut [u8; MAX_PACKET_SIZE]) -> Option<usize> {
    let mut state = VIRTIO_NET.lock();
    if !state.initialized {
        return None;
    }

    let base = state.mmio_base;
    let hdr_size = core::mem::size_of::<VirtioNetHdr>();

    unsafe {
        let used = rx_used_ring();
        let avail = rx_avail_ring();

        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let used_idx = core::ptr::read_volatile(core::ptr::addr_of!((*used).idx));

        if used_idx == state.rx_last_used_idx {
            return None; // No new packet
        }

        // Consume the next used ring entry
        let used_slot = (state.rx_last_used_idx as usize) % QUEUE_SIZE;
        let used_elem = core::ptr::read_volatile(
            core::ptr::addr_of!((*used).ring[used_slot]),
        );
        state.rx_last_used_idx = state.rx_last_used_idx.wrapping_add(1);

        let desc_idx = used_elem.id as usize;
        let written = used_elem.len as usize; // bytes device wrote (NetHdr + payload)

        // Sanity check: must have at least the header and at most our slot size
        let slot_size = MAX_PACKET_SIZE + hdr_size;
        if written < hdr_size || written > slot_size {
            // Malformed; re-post the descriptor and return None
            repost_rx_desc(avail, &mut state.rx_avail_idx, desc_idx);
            mmio_write(base, VIRTIO_MMIO_QUEUE_NOTIFY, 0);
            return None;
        }

        // Compute the payload length (strip VirtioNetHdr)
        let pkt_len = written - hdr_size;

        // Copy packet bytes out of the static RX buffer
        let rx_data_base = (*RX_DATA.data.get()).as_ptr() as usize;
        let slot_base = rx_data_base + desc_idx * (MAX_PACKET_SIZE + hdr_size);
        let payload_ptr = (slot_base + hdr_size) as *const u8;
        let copy_len = pkt_len.min(MAX_PACKET_SIZE);
        core::ptr::copy_nonoverlapping(payload_ptr, buf.as_mut_ptr(), copy_len);

        // Re-post the descriptor so the device can reuse the slot
        repost_rx_desc(avail, &mut state.rx_avail_idx, desc_idx);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        // Notify device that a new RX descriptor is available
        mmio_write(base, VIRTIO_MMIO_QUEUE_NOTIFY, 0);

        Some(copy_len)
    }
}

/// Re-add a consumed RX descriptor index to the available ring.
///
/// # Safety
/// `avail` must be a valid pointer to the RX available ring.
unsafe fn repost_rx_desc(avail: *mut VirtqAvail, rx_avail_idx: &mut u16, desc_idx: usize) {
    unsafe {
        let avail_slot = (*rx_avail_idx as usize) % QUEUE_SIZE;
        core::ptr::write_volatile(
            core::ptr::addr_of_mut!((*avail).ring[avail_slot]),
            desc_idx as u16,
        );
        let new_idx = core::ptr::read_volatile(core::ptr::addr_of!((*avail).idx))
            .wrapping_add(1);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*avail).idx), new_idx);
        *rx_avail_idx = new_idx;
    }
}
