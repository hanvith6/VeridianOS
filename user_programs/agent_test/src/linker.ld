ENTRY(_start)

SECTIONS
{
    /* The agent_test user program starts at virtual address 0x4000_0000 */
    . = 0x40000000;

    .text : {
        _text_start = .;
        *(.text.entry)
        *(.text .text.*)
        _text_end = .;
    }

    .rodata : {
        _rodata_start = .;
        *(.rodata .rodata.*)
        _rodata_end = .;
    }

    .data : {
        _data_start = .;
        *(.data .data.*)
        _data_end = .;
    }

    .bss : {
        _bss_start = .;
        *(.bss .bss.*)
        _bss_end = .;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
    }
}
