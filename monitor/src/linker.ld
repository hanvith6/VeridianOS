/*
 * VeridianOS M-mode Security Monitor Linker Script
 *
 * Links the monitor at 0x80000000 (standard RISC-V BIOS entry).
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x80000000;

SECTIONS
{
    . = BASE_ADDRESS;

    /* --- Executable Code --- */
    .text : {
        _text_start = .;
        *(.text.entry)      /* Entry point must come first */
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
    /* Monitor Stack: 32 KB (8 KB per hart for 4 harts) */
    _stack_bottom = .;
    . += 32K;
    _stack_top = .;

    _monitor_end = .;

    /* Discard unnecessary sections */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
    }
}
