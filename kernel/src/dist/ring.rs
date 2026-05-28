use core::sync::atomic::{AtomicU32, Ordering};
use core::cell::UnsafeCell;
use super::types::DkcpMessage;

/// Single-producer, single-consumer ring buffer for DKCP messages.
/// Mapped into the shared memory window at a fixed physical address.
/// Uses an atomic head/tail for coordination between domains without locks.
#[repr(C, align(4096))]
pub struct DkcpRing {
    pub head: AtomicU32,           // written by consumer
    pub tail: AtomicU32,           // written by producer
    pub _pad: [u8; 56],            // pad to cache line
    pub slots: UnsafeCell<[DkcpMessage; 256]>, // 256 × 64 bytes = 16 KB ring body
}

impl DkcpRing {
    pub const fn new() -> Self {
        Self {
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            _pad: [0; 56],
            slots: UnsafeCell::new([DkcpMessage::zeroed(); 256]),
        }
    }

    pub fn enqueue(&self, msg: &DkcpMessage) -> Result<(), &'static str> {
        let h = self.head.load(Ordering::Acquire);
        let t = self.tail.load(Ordering::Acquire);

        if t.wrapping_sub(h) >= 256 {
            return Err("Ring buffer full");
        }

        let idx = (t % 256) as usize;
        unsafe {
            let slots_ptr = self.slots.get();
            let slot_ptr = core::ptr::addr_of_mut!((*slots_ptr)[idx]);
            core::ptr::write_volatile(slot_ptr, *msg);
        }

        self.tail.store(t.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    pub fn dequeue(&self) -> Option<DkcpMessage> {
        let h = self.head.load(Ordering::Acquire);
        let t = self.tail.load(Ordering::Acquire);

        if h == t {
            return None;
        }

        let idx = (h % 256) as usize;
        let msg = unsafe {
            let slots_ptr = self.slots.get();
            let slot_ptr = core::ptr::addr_of!((*slots_ptr)[idx]);
            core::ptr::read_volatile(slot_ptr)
        };

        self.head.store(h.wrapping_add(1), Ordering::Release);
        Some(msg)
    }
}

impl core::fmt::Debug for DkcpRing {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DkcpRing")
            .field("head", &self.head.load(Ordering::Relaxed))
            .field("tail", &self.tail.load(Ordering::Relaxed))
            .finish()
    }
}

// Compile-time static assertions for layout verification
const _: () = assert!(core::mem::size_of::<DkcpMessage>() == 64);
const _: () = assert!(core::mem::size_of::<DkcpRing>() == 20480);
const _: () = assert!(core::mem::align_of::<DkcpRing>() == 4096);

// Safety requirements for sharing across harts / domains
unsafe impl Send for DkcpRing {}
unsafe impl Sync for DkcpRing {}
