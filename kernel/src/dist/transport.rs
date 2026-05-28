use spin::Mutex;
use super::ring::DkcpRing;
use super::types::DkcpMessage;

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
