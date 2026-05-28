/*
 * VeridianOS Linker Script — RISC-V 64-bit (QEMU virt machine)
 *
 * Memory layout for the QEMU 'virt' platform:
 *   0x8000_0000 — OpenSBI firmware (loaded by QEMU -bios default)
 *   0x8020_0000 — Kernel image entry point (OpenSBI jumps here)
 *
 * References:
 *   - RISC-V Privileged Spec v1.12
 *   - QEMU virt machine memory map
 *   - Asterinas linker layout (USENIX ATC'25)
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x80200000;

SECTIONS
{
    . = BASE_ADDRESS;

    /* --- Executable Code --- */
    .text : {
        _text_start = .;
        *(.text.entry)      /* Assembly entry point — must come first */
        *(.text .text.*)
        _text_end = .;
    }

    /* --- Read-Only Data --- */
    . = ALIGN(4K);
    .rodata : {
        _rodata_start = .;
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
        _rodata_end = .;
    }

    /* --- Initialized Data --- */
    . = ALIGN(4K);
    .data : {
        _data_start = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
        _data_end = .;
    }

    /* --- BSS (Zero-Initialized Data) --- */
    . = ALIGN(4K);
    .bss : {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _bss_end = .;
    }

    . = ALIGN(4K);

    /* --- Kernel Stack (64 KB — supports up to 8 harts × 8KB each) --- */
    _stack_bottom = .;
    . += 64K;
    _stack_top = .;

    /* --- Kernel Heap (16 MB) --- */
    . = ALIGN(4K);
    _heap_start = .;
    . += 16M;
    _heap_end = .;

    /* --- Free Physical Memory (for page allocator) --- */
    . = ALIGN(4K);
    _free_mem_start = .;

    /* End of kernel image */
    _kernel_end = .;

    /* Discard unnecessary sections */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
    }
}
