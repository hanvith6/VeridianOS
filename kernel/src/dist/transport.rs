use spin::Mutex;
use super::ring::DkcpRing;
use super::types::DkcpMessage;

/// When true the `dkcp_send_net` stub will route messages through the
/// virtio-net driver instead of returning an error.  Flip to `true` once
/// Phase 11 two-QEMU testing is enabled.
pub const USE_REAL_NET: bool = false;

pub trait DkcpTransport: Send + Sync {
    fn send(&self, msg: &DkcpMessage) -> Result<(), DkcpError>;
    fn recv(&self) -> Option<DkcpMessage>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DkcpError {
    RingFull,
}

pub struct LoopbackTransport;

static LOOPBACK_RING: DkcpRing = DkcpRing::new();

impl DkcpTransport for LoopbackTransport {
    fn send(&self, msg: &DkcpMessage) -> Result<(), DkcpError> {
        LOOPBACK_RING.enqueue(msg).map_err(|_| DkcpError::RingFull)?;

        // Fire an SBI IPI to Hart 0 to wake up/notify the recipient.
        crate::sbi::sbi_send_ipi(1, 0);

        Ok(())
    }

    fn recv(&self) -> Option<DkcpMessage> {
        LOOPBACK_RING.dequeue()
    }
}

/// Global active transport dynamically dispatched
pub static ACTIVE_TRANSPORT: Mutex<&'static dyn DkcpTransport> = Mutex::new(&LoopbackTransport);

/// Send a coherence protocol message via the active transport.
pub fn dkcp_send(msg: &DkcpMessage) -> Result<(), DkcpError> {
    ACTIVE_TRANSPORT.lock().send(msg)
}

/// Receive a coherence protocol message from the active transport if available.
pub fn dkcp_recv() -> Option<DkcpMessage> {
    ACTIVE_TRANSPORT.lock().recv()
}

/// Send a DKCP coherence message over the virtio-net device.
///
/// Serialises the 64-byte `DkcpMessage` directly as a raw Ethernet payload
/// (no IP/UDP framing — this is a low-level kernel transport stub).
///
/// Returns `Err(DkcpError::RingFull)` when the net driver is not yet
/// initialised or the underlying send fails.  Callers should fall back to
/// the loopback transport when `USE_REAL_NET` is false.
pub fn dkcp_send_net(msg: &DkcpMessage) -> Result<(), DkcpError> {
    if crate::virtio::net::is_initialized() {
        let bytes = unsafe {
            core::slice::from_raw_parts(msg as *const _ as *const u8, 64)
        };
        crate::virtio::net::send_packet(bytes).map_err(|_| DkcpError::RingFull)
    } else {
        Err(DkcpError::RingFull)
    }
}
