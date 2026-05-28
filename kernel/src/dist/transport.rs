use super::ring::DkcpRing;
use super::types::DkcpMessage;

/// Static loopback ring buffer for QEMU simulation.
static LOOPBACK_RING: DkcpRing = DkcpRing::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DkcpError {
    RingFull,
}

/// Send a coherence protocol message via the loopback transport.
///
/// Enqueues the message into the loopback ring buffer and fires a supervisor IPI
/// to signal that new data is available.
pub fn dkcp_send(msg: &DkcpMessage) -> Result<(), DkcpError> {
    LOOPBACK_RING.enqueue(msg).map_err(|_| DkcpError::RingFull)?;

    // Fire an SBI IPI to Hart 0 to wake up/notify the recipient.
    // In a multi-hart QEMU configuration, this triggers the software interrupt handler.
    crate::sbi::sbi_send_ipi(1, 0);

    Ok(())
}

/// Receive a coherence protocol message from the loopback transport if available.
pub fn dkcp_recv() -> Option<DkcpMessage> {
    LOOPBACK_RING.dequeue()
}
