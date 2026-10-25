/* SPDX-License-Identifier: MIT */
/*
 * 16550 UARTs other than the console, e.g. oc2's serial interface cards, as /dev/ttyS1 and up.
 * Received bytes go into a ring buffer from the interrupt handler, sending fills the transmit FIFO.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "internal.h"
#include "plic.h"
#include "serial.h"
#include "uart.h"

#define FIFO_SIZE 16
#define RX_SIZE 1024
#define DEFAULT_BAUD 9600

struct port {
    u64 base;
    u32 irq;
    u32 clock;
    u32 baud;
    char *rx;
    volatile u32 head, tail; /* interrupt handler writes head, readers tail */
    int notified; /* reported by serial_take_signal since the last read */
    u8 ier;
    char name[16];
};

static struct port ports[MAX_SERIAL];
static int port_count;

static void set_ier(struct port *port, const u8 ier) {
    port->ier = ier;
    mmio_write8(port->base + UART_IER, ier);
}

static void drain(struct port *port) {
    while (mmio_read8(port->base + UART_LSR) & LSR_DR) {
        if (port->head - port->tail >= RX_SIZE) {
            /* Full: the UART holds the rest, and counts what it drops as an overrun. */
            set_ier(port, port->ier & ~IER_RX);
            return;
        }
        port->rx[port->head % RX_SIZE] = (char) mmio_read8(port->base + UART_RBR);
        port->head++;
    }
}

void serial_init(void) {
    for (int i = 0; i < board.serial_count; i++) {
        char *rx = malloc(RX_SIZE);
        if (rx == NULL) {
            break;
        }
        struct port *port = &ports[port_count];
        port->base = board.serial[i];
        port->irq = board.serial_irq[i];
        port->clock = board.serial_clock[i];
        port->rx = rx;
        snprintf(port->name, sizeof(port->name), "ttyS%d", port_count + 1);
        set_ier(port, 0);
        serial_set_baud(port_count, DEFAULT_BAUD);
        mmio_write8(port->base + UART_FCR, 0x07);
        plic_enable(port->irq);
        set_ier(port, IER_RX);
        port_count++;
    }
}

bool serial_interrupt(const u32 irq) {
    for (int i = 0; i < port_count; i++) {
        struct port *port = &ports[i];
        if (port->irq == irq) {
            drain(port);
            /* A writer waits for the transmit FIFO to empty, see serial_write. */
            if ((port->ier & IER_TX) && (mmio_read8(port->base + UART_LSR) & LSR_THRE)) {
                set_ier(port, port->ier & ~IER_TX);
            }
            return 1;
        }
    }
    return 0;
}

int serial_count(void) {
    return port_count;
}

int serial_find(const char *name) {
    for (int i = 0; i < port_count; i++) {
        if (strcmp(ports[i].name, name) == 0) {
            return i;
        }
    }
    return -1;
}

const char *serial_name(const int index) {
    return ports[index].name;
}

u64 serial_base(const int index) {
    return ports[index].base;
}

int serial_set_baud(const int index, const u32 baud) {
    const struct port *port = &ports[index];
    if (baud == 0 || port->clock == 0 || baud > port->clock / 16) {
        return -1;
    }
    u32 divisor = port->clock / (16 * baud);
    divisor = divisor > 0xFFFF ? 0xFFFF : divisor;
    const u64 sie = csr_read(sstatus) & SSTATUS_SIE;
    csr_clear(sstatus, SSTATUS_SIE);
    mmio_write8(port->base + UART_LCR, LCR_8N1 | LCR_DLAB);
    mmio_write8(port->base + UART_DLL, (u8) divisor);
    mmio_write8(port->base + UART_DLM, (u8) (divisor >> 8));
    mmio_write8(port->base + UART_LCR, LCR_8N1);
    csr_set(sstatus, sie);
    ports[index].baud = baud;
    return 0;
}

u32 serial_baud(const int index) {
    return ports[index].baud;
}

bool serial_readable(const int index) {
    struct port *port = &ports[index];
    csr_clear(sstatus, SSTATUS_SIE);
    drain(port);
    const int readable = port->head != port->tail;
    csr_set(sstatus, SSTATUS_SIE);
    return readable;
}

/* When busy, enables the transmit interrupt until the FIFO empties, so a waiting writer wakes up. */
bool serial_writable(const int index) {
    struct port *port = &ports[index];
    if (mmio_read8(port->base + UART_LSR) & LSR_THRE) {
        return 1;
    }
    const u64 sie = csr_read(sstatus) & SSTATUS_SIE;
    csr_clear(sstatus, SSTATUS_SIE);
    set_ier(port, port->ier | IER_TX);
    csr_set(sstatus, sie);
    return 0;
}

size_t serial_read(const int index, char *dst, const size_t max) {
    struct port *port = &ports[index];
    csr_clear(sstatus, SSTATUS_SIE);
    drain(port);
    size_t count = 0;
    while (count < max && port->tail != port->head) {
        dst[count++] = port->rx[port->tail % RX_SIZE];
        port->tail++;
    }
    set_ier(port, port->ier | IER_RX);
    csr_set(sstatus, SSTATUS_SIE);
    port->notified = 0;
    return count;
}

/* Fills the transmit FIFO if it is empty. Returns how many bytes were accepted, 0 if it is busy. */
size_t serial_write(const int index, const char *src, const size_t len) {
    const struct port *port = &ports[index];
    if (!serial_writable(index)) {
        return 0;
    }
    const size_t count = len < FIFO_SIZE ? len : FIFO_SIZE;
    for (size_t i = 0; i < count; i++) {
        mmio_write8(port->base + UART_THR, (u8) src[i]);
    }
    return count;
}

bool serial_signal_ready(void) {
    for (int i = 0; i < port_count; i++) {
        if (!ports[i].notified && ports[i].head != ports[i].tail) {
            return 1;
        }
    }
    return 0;
}

int serial_take_signal(void) {
    for (int i = 0; i < port_count; i++) {
        if (!ports[i].notified && ports[i].head != ports[i].tail) {
            ports[i].notified = 1;
            return i;
        }
    }
    return -1;
}
