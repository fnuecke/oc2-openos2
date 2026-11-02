# SPDX-License-Identifier: MIT
# Builds a C application on top of the kernel. See examples/Makefile.
#
# The application must provide app_main(); the kernel powers off when it returns.
# Threads and system calls aren't a thing. Waiting works through with event_count()/event_wait().
# Interrupt handlers run in the trap handler and may only set flags, never longjmp.
#
# Notable differences from a regular C environment:
# - stdin uses the raw console, meaning there's no echo and no line editing.
# - With a console interrupt handler set, Ctrl+C drops typed input. The application has to call
#   console_clear_interrupt() and clearerr(stdin) to resume reading (will get EOF until then).
# - Time is UTC. strftime would read the thread pointer (which we don't have), so use strftime_l with musl's C locale.
# - mkstemp descriptor has to be closed with fs_close().
# - Not supported: locales, wide character I/O, signals, threads, popen, fopen mode "x", freopen
#   without a path, and anything else that needs a system call. These fail to link, or link code
#   that traps.

ifndef KERNEL
KERNEL := $(patsubst %/,%,$(dir $(lastword $(MAKEFILE_LIST))))
endif
MUSL ?= /opt/musl

KERNEL_ARCH    := -march=rv64imafdc_zicsr_zifencei -mabi=lp64d -mcmodel=medlow
KERNEL_CFLAGS  := $(KERNEL_ARCH) -nostdinc -isystem $(MUSL)/include -isystem $(shell $(CC) -print-file-name=include) \
                  -I$(KERNEL)/include -fno-pie -fno-stack-protector -fno-asynchronous-unwind-tables \
                  -ffunction-sections -fdata-sections
KERNEL_LDFLAGS := $(KERNEL_ARCH) -static -nostdlib -no-pie -Wl,--gc-sections -Wl,--build-id=none -Wl,-u,_start \
                  -T $(KERNEL)/lib/kernel.ld
KERNEL_LIBS    := $(KERNEL)/lib/libkernel.a $(MUSL)/lib/libc.a -lgcc
