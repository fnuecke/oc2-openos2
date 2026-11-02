/* SPDX-License-Identifier: MIT */
OUTPUT_ARCH(riscv)
ENTRY(_start)

/* RAM base + 64 KiB, the Image header's text_offset: well above the 8 KiB resident firmware. */
KERNEL_BASE = 0xFFFFFFFF80010000;
/* The end of the 2 MiB page that holds the kernel, which kmain.rs remaps with 4 KiB pages. */
KERNEL_LIMIT = 0xFFFFFFFF80200000;
GUARD_SIZE = 16K; /* unmapped, see kmain.rs; larger than musl's largest frame (8 KiB) */
STACK_SIZE = 128K;
TRAP_STACK_SIZE = 2K;

SECTIONS {
    . = KERNEL_BASE;
    _image_start = .;

    .text : {
        KEEP(*(.text.head))
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.* .srodata .srodata.*)
    }

    .data : {
        *(.data .data.* .sdata .sdata.*)
    }

    .bss (NOLOAD) : {
        . = ALIGN(16);
        _bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(16);
        _bss_end = .;
        . = ALIGN(4096);
        _kernel_page_tables = .; /* two, filled in by kmain.rs */
        . += 8K;
        _stack_guard = .;
        . += GUARD_SIZE;
        _stack_bottom = .;
        . += STACK_SIZE;
        _stack_top = .;
        . += TRAP_STACK_SIZE;
        _trap_stack_top = .;
    }

    . = ALIGN(4096);
    _image_end = .;

    ASSERT(_image_end <= KERNEL_LIMIT, "the kernel must fit the 2 MiB page that kmain.rs remaps")

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr .note .note.* .comment .riscv.attributes)
    }
}
