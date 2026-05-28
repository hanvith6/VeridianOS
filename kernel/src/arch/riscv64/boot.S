# VeridianOS — RISC-V 64-bit Boot Entry Point
#
# This is the very first code that executes when OpenSBI transfers control
# to the kernel. It runs in S-mode (Supervisor mode) on the QEMU virt machine.
#
# Responsibilities:
#   1. Determine hart ID — only hart 0 proceeds, others park
#   2. Set up the initial stack pointer
#   3. Zero out the BSS section
#   4. Jump to the Rust `kmain` entry point
#
# References:
#   - RISC-V Privileged Spec v1.12 §3 (Supervisor-Level ISA)
#   - "OS in 1000 Lines" (RISC-V boot sequence)
#   - Asterinas boot flow (USENIX ATC'25)

.attribute arch, "rv64gc"

.section .text.entry
.globl _start

_start:
    # Save hart ID in thread pointer register (tp)
    mv      tp, a0

    # ---------------------------------------------------------------
    # Hart selection: Only hart 0 boots the kernel.
    # Other harts spin in a WFI loop until woken by IPI.
    # a0 = hart ID (passed by OpenSBI)
    # a1 = pointer to device tree blob (DTB)
    # ---------------------------------------------------------------
    # In S-mode, OpenSBI passes hart ID in a0
    bnez    a0, _park_hart          # If not hart 0, park

    # ---------------------------------------------------------------
    # Set up the stack pointer
    # Stack grows downward; _stack_top is defined in linker.ld
    # ---------------------------------------------------------------
    la      sp, _stack_top

.globl _secondary_start
_secondary_start:
    # Save hart ID in thread pointer register (tp)
    mv      tp, a0

    # Set up stack pointer for this hart: sp = _stack_top - (hart_id * 8192)
    la      sp, _stack_top
    li      t0, 8192
    mul     t0, a0, t0
    sub     sp, sp, t0

    # Jump to secondary entry point in Rust
    call    ksecondary_main
_secondary_park:
    wfi
    j       _secondary_park

    # ---------------------------------------------------------------
    # Zero out BSS section
    # BSS contains uninitialized globals that must be zeroed
    # ---------------------------------------------------------------
    la      t0, _bss_start
    la      t1, _bss_end
_zero_bss:
    beq     t0, t1, _bss_done
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       _zero_bss
_bss_done:

    # ---------------------------------------------------------------
    # Save device tree pointer for later use
    # a1 contains the DTB address from OpenSBI
    # ---------------------------------------------------------------
    # a0 = hart ID (already 0)
    # a1 = DTB pointer (preserved from OpenSBI)

    # ---------------------------------------------------------------
    # Jump to Rust entry point
    # kmain(hart_id: usize, dtb_ptr: usize)
    # ---------------------------------------------------------------
    call    kmain

    # If kmain returns (shouldn't happen), halt
    j       _park_hart

    # ---------------------------------------------------------------
    # Park non-boot harts
    # They spin in a low-power WFI (Wait For Interrupt) loop
    # until the kernel sends an IPI to wake them
    # ---------------------------------------------------------------
_park_hart:
    wfi
    j       _park_hart

# -------------------------------------------------------------------
# switch_context(current: *mut ThreadContext, next: *const ThreadContext)
#
# Register assignment:
#   a0 = pointer to current ThreadContext (to save registers)
#   a1 = pointer to next ThreadContext (to load registers)
# -------------------------------------------------------------------
.globl switch_context
.align 2
switch_context:
    # 1. Save the current CPU context (callee-saved registers)
    sd      ra, 0(a0)
    sd      sp, 8(a0)
    sd      s0, 16(a0)
    sd      s1, 24(a0)
    sd      s2, 32(a0)
    sd      s3, 40(a0)
    sd      s4, 48(a0)
    sd      s5, 56(a0)
    sd      s6, 64(a0)
    sd      s7, 72(a0)
    sd      s8, 80(a0)
    sd      s9, 88(a0)
    sd      s10, 96(a0)
    sd      s11, 104(a0)

    # 2. Load the next CPU context
    ld      ra, 0(a1)
    ld      sp, 8(a1)
    ld      s0, 16(a1)
    ld      s1, 24(a1)
    ld      s2, 32(a1)
    ld      s3, 40(a1)
    ld      s4, 48(a1)
    ld      s5, 56(a1)
    ld      s6, 64(a1)
    ld      s7, 72(a1)
    ld      s8, 80(a1)
    ld      s9, 88(a1)
    ld      s10, 96(a1)
    ld      s11, 104(a1)

    # 3. Return to the instruction address in the newly loaded 'ra' register
    ret

# -------------------------------------------------------------------
# trap_entry
#
# Low-level exception and interrupt handler.
# If the CPU encounters a trap (illegal instruction, page fault, etc.),
# it jumps here. We print "TRAP" directly to the UART console and halt.
# -------------------------------------------------------------------
.globl trap_entry
.align 4
trap_entry:
    # Write "TRAP\n" directly to UART (0x10000000) via MMIO
    li      t0, 0x10000000
    li      t1, 84          # 'T'
    sb      t1, 0(t0)
    li      t1, 82          # 'R'
    sb      t1, 0(t0)
    li      t1, 65          # 'A'
    sb      t1, 0(t0)
    li      t1, 80          # 'P'
    sb      t1, 0(t0)
    li      t1, 10          # '\n'
    sb      t1, 0(t0)

_trap_loop:
    wfi
    j       _trap_loop
