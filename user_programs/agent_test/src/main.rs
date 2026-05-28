//! VeridianOS Agent Runtime Verification Program (Phase 9)
//!
//! Tests the complete agent lifecycle:
//! 1. Spawn two agents (A=orchestrator, B=worker)
//! 2. Create an IPC channel
//! 3. Agent A sends a task message to Agent B
//! 4. Agent B receives the message and verifies content
//! 5. Verify agent status reporting
//! 6. Exit cleanly with code 0

#![no_std]
#![no_main]

use core::panic::PanicInfo;

// Syscall inline assembly helper
#[inline(always)]
pub fn syscall5(id: usize, arg0: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) -> isize {
    let ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") id,
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            in("a3") arg3,
            in("a4") arg4,
            lateout("a0") ret,
        );
    }
    ret
}

const SYS_WRITE: usize = 1;
const SYS_EXIT: usize = 2;
const SYS_AGENT_SPAWN: usize = 70;
const SYS_CHANNEL_CREATE: usize = 71;
const SYS_CHANNEL_SEND: usize = 72;
const SYS_CHANNEL_RECV: usize = 73;
const SYS_AGENT_STATUS: usize = 74;

fn print(s: &str) {
    syscall5(SYS_WRITE, s.as_ptr() as usize, s.len(), 0, 0, 0);
}

const MSG_SIZE: usize = 64;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("[USER] VeridianOS Phase 9 Agent Runtime Verification\n");
    print("[USER] =============================================\n");

    // 1. Spawn Agent A (orchestrator, parent=0 means kernel root)
    let intent_a = b"orchestrator-agent-00000000000000"; // exactly 32 bytes
    let agent_a_ret = syscall5(
        SYS_AGENT_SPAWN,
        0, // parent = root
        intent_a.as_ptr() as usize,
        32,
        0,
        0,
    );
    if agent_a_ret < 0 {
        print("[USER] FAIL: Could not spawn Agent A\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    let agent_a_id = agent_a_ret as u32;
    print("[USER] Agent A (orchestrator) spawned successfully.\n");

    // 2. Spawn Agent B (worker, parent = Agent A)
    let intent_b = b"worker-agent-000000000000000000\0"; // 32 bytes (31 + null)
    let agent_b_ret = syscall5(
        SYS_AGENT_SPAWN,
        agent_a_id as usize,
        intent_b.as_ptr() as usize,
        31,
        0,
        0,
    );
    if agent_b_ret < 0 {
        print("[USER] FAIL: Could not spawn Agent B\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    let agent_b_id = agent_b_ret as u32;
    print("[USER] Agent B (worker) spawned successfully.\n");

    // 3. Create IPC channel (owned by Agent A)
    let ch_ret = syscall5(SYS_CHANNEL_CREATE, agent_a_id as usize, 0, 0, 0, 0);
    if ch_ret < 0 {
        print("[USER] FAIL: Could not create channel\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    let channel_id = ch_ret as usize;
    print("[USER] IPC channel created successfully.\n");

    // 4. Agent A sends a task message to Agent B via channel
    let mut msg = [0u8; MSG_SIZE];
    let task_text = b"compute: fib(42)";
    msg[..task_text.len()].copy_from_slice(task_text);
    // Encode sender id in last 4 bytes
    msg[60] = (agent_a_id & 0xFF) as u8;
    msg[61] = ((agent_a_id >> 8) & 0xFF) as u8;
    msg[62] = ((agent_a_id >> 16) & 0xFF) as u8;
    msg[63] = ((agent_a_id >> 24) & 0xFF) as u8;

    let send_ret = syscall5(
        SYS_CHANNEL_SEND,
        channel_id,
        msg.as_ptr() as usize,
        MSG_SIZE,
        0,
        0,
    );
    if send_ret < 0 {
        print("[USER] FAIL: Channel send failed\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    print("[USER] Task message sent via IPC channel successfully.\n");

    // 5. Agent B receives the message
    let mut recv_buf = [0u8; MSG_SIZE];
    let mut recv_len: usize = 0;
    let recv_ret = syscall5(
        SYS_CHANNEL_RECV,
        channel_id,
        recv_buf.as_mut_ptr() as usize,
        &mut recv_len as *mut usize as usize,
        0,
        0,
    );
    if recv_ret < 0 {
        print("[USER] FAIL: Channel recv failed\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    print("[USER] Task message received from IPC channel successfully.\n");

    // 6. Verify message content matches what was sent
    let mut content_ok = true;
    let mut i = 0usize;
    while i < task_text.len() {
        if recv_buf[i] != task_text[i] {
            content_ok = false;
            break;
        }
        i += 1;
    }
    if !content_ok {
        print("[USER] FAIL: Message content verification failed!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    print("[USER] IPC message content verified: 'compute: fib(42)' intact.\n");

    // 7. Verify sender ID in message matches Agent A's ID
    let decoded_sender = (recv_buf[60] as u32)
        | ((recv_buf[61] as u32) << 8)
        | ((recv_buf[62] as u32) << 16)
        | ((recv_buf[63] as u32) << 24);
    if decoded_sender != agent_a_id {
        print("[USER] FAIL: Sender ID mismatch in received message!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    print("[USER] Sender agent ID verified in message payload.\n");

    // 8. Check agent A status
    let mut state_a: u8 = 255;
    let status_a_ret = syscall5(
        SYS_AGENT_STATUS,
        agent_a_id as usize,
        &mut state_a as *mut u8 as usize,
        0,
        0,
        0,
    );
    if status_a_ret < 0 || state_a == 255 {
        print("[USER] FAIL: Agent A status query failed\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    print("[USER] Agent A status queried successfully.\n");

    // 9. Check agent B status (child of A)
    let mut state_b: u8 = 255;
    let status_b_ret = syscall5(
        SYS_AGENT_STATUS,
        agent_b_id as usize,
        &mut state_b as *mut u8 as usize,
        0,
        0,
        0,
    );
    if status_b_ret < 0 || state_b == 255 {
        print("[USER] FAIL: Agent B status query failed\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
        loop {}
    }
    print("[USER] Agent B status (child of A) queried successfully.\n");

    // ALL CHECKS PASSED
    print("[USER] =============================================\n");
    print("[USER] Agent Runtime Verification SUCCESS!\n");
    print("[USER] Phase 9 Complete: Agents, Channels, IPC all working!\n");
    syscall5(SYS_EXIT, 0, 0, 0, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    loop {}
}
