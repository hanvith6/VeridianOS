# VeridianOS — RISC-V 64-bit Low-Level Trap Vector
#
# Implements the trap vector entry point `trap_vector`.
# On entry:
#   - If we came from User Mode:
#       * `sscratch` contains the kernel stack pointer.
#       * `sp` contains the user stack pointer.
#   - If we came from Supervisor Mode:
#       * `sscratch` contains 0.
#       * `sp` contains the supervisor stack pointer.

.attribute arch, "rv64gc"

.section .text
.globl trap_vector
.align 4
trap_vector:
    # Swap user sp and kernel sp using sscratch
    csrrw sp, sscratch, sp
    bnez sp, .from_user

    # Trap from supervisor mode: restore sp, keep sscratch = 0
    csrrw sp, sscratch, sp
    addi sp, sp, -272
    sd x1, 8(sp)
    # Calculate and save original supervisor sp (before subtraction)
    addi x1, sp, 272
    sd x1, 16(sp)
    j .save_remaining

.from_user:
    # Trap from user mode: sp is now kernel stack, sscratch is user sp.
    # Allocate trap frame
    addi sp, sp, -272
    sd x1, 8(sp)
    # Read user sp from sscratch, and save to TrapFrame
    csrr x1, sscratch
    sd x1, 16(sp)
    # Clear sscratch (indicates we are in kernel mode)
    csrw sscratch, zero

.save_remaining:
    # Save general purpose registers x3-x31
    sd x3, 24(sp)
    sd x4, 32(sp)
    sd x5, 40(sp)
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)

    # Save sstatus and sepc
    csrr t0, sstatus
    sd t0, 256(sp)
    csrr t1, sepc
    sd t1, 264(sp)

    # Pass the trap frame pointer (sp) as the first argument (a0)
    mv a0, sp
    
    # Call the Rust trap handler
    call trap_handler

    # On return from trap_handler, sp points to the TrapFrame (it might have been modified)
    # Now restore the registers.
    # Load sstatus and sepc back
    ld t0, 256(sp)
    csrw sstatus, t0
    ld t1, 264(sp)
    csrw sepc, t1

    # Check if we are returning to user mode
    # bit 8 of sstatus: 1 << 8 = 0x100
    andi t0, t0, 0x100
    bnez t0, .to_supervisor

    # Returning to user mode:
    # sscratch must hold the kernel stack pointer (which is sp + 272)
    addi t0, sp, 272
    csrw sscratch, t0
    j .restore_regs

.to_supervisor:
    # Returning to supervisor mode:
    # sscratch must hold 0
    csrw sscratch, zero

.restore_regs:
    # Restore general purpose registers x1, x3-x31
    ld x1, 8(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    ld x6, 48(sp)
    ld x7, 56(sp)
    ld x8, 64(sp)
    ld x9, 72(sp)
    ld x10, 80(sp)
    ld x11, 88(sp)
    ld x12, 96(sp)
    ld x13, 104(sp)
    ld x14, 112(sp)
    ld x15, 120(sp)
    ld x16, 128(sp)
    ld x17, 136(sp)
    ld x18, 144(sp)
    ld x19, 152(sp)
    ld x20, 160(sp)
    ld x21, 168(sp)
    ld x22, 176(sp)
    ld x23, 184(sp)
    ld x24, 192(sp)
    ld x25, 200(sp)
    ld x26, 208(sp)
    ld x27, 216(sp)
    ld x28, 224(sp)
    ld x29, 232(sp)
    ld x30, 240(sp)
    ld x31, 248(sp)

    # Finally, restore sp (x2)
    ld sp, 16(sp)

    # Return to previous privilege level
    sret

.globl user_code_start
.globl user_code_end
.align 4
user_code_start:
    # 1. SYS_WRITE (1)
    # We load arguments for the syscall.
    # We want to print the message string. Since it's located at `user_msg` (PC-relative),
    # we can use `lla` or `la` to get its address. Let's use `lla` (load local address) which is PIC/PC-relative.
    lla a0, user_msg
    # msg length: 37 bytes
    li a1, 37
    # SYS_WRITE number is 1
    li a7, 1
    ecall

    # 2. SYS_EXIT (2)
    # exit code = 0
    li a0, 0
    li a7, 2
    ecall

.align 4
user_msg:
    .ascii "Hello from real User Mode via ecall!\n"
user_code_end:

