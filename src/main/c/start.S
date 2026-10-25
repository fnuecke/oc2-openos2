/* SPDX-License-Identifier: MIT */
/*
 * Entry with a RISC-V Linux Image header, so the firmware loads us at RAM base + text_offset and
 * enters in S-mode with a0 = hart id and a1 = DTB. boot.c turns on paging, then this continues at
 * the kernel's virtual address.
 */

	.section .text.head
	.globl _start
_start:
	.option	push
	.option	norvc
	j	entry
	.option	pop
	.word	0
	.dword	0x10000			/* text_offset: 64 KiB above RAM base, see kernel.ld */
	.dword	_image_end - _start	/* image_size, including bss */
	.dword	0			/* flags */
	.word	2			/* version */
	.word	0
	.dword	0
	.ascii	"RISCV\0\0\0"
	.ascii	"RSC\x05"
	.word	0

entry:
	/* Physical addresses until paging is on, so PC-relative addressing only. */
	mv	s0, a1
	lla	sp, _stack_top
	call	boot_paging

	lui	t0, %hi(virtual_entry)
	addi	t0, t0, %lo(virtual_entry)
	jr	t0

virtual_entry:
	la	sp, _stack_top
	li	t0, 1 << 13		/* sstatus.FS = initial, before any C code may use the FPU */
	csrs	sstatus, t0
	fscsr	zero
	mv	a0, s0
	call	kmain
3:	wfi
	j	3b

/*
 * Interrupts are handled on the current stack with the full register state saved, since they
 * can arrive in any C code. Exceptions are fatal and reported on a separate stack, so that a
 * stack overflow can still be reported. To avoid clobbering a register in the interrupt case,
 * we swap one out into sscratch and after the type check we swap it back.
 */
#define FRAME_SIZE 512
#define FRAME_FCSR 496

	.align	2
	.globl	trap_entry
trap_entry:
	csrrw	t0, sscratch, t0
	csrr	t0, scause
	bgez	t0, exception
	csrrw	t0, sscratch, t0

	addi	sp, sp, -FRAME_SIZE
	sd	ra, 0*8(sp)
	sd	gp, 1*8(sp)
	sd	tp, 2*8(sp)
	sd	t0, 3*8(sp)
	sd	t1, 4*8(sp)
	sd	t2, 5*8(sp)
	sd	s0, 6*8(sp)
	sd	s1, 7*8(sp)
	sd	a0, 8*8(sp)
	sd	a1, 9*8(sp)
	sd	a2, 10*8(sp)
	sd	a3, 11*8(sp)
	sd	a4, 12*8(sp)
	sd	a5, 13*8(sp)
	sd	a6, 14*8(sp)
	sd	a7, 15*8(sp)
	sd	s2, 16*8(sp)
	sd	s3, 17*8(sp)
	sd	s4, 18*8(sp)
	sd	s5, 19*8(sp)
	sd	s6, 20*8(sp)
	sd	s7, 21*8(sp)
	sd	s8, 22*8(sp)
	sd	s9, 23*8(sp)
	sd	s10, 24*8(sp)
	sd	s11, 25*8(sp)
	sd	t3, 26*8(sp)
	sd	t4, 27*8(sp)
	sd	t5, 28*8(sp)
	sd	t6, 29*8(sp)
	fsd	f0, 30*8(sp)
	fsd	f1, 31*8(sp)
	fsd	f2, 32*8(sp)
	fsd	f3, 33*8(sp)
	fsd	f4, 34*8(sp)
	fsd	f5, 35*8(sp)
	fsd	f6, 36*8(sp)
	fsd	f7, 37*8(sp)
	fsd	f8, 38*8(sp)
	fsd	f9, 39*8(sp)
	fsd	f10, 40*8(sp)
	fsd	f11, 41*8(sp)
	fsd	f12, 42*8(sp)
	fsd	f13, 43*8(sp)
	fsd	f14, 44*8(sp)
	fsd	f15, 45*8(sp)
	fsd	f16, 46*8(sp)
	fsd	f17, 47*8(sp)
	fsd	f18, 48*8(sp)
	fsd	f19, 49*8(sp)
	fsd	f20, 50*8(sp)
	fsd	f21, 51*8(sp)
	fsd	f22, 52*8(sp)
	fsd	f23, 53*8(sp)
	fsd	f24, 54*8(sp)
	fsd	f25, 55*8(sp)
	fsd	f26, 56*8(sp)
	fsd	f27, 57*8(sp)
	fsd	f28, 58*8(sp)
	fsd	f29, 59*8(sp)
	fsd	f30, 60*8(sp)
	fsd	f31, 61*8(sp)
	frcsr	t0
	sd	t0, FRAME_FCSR(sp)
	csrr	a0, scause
	call	handle_interrupt
	ld	t0, FRAME_FCSR(sp)
	fscsr	t0
	fld	f0, 30*8(sp)
	fld	f1, 31*8(sp)
	fld	f2, 32*8(sp)
	fld	f3, 33*8(sp)
	fld	f4, 34*8(sp)
	fld	f5, 35*8(sp)
	fld	f6, 36*8(sp)
	fld	f7, 37*8(sp)
	fld	f8, 38*8(sp)
	fld	f9, 39*8(sp)
	fld	f10, 40*8(sp)
	fld	f11, 41*8(sp)
	fld	f12, 42*8(sp)
	fld	f13, 43*8(sp)
	fld	f14, 44*8(sp)
	fld	f15, 45*8(sp)
	fld	f16, 46*8(sp)
	fld	f17, 47*8(sp)
	fld	f18, 48*8(sp)
	fld	f19, 49*8(sp)
	fld	f20, 50*8(sp)
	fld	f21, 51*8(sp)
	fld	f22, 52*8(sp)
	fld	f23, 53*8(sp)
	fld	f24, 54*8(sp)
	fld	f25, 55*8(sp)
	fld	f26, 56*8(sp)
	fld	f27, 57*8(sp)
	fld	f28, 58*8(sp)
	fld	f29, 59*8(sp)
	fld	f30, 60*8(sp)
	fld	f31, 61*8(sp)
	ld	ra, 0*8(sp)
	ld	gp, 1*8(sp)
	ld	tp, 2*8(sp)
	ld	t0, 3*8(sp)
	ld	t1, 4*8(sp)
	ld	t2, 5*8(sp)
	ld	s0, 6*8(sp)
	ld	s1, 7*8(sp)
	ld	a0, 8*8(sp)
	ld	a1, 9*8(sp)
	ld	a2, 10*8(sp)
	ld	a3, 11*8(sp)
	ld	a4, 12*8(sp)
	ld	a5, 13*8(sp)
	ld	a6, 14*8(sp)
	ld	a7, 15*8(sp)
	ld	s2, 16*8(sp)
	ld	s3, 17*8(sp)
	ld	s4, 18*8(sp)
	ld	s5, 19*8(sp)
	ld	s6, 20*8(sp)
	ld	s7, 21*8(sp)
	ld	s8, 22*8(sp)
	ld	s9, 23*8(sp)
	ld	s10, 24*8(sp)
	ld	s11, 25*8(sp)
	ld	t3, 26*8(sp)
	ld	t4, 27*8(sp)
	ld	t5, 28*8(sp)
	ld	t6, 29*8(sp)
	addi	sp, sp, FRAME_SIZE
	sret

exception:
	csrrw	t0, sscratch, t0
	csrw	sscratch, sp
	la	sp, _trap_stack_top
	csrr	a0, scause
	csrr	a1, sepc
	csrr	a2, stval
	csrr	a3, sscratch
	mv	a4, ra
	call	trap_report
4:	wfi
	j	4b
