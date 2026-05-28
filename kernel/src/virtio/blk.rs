//! VirtIO Block Device Driver for VeridianOS
//!
//! Implements a polling (non-interrupt-driven) VirtIO block device driver.
//! This driver can read 512-byte sectors from a QEMU-emulated virtio-blk
//! disk image using the VirtQueue mechanism.
//!
//! # Architecture
//!
//! VirtIO block I/O uses a 3-descriptor chain per request:
//! ```text
//! [Desc 0: BlkReq header (type=READ, sector=N)] → NEXT
//! [Desc 1: Data buffer (512 bytes)]             → NEXT | WRITE
//! [Desc 2: Status byte]                         → WRITE
//! ```
//! The driver writes this chain to the Available ring, notifies the device,
//! then polls the Used ring until the device has processed the request.
//!
//! References:
//! - VirtIO Specification v1.2, Section 5.2 (Block Device)
//! - [OSDev VirtIO wiki](https://wiki.osdev.org/Virtio)

use super::{
    mmio_read, mmio_write, VirtqAvail, VirtqDesc, VirtqUsed, QUEUE_SIZE,
    VIRTIO_DEVICE_ID_BLOCK, VIRTIO_MAGIC,
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

/// Base address of VirtIO slot 0. QEMU virt maps 8 slots starting here.
const VIRTIO_MMIO_SLOT_BASE: usize = 0x10001000;
/// Stride between consecutive VirtIO MMIO slots.
const VIRTIO_MMIO_SLOT_STRIDE: usize = 0x1000;
/// Total number of VirtIO MMIO slots on QEMU virt.
const VIRTIO_MMIO_SLOT_COUNT: usize = 8;
/// VirtIO device-specific configuration space offset.
const VIRTIO_MMIO_CONFIG: usize = 0x100;


/// The size of one disk sector in bytes.
pub const SECTOR_SIZE: usize = 512;

/// VirtIO Block Request types (VirtIO Spec §5.2.6)
const VIRTIO_BLK_T_IN: u32 = 0; // Read from device

/// VirtIO Block status codes
const VIRTIO_BLK_S_OK: u8 = 0;

/// Block device request header sent to the device.
///
/// This must be placed in a descriptor that the device can read.
#[repr(C)]
struct VirtioBlkReq {
    /// Request type: VIRTIO_BLK_T_IN (0) for read, VIRTIO_BLK_T_OUT (1) for write
    blk_type: u32,
    /// Reserved, must be zero
    reserved: u32,
    /// First sector (Logical Block Address) to read/write
    sector: u64,
}

/// The complete VirtIO Block device state, stored in static memory.
///
/// VirtQueue memory is in VQ_BUF (a separate 8KB page-aligned static).
/// The VirtQueue structures (desc, avail, used) MUST be page-aligned for
/// the device to DMA correctly. VQ_BUF provides the contiguous buffer.
struct VirtioBlkState {
    /// Block request header for the current in-flight request
    blk_req: VirtioBlkReq,
    /// 512-byte data buffer for reads/writes
    data_buf: [u8; SECTOR_SIZE],
    /// 1-byte status response from the device
    status: u8,
    /// Whether this driver has been successfully initialized
    initialized: bool,
    /// Number of sectors on this device (from the device config space)
    capacity: u64,
    /// The MMIO base address of the discovered block device
    mmio_base: usize,
    /// Index into avail ring for next submission
    avail_idx: u16,
    /// Last seen used ring idx (for polling completion)
    last_used_idx: u16,
}

impl VirtioBlkState {
    const fn new() -> Self {
        Self {
            blk_req: VirtioBlkReq {
                blk_type: 0,
                reserved: 0,
                sector: 0,
            },
            data_buf: [0u8; SECTOR_SIZE],
            status: 0xFF,
            initialized: false,
            capacity: 0,
            mmio_base: 0,
            avail_idx: 0,
            last_used_idx: 0,
        }
    }
}

/// Single contiguous page-aligned VirtQueue buffer for VirtIO legacy (v1).
///
/// VirtIO Legacy computes avail/used ring offsets from a single base PFN:
///   Descriptors: buf[0..N*16]
///   Avail ring:  buf[N*16..] (immediately after descriptors, 2-byte aligned)
///   Used ring:   buf[page_aligned(N*16 + avail_size)..] (next page boundary)
///
/// Using 2 pages (8192 bytes) comfortably fits a QUEUE_SIZE=8 configuration:
///   Desc:  8 * 16 = 128 bytes
///   Avail: 4 + 8*2 + 2 = 22 bytes → total = 150 bytes, well within first page
///   Used:  4 + 8*8 + 2 = 70 bytes → starts at offset 4096 (second page)
#[repr(C, align(4096))]
struct LegacyVirtQueue {
    data: [u8; 8192],
}

static mut VQ_BUF: LegacyVirtQueue = LegacyVirtQueue { data: [0u8; 8192] };

unsafe fn desc_table() -> *mut VirtqDesc {
    unsafe { core::ptr::addr_of_mut!(VQ_BUF.data) as *mut VirtqDesc }
}

unsafe fn avail_ring() -> *mut VirtqAvail {
    unsafe { (core::ptr::addr_of_mut!(VQ_BUF.data) as usize + 128) as *mut VirtqAvail }
}

unsafe fn used_ring() -> *mut VirtqUsed {
    unsafe { (core::ptr::addr_of_mut!(VQ_BUF.data) as usize + 4096) as *mut VirtqUsed }
}

/// Global singleton VirtIO block driver state (control/metadata only).
static VIRTIO_BLK: Mutex<VirtioBlkState> = Mutex::new(VirtioBlkState::new());


/// Initialize the VirtIO block device.
///
/// Must be called once at kernel boot before any `read_sector` calls.
/// Returns `Ok(capacity_sectors)` on success, `Err` if no device is found
/// or initialization fails.
pub fn init() -> Result<u64, &'static str> {
    let mut state = VIRTIO_BLK.lock();

    // Scan all VirtIO MMIO slots to find the block device.
    // QEMU places virtio-blk at the highest available slot (slot index 7 = 0x10008000
    // when it is the only device), so we scan from high to low.
    let base = unsafe {
        let mut found = None;
        // Print and scan all 8 mapped VirtIO MMIO slots
        for slot in 0..VIRTIO_MMIO_SLOT_COUNT {
            let addr = VIRTIO_MMIO_SLOT_BASE + slot * VIRTIO_MMIO_SLOT_STRIDE;
            let magic = mmio_read(addr, VIRTIO_MMIO_MAGIC);
            let version = mmio_read(addr, super::VIRTIO_MMIO_VERSION);
            let dev_id = mmio_read(addr, VIRTIO_MMIO_DEVICE_ID);
            crate::println!(
                "[VIRTIO] Slot {:2}: 0x{:X}  magic=0x{:08X} ver={} dev_id={}",
                slot, addr, magic, version, dev_id
            );
            if magic == VIRTIO_MAGIC && dev_id == VIRTIO_DEVICE_ID_BLOCK {
                found = Some(addr);
                break;
            }
        }
        match found {
            Some(addr) => addr,
            None => return Err("VirtIO: No block device found in any MMIO slot"),
        }
    };
    state.mmio_base = base;
    crate::println!("[VIRTIO] Block device discovered at MMIO 0x{:X}", base);

    unsafe {
        // Read the device version to determine which protocol to use.
        // v1 = legacy MMIO (QUEUE_PFN), v2 = modern MMIO (split queue addresses).
        let version = mmio_read(base, super::VIRTIO_MMIO_VERSION);
        crate::println!("[VIRTIO] Device version: {}", version);

        // Step 2: Reset the device by writing 0 to Status
        mmio_write(base, VIRTIO_MMIO_STATUS, 0);

        // Step 3: Set ACKNOWLEDGE — OS has detected the device
        mmio_write(base, VIRTIO_MMIO_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);

        // Step 4: Set DRIVER — OS knows how to drive it
        mmio_write(base, VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

        if version == 1 {
            // ── VirtIO Legacy MMIO protocol ──────────────────────────────────
            // References: VirtIO Spec v1.0 Appendix D, OSDev VirtIO page.
            //
            // In legacy mode:
            //  - Feature negotiation uses only DEVICE_FEATURES / DRIVER_FEATURES
            //    without a SEL register and without FEATURES_OK.
            //  - The queue is set up as a single contiguous region pointed to by
            //    QUEUE_PFN (offset 0x040) — the 4KB page frame number.
            //  - Queue page size is configured via QUEUE_ALIGN (offset 0x03C) and
            //    QUEUE_PFN (offset 0x040). We must write QUEUE_ALIGN before QUEUE_PFN.

            // Feature negotiation (legacy — no SEL, no FEATURES_OK)
            let _device_features = mmio_read(base, VIRTIO_MMIO_DEVICE_FEATURES);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES, 0); // accept no optional features

            // Select queue 0
            mmio_write(base, VIRTIO_MMIO_QUEUE_SEL, 0);
            let max_queue = mmio_read(base, VIRTIO_MMIO_QUEUE_NUM_MAX);
            crate::println!("[VIRTIO DEBUG] max_queue = {}", max_queue);
            if max_queue == 0 {
                return Err("VirtIO legacy: queue 0 not available");
            }
            let queue_num = QUEUE_SIZE.min(max_queue as usize) as u32;
            mmio_write(base, VIRTIO_MMIO_QUEUE_NUM, queue_num);

            // Tell the device the guest page size (4096 bytes)
            mmio_write(base, VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE, 4096);

            // Tell the device the page granularity (4096 bytes)
            mmio_write(base, VIRTIO_MMIO_LEGACY_QUEUE_ALIGN, 4096);

            // Compute the PFN of our page-aligned VQ_BUF (desc table starts at offset 0)
            let buf_phys = core::ptr::addr_of!(VQ_BUF.data) as u64;
            let pfn = (buf_phys / 4096) as u32;
            crate::println!("[VIRTIO] Legacy PFN: 0x{:X} (phys=0x{:X})", pfn, buf_phys);
            mmio_write(base, VIRTIO_MMIO_LEGACY_QUEUE_PFN, pfn);
            let read_pfn = mmio_read(base, VIRTIO_MMIO_LEGACY_QUEUE_PFN);
            crate::println!("[VIRTIO DEBUG] Read back legacy PFN = 0x{:X}", read_pfn);

        } else {
            // ── VirtIO Modern MMIO protocol (version 2) ──────────────────────
            // Feature negotiation with SEL registers and FEATURES_OK check.
            mmio_write(base, VIRTIO_MMIO_DEVICE_FEATURES_SEL, 0);
            let _device_features = mmio_read(base, VIRTIO_MMIO_DEVICE_FEATURES);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES_SEL, 0);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES, 0);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES_SEL, 1);
            mmio_write(base, VIRTIO_MMIO_DRIVER_FEATURES, 0);

            mmio_write(base, VIRTIO_MMIO_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK);
            let status = mmio_read(base, VIRTIO_MMIO_STATUS);
            if status & VIRTIO_STATUS_FEATURES_OK == 0 {
                return Err("VirtIO modern: device rejected feature negotiation");
            }

            mmio_write(base, VIRTIO_MMIO_QUEUE_SEL, 0);
            let max_queue = mmio_read(base, VIRTIO_MMIO_QUEUE_NUM_MAX);
            if max_queue == 0 {
                return Err("VirtIO modern: queue 0 not available");
            }
            let queue_num = QUEUE_SIZE.min(max_queue as usize) as u32;
            mmio_write(base, VIRTIO_MMIO_QUEUE_NUM, queue_num);

            let desc_phys = desc_table() as u64;
            mmio_write(base, VIRTIO_MMIO_QUEUE_DESC_LOW, desc_phys as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DESC_HIGH, (desc_phys >> 32) as u32);

            let avail_phys = avail_ring() as u64;
            mmio_write(base, VIRTIO_MMIO_QUEUE_DRIVER_LOW, avail_phys as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DRIVER_HIGH, (avail_phys >> 32) as u32);

            let used_phys = used_ring() as u64;
            mmio_write(base, VIRTIO_MMIO_QUEUE_DEVICE_LOW, used_phys as u32);
            mmio_write(base, VIRTIO_MMIO_QUEUE_DEVICE_HIGH, (used_phys >> 32) as u32);

            mmio_write(base, VIRTIO_MMIO_QUEUE_READY, 1);
        }

        // Step 9: Read device capacity from config space (offset 0 = number of sectors)
        let capacity_lo = mmio_read(base, VIRTIO_MMIO_CONFIG) as u64;
        let capacity_hi = mmio_read(base, VIRTIO_MMIO_CONFIG + 4) as u64;
        state.capacity = capacity_lo | (capacity_hi << 32);

        // Step 10: Set DRIVER_OK — initialization complete!
        mmio_write(base, VIRTIO_MMIO_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK);

        state.initialized = true;
    }

    let cap = state.capacity;
    let base_addr = state.mmio_base;
    crate::println!(
        "[VIRTIO] Block device ready at 0x{:X}. Capacity: {} sectors ({} KB)",
        base_addr,
        cap,
        cap / 2
    );

    Ok(state.capacity)
}

/// Read one 512-byte sector from the block device into the provided buffer.
///
/// # Arguments
/// * `lba` - Logical Block Address (sector number, 0-indexed)
/// * `buf` - Output buffer; must be exactly `SECTOR_SIZE` (512) bytes
///
/// # Returns
/// `Ok(())` on success, `Err` if the device returned an error status.
pub fn read_sector(lba: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let mut state = VIRTIO_BLK.lock();

    if !state.initialized {
        return Err("VirtIO block driver not initialized");
    }

    let base = state.mmio_base;

    unsafe {
        // Prepare the block request header
        state.blk_req.blk_type = VIRTIO_BLK_T_IN;
        state.blk_req.reserved = 0;
        state.blk_req.sector = lba;
        state.status = 0xFF; // 0xFF = unset; device will write 0 on success

        // Build a 3-descriptor chain using page-aligned VQ_DESC:
        // Desc[0]: BlkReq header (device reads this to know what we want)
        // Desc[1]: Data buffer  (device writes the sector data here)
        // Desc[2]: Status byte  (device writes 0=OK / 1=IOERR / 2=UNSUPP)

        let req_phys = &state.blk_req as *const VirtioBlkReq as u64;
        let buf_phys = state.data_buf.as_ptr() as u64;
        let status_phys = &state.status as *const u8 as u64;

        let desc = desc_table();
        let avail = avail_ring();
        let used = used_ring();

        crate::println!("[VIRTIO DEBUG] req_phys=0x{:X}, buf_phys=0x{:X}, status_phys=0x{:X}", req_phys, buf_phys, status_phys);
        crate::println!("[VIRTIO DEBUG] desc_table=0x{:X}, avail_ring=0x{:X}, used_ring=0x{:X}", desc as usize, avail as usize, used as usize);

        // Descriptor 0: request header (readable by device)
        core::ptr::write_volatile(desc.offset(0), VirtqDesc {
            addr: req_phys,
            len: core::mem::size_of::<VirtioBlkReq>() as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next: 1,
        });

        // Descriptor 1: data buffer (device writes sector data here)
        core::ptr::write_volatile(desc.offset(1), VirtqDesc {
            addr: buf_phys,
            len: SECTOR_SIZE as u32,
            flags: VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            next: 2,
        });

        // Descriptor 2: status byte (device writes completion status here)
        core::ptr::write_volatile(desc.offset(2), VirtqDesc {
            addr: status_phys,
            len: 1,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        });

        // Add descriptor chain head (index 0) to the available ring
        let avail_ring_slot = (state.avail_idx as usize) % QUEUE_SIZE;
        core::ptr::write_volatile(
            core::ptr::addr_of_mut!((*avail).ring[avail_ring_slot]),
            0,
        );
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let new_idx = core::ptr::read_volatile(core::ptr::addr_of!((*avail).idx)).wrapping_add(1);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*avail).idx), new_idx);
        state.avail_idx = new_idx;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        // Debug print the written descriptor and avail fields
        crate::println!(
            "[VIRTIO DEBUG] Desc[0]: addr=0x{:X}, len={}, flags={}, next={}",
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(0)).addr)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(0)).len)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(0)).flags)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(0)).next))
        );
        crate::println!(
            "[VIRTIO DEBUG] Desc[1]: addr=0x{:X}, len={}, flags={}, next={}",
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(1)).addr)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(1)).len)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(1)).flags)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(1)).next))
        );
        crate::println!(
            "[VIRTIO DEBUG] Desc[2]: addr=0x{:X}, len={}, flags={}, next={}",
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(2)).addr)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(2)).len)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(2)).flags)),
            core::ptr::read_volatile(core::ptr::addr_of!((*desc.offset(2)).next))
        );
        crate::println!(
            "[VIRTIO DEBUG] Avail: flags={}, idx={}, ring[0]={}",
            core::ptr::read_volatile(core::ptr::addr_of!((*avail).flags)),
            core::ptr::read_volatile(core::ptr::addr_of!((*avail).idx)),
            core::ptr::read_volatile(core::ptr::addr_of!((*avail).ring[0]))
        );

        // Notify the device that queue 0 has a new request
        mmio_write(base, VIRTIO_MMIO_QUEUE_NOTIFY, 0);

        // Poll the used ring until the device has processed our request
        let target_used_idx = state.last_used_idx.wrapping_add(1);
        let mut spins = 0u32;
        loop {
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            let current_used = core::ptr::read_volatile(core::ptr::addr_of!((*used).idx));
            if current_used == target_used_idx {
                break;
            }
            spins += 1;
            if spins > 50_000_000 {
                let dev_status = mmio_read(base, VIRTIO_MMIO_STATUS);
                let current_avail = core::ptr::read_volatile(core::ptr::addr_of!((*avail).idx));
                crate::println!(
                    "[VIRTIO] TIMEOUT: used.idx={}, target={}, avail.idx={}, dev_status=0x{:X}, blk_status=0x{:X}",
                    current_used,
                    target_used_idx,
                    current_avail,
                    dev_status,
                    core::ptr::read_volatile(&state.status)
                );
                return Err("VirtIO: Device read timed out");
            }
        }
        state.last_used_idx = target_used_idx;

        // Check device status byte
        if state.status != VIRTIO_BLK_S_OK {
            return Err("VirtIO: Block read returned error status");
        }

        // Copy result from internal data_buf into caller's buffer
        buf.copy_from_slice(&state.data_buf);
    }

    Ok(())
}

/// Read `count` consecutive sectors starting at `lba` into `buf`.
///
/// # Arguments
/// * `lba` - Starting logical block address
/// * `count` - Number of sectors to read
/// * `buf` - Output buffer; must be at least `count * SECTOR_SIZE` bytes
pub fn read_sectors(lba: u64, count: usize, buf: &mut [u8]) -> Result<(), &'static str> {
    if buf.len() < count * SECTOR_SIZE {
        return Err("VirtIO: Output buffer too small for requested sector count");
    }

    let mut sector_buf = [0u8; SECTOR_SIZE];
    for i in 0..count {
        read_sector(lba + i as u64, &mut sector_buf)?;
        let offset = i * SECTOR_SIZE;
        buf[offset..offset + SECTOR_SIZE].copy_from_slice(&sector_buf);
    }

    Ok(())
}

/// Returns the capacity of the block device in sectors.
pub fn capacity() -> u64 {
    VIRTIO_BLK.lock().capacity
}
