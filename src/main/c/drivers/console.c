/* SPDX-License-Identifier: MIT */
/* The 16550 UART console. Its receive interrupt comes through the PLIC. */

#include <stdarg.h>
#include <stdio.h>
#include "console.h"
#include "internal.h"
#include "plic.h"
#include "uart.h"

#define RX_SIZE 256
#define CTRL_C 0x03

static char rx[RX_SIZE];
static volatile u32 head, tail; /* interrupt handler writes head, console_read tail */

static interrupt_handler console_handler;
static volatile u32 interrupts;
static volatile int flush_pending;
static volatile u32 flush_until; /* input before this was typed ahead of a Ctrl+C */

static void drain(void) {
    if (board.uart == 0) {
        return;
    }
    while (mmio_read8(board.uart + UART_LSR) & LSR_DR) {
        if (head - tail >= RX_SIZE) {
            /* Full: the UART holds the rest, and counts what it drops as an overrun. */
            mmio_write8(board.uart + UART_IER, 0);
            return;
        }
        const char c = (char) mmio_read8(board.uart + UART_RBR);
        entropy_add(time_now() ^ (u8) c);
        if (c == CTRL_C && console_handler != NULL) {
            /* Like a tty, Ctrl+C drops what was typed ahead of it; console_read applies this. */
            flush_until = head;
            flush_pending = 1;
            wait_wake();
            interrupts++;
            console_handler();
        } else {
            rx[head % RX_SIZE] = c;
            head++;
        }
    }
}

void console_init(void) {
    if (board.uart == 0) {
        return;
    }
    mmio_write8(board.uart + UART_LCR, LCR_8N1);
    drain(); /* enabling the FIFO clears it, keep what was typed during boot */
    mmio_write8(board.uart + UART_FCR, 0x07);
    plic_enable(board.uart_irq);
    mmio_write8(board.uart + UART_IER, IER_RX);
}

bool console_interrupt(const u32 irq) {
    if (board.uart == 0 || irq != board.uart_irq) {
        return 0;
    }
    drain();
    return 1;
}

void kprintf(const char *format, ...) {
    char buffer[256];
    va_list args;
    va_start(args, format);
    const int len = vsnprintf(buffer, sizeof(buffer), format, args);
    va_end(args);
    console_write(buffer, len < (int) sizeof(buffer) ? (size_t) len : sizeof(buffer) - 1);
}

static void put(const char c) {
    while ((mmio_read8(board.uart + UART_LSR) & LSR_THRE) == 0) {
    }
    mmio_write8(board.uart + UART_THR, (u8) c);
}

void console_write(const char *s, const size_t len) {
    if (board.uart == 0) {
        return;
    }
    for (size_t i = 0; i < len; i++) {
        if (s[i] == '\n') {
            put('\r');
        }
        put(s[i]);
    }
}

void console_puts(const char *s) {
    size_t len = 0;
    while (s[len]) {
        len++;
    }
    console_write(s, len);
}

bool console_has_input(void) {
    return head != (flush_pending ? flush_until : tail);
}

static bool has_input(void *ctx) {
    (void) ctx;
    return console_has_input();
}

/* Input is consumed with interrupts masked, so drain can not move the tail meanwhile. */
static void drop_typed_ahead(void) {
    if (flush_pending) {
        tail = flush_until;
        flush_pending = 0;
    }
}

size_t console_read(char *dst, const size_t max, const u64 deadline) {
    wait_until(has_input, NULL, deadline);
    csr_clear(sstatus, SSTATUS_SIE);
    drop_typed_ahead();
    size_t count = 0;
    while (count < max && tail != head) {
        dst[count++] = rx[tail % RX_SIZE];
        tail++;
    }
    csr_set(sstatus, SSTATUS_SIE);
    if (board.uart != 0) {
        mmio_write8(board.uart + UART_IER, IER_RX);
    }
    return count;
}

void console_set_interrupt_handler(const interrupt_handler handler) {
    console_handler = handler;
}

void console_clear_interrupt(void) {
    wait_clear_wake();
}

u32 console_interrupts(void) {
    return interrupts;
}
