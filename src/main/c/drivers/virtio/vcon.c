/* SPDX-License-Identifier: MIT */
/*
 * virtio-console in multiport mode, used for the named ports of the oc2 device bus. The port
 * handshake runs on the control queues: the driver announces DEVICE_READY, the device adds each
 * port, the driver acknowledges with PORT_READY, the device sends the port's name and opens it.
 * Ports are looked up by name, since their numbering is not part of the contract.
 */

#include <stdlib.h>
#include <string.h>
#include "drivers/plic.h"
#include "fs/sysfs.h"
#include "internal.h"
#include "virtio.h"

#define F_MULTIPORT (1u << 1)
#define CONFIG_MAX_PORTS 4

#define CONTROL_RECEIVE 2
#define CONTROL_TRANSMIT 3

#define DEVICE_READY 0
#define DEVICE_ADD 1
#define PORT_READY 3
#define PORT_OPEN 6
#define PORT_NAME 7

#define QUEUE_SIZE 4
#define CONTROL_SIZE 8
#define CONTROL_BUFFER (CONTROL_SIZE + 64)
#define RECEIVE_BUFFERS 2
#define RECEIVE_BUFFER 1024
#define MAX_PORTS 8
#define MAX_NAME 63

struct control {
    u32 id;
    u16 event;
    u16 value;
};

struct port {
    char name[MAX_NAME + 1];
    int named;
    int opened; /* receive buffers are allocated on first open */
    int notified; /* reported by vcon_take_signal since the last read */
    int blocked; /* a send found no free descriptor */
    struct virtq rx, tx;
    u8 *rx_buffers[QUEUE_SIZE]; /* by descriptor head */
    int current; /* head of the receive buffer being consumed, or -1 */
    u32 current_length, current_offset;
};

static struct {
    u64 base;
    u32 irq;
    u32 port_count;
    struct virtq control_rx, control_tx;
    u8 *control_rx_buffers[QUEUE_SIZE];
    struct control control_tx_buffers[QUEUE_SIZE];
    struct port ports[MAX_PORTS];
} con;

static int post_receive(struct virtq *q, u8 *buffer, const u32 size, u8 **by_head) {
    const struct virtq_buf buf = {buffer, size};
    const int head = virtq_add(q, &buf, 0, 1);
    if (head >= 0) {
        by_head[head] = buffer;
    }
    return head;
}

static void send_control(const u32 id, const u16 event, const u16 value) {
    while (virtq_next_used(&con.control_tx, NULL) >= 0) {
    }
    if (con.control_tx.free_count == 0) {
        return;
    }
    const int slot = con.control_tx.free_head;
    struct control *message = &con.control_tx_buffers[slot];
    message->id = id;
    message->event = event;
    message->value = value;
    const struct virtq_buf buf = {message, CONTROL_SIZE};
    if (virtq_add(&con.control_tx, &buf, 1, 0) >= 0) {
        virtq_notify(&con.control_tx);
    }
}

static void pump_control(void) {
    if (con.base == 0) {
        return;
    }
    int head;
    u32 length;
    int reposted = 0;
    while ((head = virtq_next_used(&con.control_rx, &length)) >= 0) {
        u8 *buffer = con.control_rx_buffers[head];
        if (length >= CONTROL_SIZE) {
            struct control message;
            memcpy(&message, buffer, sizeof(message));
            if (message.id < con.port_count) {
                struct port *port = &con.ports[message.id];
                switch (message.event) {
                    case DEVICE_ADD:
                        send_control(message.id, PORT_READY, 1);
                        break;
                    case PORT_NAME: {
                        u32 name_length = length - CONTROL_SIZE;
                        if (name_length > MAX_NAME) {
                            name_length = MAX_NAME;
                        }
                        memcpy(port->name, buffer + CONTROL_SIZE, name_length);
                        port->name[name_length] = 0;
                        port->named = 1;
                        break;
                    }
                }
            }
        }
        post_receive(&con.control_rx, buffer, CONTROL_BUFFER, con.control_rx_buffers);
        reposted = 1;
    }
    if (reposted) {
        virtq_notify(&con.control_rx);
    }
}

static int setup_port(struct port *port, const u32 index) {
    const u16 rx = (u16) (index == 0 ? 0 : 2 * index + 2);
    const u16 tx = (u16) (index == 0 ? 1 : 2 * index + 3);
    port->current = -1;
    return virtq_init(&port->rx, con.base, rx, QUEUE_SIZE) != 0 || virtq_init(&port->tx, con.base, tx, QUEUE_SIZE) != 0 ? -1 : 0;
}

int vcon_init(void) {
    for (int i = 0; i < board.virtio_count; i++) {
        const u64 base = board.virtio[i];
        if (virtio_device_id(base) != VIRTIO_ID_CONSOLE) {
            continue;
        }
        const long features = virtio_begin(base, F_MULTIPORT);
        if (features < 0) {
            continue;
        }
        if (!(features & F_MULTIPORT)) {
            virtio_reset(base);
            continue;
        }
        con.base = base;
        con.irq = board.virtio_irq[i];
        con.port_count = virtio_config32(base, CONFIG_MAX_PORTS);
        if (con.port_count > MAX_PORTS) {
            con.port_count = MAX_PORTS;
        }
        if (virtq_init(&con.control_rx, base, CONTROL_RECEIVE, QUEUE_SIZE) != 0
            || virtq_init(&con.control_tx, base, CONTROL_TRANSMIT, QUEUE_SIZE) != 0) {
            virtio_reset(base);
            con.base = 0;
            return -1;
        }
        for (u32 port = 0; port < con.port_count; port++) {
            if (setup_port(&con.ports[port], port) != 0) {
                virtio_reset(base);
                con.base = 0;
                return -1;
            }
        }
        for (int slot = 0; slot < QUEUE_SIZE; slot++) {
            u8 *buffer = malloc(CONTROL_BUFFER);
            if (buffer == NULL) {
                virtio_reset(base);
                con.base = 0;
                return -1;
            }
            post_receive(&con.control_rx, buffer, CONTROL_BUFFER, con.control_rx_buffers);
        }
        virtio_ready(base);
        plic_enable(con.irq);
        send_control(0, DEVICE_READY, 1);
        return 0;
    }
    return -1;
}

bool vcon_named(const int port) {
    pump_control();
    return port >= 0 && (u32) port < con.port_count && con.ports[port].named;
}

static int find_named(const char *name) {
    for (u32 i = 0; i < con.port_count; i++) {
        if (con.ports[i].named && strcmp(con.ports[i].name, name) == 0) {
            return (int) i;
        }
    }
    return -1;
}

static bool has_control_pending(void *ctx) {
    (void) ctx;
    return *(volatile u16 *) &con.control_rx.used->idx != con.control_rx.last_used;
}

int vcon_find(const char *name, const u64 deadline) {
    if (con.base == 0) {
        return -1;
    }
    const int alias = sysfs_port(name, strlen(name));
    if (alias >= 0) {
        return alias;
    }
    for (;;) {
        pump_control();
        const int port = find_named(name);
        if (port >= 0) {
            return port;
        }
        if (!wait_until(has_control_pending, NULL, deadline)) {
            return -1;
        }
    }
}

static int open_port(struct port *port) {
    if (port->opened) {
        return 0;
    }
    for (int i = 0; i < RECEIVE_BUFFERS; i++) {
        u8 *buffer = malloc(RECEIVE_BUFFER);
        if (buffer == NULL) {
            return -1;
        }
        post_receive(&port->rx, buffer, RECEIVE_BUFFER, port->rx_buffers);
    }
    virtq_notify(&port->rx);
    port->opened = 1;
    return 0;
}

int vcon_open(const char *name, const u64 deadline) {
    const int port = vcon_find(name, deadline);
    if (port >= 0 && !con.ports[port].opened) {
        if (open_port(&con.ports[port]) != 0) {
            return -1;
        }
        send_control((u32) port, PORT_OPEN, 1);
    }
    return port;
}

static bool is_port_readable(void *ctx) {
    struct port *port = ctx;
    return port->current >= 0 || *(volatile u16 *) &port->rx.used->idx != port->rx.last_used;
}

bool vcon_readable(const int index) {
    return is_port_readable(&con.ports[index]);
}

bool vcon_writable(const int index) {
    const struct port *port = &con.ports[index];
    return port->tx.free_count > 0 || *(volatile u16 *) &port->tx.used->idx != port->tx.last_used;
}

long vcon_read(const int index, void *dst, const size_t max, const u64 deadline) {
    struct port *port = &con.ports[index];
    port->notified = 0;
    pump_control();
    if (!wait_until(is_port_readable, port, deadline)) {
        return 0;
    }
    size_t done = 0;
    while (done < max) {
        if (port->current < 0) {
            u32 length;
            const int head = virtq_next_used(&port->rx, &length);
            if (head < 0) {
                break;
            }
            port->current = head;
            port->current_length = length > RECEIVE_BUFFER ? RECEIVE_BUFFER : length;
            port->current_offset = 0;
        }
        u8 *buffer = port->rx_buffers[port->current];
        size_t chunk = port->current_length - port->current_offset;
        if (chunk > max - done) {
            chunk = max - done;
        }
        memcpy((u8 *) dst + done, buffer + port->current_offset, chunk);
        done += chunk;
        port->current_offset += (u32) chunk;
        if (port->current_offset >= port->current_length) {
            port->current = -1;
            post_receive(&port->rx, buffer, RECEIVE_BUFFER, port->rx_buffers);
            virtq_notify(&port->rx);
        }
    }
    return (long) done;
}

int vcon_send(const int index, const void *data, const size_t len) {
    struct port *port = &con.ports[index];
    pump_control();
    const struct virtq_buf buf = {(void *) data, (u32) len};
    const int head = virtq_add(&port->tx, &buf, 1, 0);
    if (head >= 0) {
        virtq_notify(&port->tx);
    } else {
        port->blocked = 1;
    }
    return head;
}

int vcon_reclaim(const int index) {
    return virtq_next_used(&con.ports[index].tx, NULL);
}

void vcon_ports(void (*fn)(void *ctx, int port, const char *name), void *ctx) {
    pump_control();
    for (u32 i = 0; i < con.port_count; i++) {
        if (con.ports[i].named) {
            fn(ctx, (int) i, con.ports[i].name);
        }
    }
}

const char *vcon_name(const int index) {
    return con.ports[index].name;
}

static bool is_signal_due(const struct port *port, const int index) {
    return port->opened && ((!port->notified && is_port_readable((void *) port)) || (port->blocked && vcon_writable(index)));
}

bool vcon_signal_ready(void) {
    for (u32 i = 0; i < con.port_count; i++) {
        if (is_signal_due(&con.ports[i], (int) i)) {
            return 1;
        }
    }
    return 0;
}

int vcon_take_signal(void) {
    for (u32 i = 0; i < con.port_count; i++) {
        struct port *port = &con.ports[i];
        if (is_signal_due(port, (int) i)) {
            port->notified = 1;
            port->blocked = 0;
            return (int) i;
        }
    }
    return -1;
}
