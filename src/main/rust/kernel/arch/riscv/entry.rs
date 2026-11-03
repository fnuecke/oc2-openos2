// SPDX-License-Identifier: MIT
//! Entry with a RISC-V Linux Image header, for firmware compat.
//!
//! Interrupts are handled on the current stack with the full register state saved.
//! Exceptions are fatal and reported on a separate stack, so that a stack overflow
//! can still be reported. To avoid clobbering a register in the interrupt case,
//! one is swapped out into sscratch and after the type check swapped back.

core::arch::global_asm!(
    r#"
    .option arch, +d
    .section .text.head, "ax", @progbits
    .globl  _start
_start:
    .option push
    .option norvc
    j       entry
    .option pop
    .word   0
    .dword  0x10000             /* text_offset: 64 KiB above RAM base, see kernel.ld */
    .dword  _image_end - _start /* image_size, including bss */
    .dword  0                   /* flags */
    .word   2                   /* version */
    .word   0
    .dword  0
    .ascii  "RISCV\0\0\0"
    .ascii  "RSC\x05"
    .word   0

entry:
    /* Physical addresses until paging is on, so PC-relative addressing only. */
    mv      s0, a1
    lla     sp, _stack_top
    call    boot_paging

    lui     t0, %hi(virtual_entry)
    addi    t0, t0, %lo(virtual_entry)
    jr      t0

virtual_entry:
    lla     sp, _stack_top
    li      t0, 1 << 13         /* sstatus.FS = initial, before any code may use the FPU */
    csrs    sstatus, t0
    fscsr   zero
    lla     t0, _bss_start
    lla     t1, _bss_end
1:  sd      zero, 0(t0)
    addi    t0, t0, 8
    bltu    t0, t1, 1b
    mv      a0, s0
    call    kmain
3:  wfi
    j       3b

    .equ    FRAME_SIZE, 512
    .equ    FRAME_FCSR, 496

    .text
    .align  2
    .globl  trap_entry
trap_entry:
    csrrw   t0, sscratch, t0
    csrr    t0, scause
    bgez    t0, exception
    csrrw   t0, sscratch, t0

    addi    sp, sp, -FRAME_SIZE
    .set    slot, 0
    .irp    r, ra, gp, tp, t0, t1, t2, s0, s1, a0, a1, a2, a3, a4, a5, a6, a7, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11, t3, t4, t5, t6
    sd      \r, slot * 8(sp)
    .set    slot, slot + 1
    .endr
    .irp    n, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31
    fsd     f\n, (30 + \n) * 8(sp)
    .endr
    frcsr   t0
    sd      t0, FRAME_FCSR(sp)
    csrr    a0, scause
    call    handle_interrupt
    ld      t0, FRAME_FCSR(sp)
    fscsr   t0
    .irp    n, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31
    fld     f\n, (30 + \n) * 8(sp)
    .endr
    .set    slot, 0
    .irp    r, ra, gp, tp, t0, t1, t2, s0, s1, a0, a1, a2, a3, a4, a5, a6, a7, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11, t3, t4, t5, t6
    ld      \r, slot * 8(sp)
    .set    slot, slot + 1
    .endr
    addi    sp, sp, FRAME_SIZE
    sret

exception:
    csrrw   t0, sscratch, t0
    csrw    sscratch, sp
    lla     sp, _trap_stack_top
    csrr    a0, scause
    csrr    a1, sepc
    csrr    a2, stval
    csrr    a3, sscratch
    mv      a4, ra
    call    trap_report
4:  wfi
    j       4b
"#
);

unsafe extern "C" {
    pub fn trap_entry();
}
