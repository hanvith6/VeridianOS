use spin::Mutex;
use super::ring::DkcpRing;
use super::types::DkcpMessage;

/// When true the `dkcp_send_net` stub will route messages through the
/// virtio-net driver instead of returning an error.  Flip to `true` once
/// Phase 11 two-QEMU testing is enabled.
pub const USE_REAL_NET: bool = true;

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

pub struct VirtioNetTransport;

impl DkcpTransport for VirtioNetTransport {
    fn send(&self, msg: &DkcpMessage) -> Result<(), DkcpError> {
        let mut frame = [0u8; 14 + 64];
        
        let dst_id = msg.dst_domain.0 as u8;
        let src_id = msg.src_domain.0 as u8;
        
        // Destination MAC: 52:54:00:12:34:XX
        frame[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, dst_id]);
        // Source MAC: 52:54:00:12:34:YY
        frame[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, src_id]);
        
        // EtherType: 0x88B5
        frame[12..14].copy_from_slice(&[0x88, 0xB5]);
        
        // Payload: DkcpMessage (64 bytes)
        let msg_bytes = unsafe {
            core::slice::from_raw_parts(msg as *const _ as *const u8, 64)
        };
        frame[14..78].copy_from_slice(msg_bytes);
        
        crate::virtio::net::send_packet(&frame).map_err(|_| DkcpError::RingFull)
    }

    fn recv(&self) -> Option<DkcpMessage> {
        let mut rx_buf = [0u8; crate::virtio::net::MAX_PACKET_SIZE];
        if let Some(len) = crate::virtio::net::try_recv_packet(&mut rx_buf) {
            if len >= 78 {
                if rx_buf[12] == 0x88 && rx_buf[13] == 0xB5 {
                    let mut msg = DkcpMessage::zeroed();
                    let msg_bytes = unsafe {
                        core::slice::from_raw_parts_mut(&mut msg as *mut _ as *mut u8, 64)
                    };
                    msg_bytes.copy_from_slice(&rx_buf[14..78]);
                    return Some(msg);
                }
            }
        }
        None
    }
}

/// Global active transport dynamically dispatched
pub static ACTIVE_TRANSPORT: Mutex<&'static dyn DkcpTransport> = Mutex::new(&LoopbackTransport);

/// Send a coherence protocol message via the active transport.
pub fn dkcp_send(msg: &DkcpMessage) -> Result<(), DkcpError> {
    let local_id = super::cluster::CLUSTER.lock().local_id;
    if msg.dst_domain == local_id || msg.dst_domain == super::types::KernelDomainId::LOCAL {
        let transport = LoopbackTransport;
        transport.send(msg)
    } else {
        ACTIVE_TRANSPORT.lock().send(msg)
    }
}

/// Receive a coherence protocol message from the active transport if available.
pub fn dkcp_recv() -> Option<DkcpMessage> {
    let transport = LoopbackTransport;
    if let Some(msg) = transport.recv() {
        return Some(msg);
    }
    ACTIVE_TRANSPORT.lock().recv()
}

/// Send a DKCP coherence message over the virtio-net device.
pub fn dkcp_send_net(msg: &DkcpMessage) -> Result<(), DkcpError> {
    if crate::virtio::net::is_initialized() {
        let transport = VirtioNetTransport;
        transport.send(msg)
    } else {
        Err(DkcpError::RingFull)
    }
}
