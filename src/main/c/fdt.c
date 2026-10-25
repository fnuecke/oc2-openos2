/* SPDX-License-Identifier: MIT */
/* Flattened device tree scan for the devices this kernel drives. */

#include <stdlib.h>
#include <string.h>
#include "internal.h"

#define FDT_MAGIC 0xD00DFEED
#define FDT_BEGIN_NODE 1
#define FDT_END_NODE 2
#define FDT_PROP 3
#define FDT_NOP 4
#define FDT_END 9

#define MAX_DEPTH 8
#define MAX_SYSCONS 4
#define MAX_UARTS (MAX_SERIAL + 1)

enum {
    KIND_NONE,
    KIND_MEMORY,
    KIND_UART,
    KIND_PLIC,
    KIND_RTC,
    KIND_VIRTIO,
    KIND_SYSCON,
    KIND_POWEROFF,
    KIND_REBOOT,
    KIND_RESERVED,
};

struct node {
    int kind;
    int address_cells; /* for children */
    int size_cells;
    u64 reg;
    u64 size;
    int has_reg;
    u32 irq;
    u32 phandle;
    u32 regmap;
    u32 offset;
    u32 value;
    int plic_context;
    int in_reserved; /* below /reserved-memory */
    int is_chosen;
    int is_sedna_syscon; /* has host power requests, see drivers/syscon.c */
    u32 clock;
};

struct syscon {
    u32 phandle;
    u64 reg;
};

struct board board;

static struct syscon syscons[MAX_SYSCONS];
static int syscon_count;
static struct {
    u64 reg;
    u32 irq, clock;
} uarts[MAX_UARTS];
static int uart_count;
static u64 stdout_reg; /* the unit address in /chosen's stdout-path, 0 if none */
static struct {
    u32 regmap, offset, value;
    int present;
} poweroff, reboot_info;

static u32 be32(const void *p) {
    const u8 *b = p;
    return ((u32) b[0] << 24) | ((u32) b[1] << 16) | ((u32) b[2] << 8) | b[3];
}

static u64 read_cells(const u8 *p, const int cells) {
    u64 value = 0;
    for (int i = 0; i < cells; i++) {
        value = (value << 32) | be32(p + 4 * i);
    }
    return value;
}

static bool has_string(const char *list, const u32 len, const char *needle) {
    for (u32 i = 0; i < len; i += strlen(list + i) + 1) {
        if (strcmp(list + i, needle) == 0) {
            return 1;
        }
    }
    return 0;
}

static int classify(const char *list, const u32 len) {
    if (has_string(list, len, "virtio,mmio")) return KIND_VIRTIO;
    if (has_string(list, len, "ns16550a") || has_string(list, len, "ns16550")) return KIND_UART;
    if (has_string(list, len, "riscv,plic0") || has_string(list, len, "sifive,plic-1.0.0")) return KIND_PLIC;
    if (has_string(list, len, "google,goldfish-rtc")) return KIND_RTC;
    if (has_string(list, len, "syscon-poweroff")) return KIND_POWEROFF;
    if (has_string(list, len, "syscon-reboot")) return KIND_REBOOT;
    if (has_string(list, len, "syscon")) return KIND_SYSCON;
    return KIND_NONE;
}

static void commit(const struct node *node) {
    switch (node->kind) {
        case KIND_MEMORY:
            if (node->has_reg && board.ram_size == 0) {
                board.ram_base = node->reg;
                board.ram_size = node->size;
            }
            if (node->has_reg) {
                board.ram_total += node->size;
                if (board.ram_region_count < MAX_RAM) {
                    board.ram_region_base[board.ram_region_count] = node->reg;
                    board.ram_region_size[board.ram_region_count] = node->size;
                    board.ram_region_count++;
                }
            }
            break;
        case KIND_UART:
            if (node->has_reg && uart_count < MAX_UARTS) {
                uarts[uart_count].reg = node->reg;
                uarts[uart_count].irq = node->irq;
                uarts[uart_count].clock = node->clock;
                uart_count++;
            }
            break;
        case KIND_PLIC:
            if (node->has_reg && board.plic == 0) {
                board.plic = node->reg;
                board.plic_context = node->plic_context;
            }
            break;
        case KIND_RTC:
            if (node->has_reg && board.rtc_count < MAX_RTC) {
                board.rtc[board.rtc_count++] = node->reg;
            }
            break;
        case KIND_VIRTIO:
            if (node->has_reg && board.virtio_count < MAX_VIRTIO) {
                board.virtio[board.virtio_count] = node->reg;
                board.virtio_irq[board.virtio_count] = node->irq;
                board.virtio_count++;
            }
            break;
        case KIND_SYSCON:
            if (node->has_reg && syscon_count < MAX_SYSCONS) {
                syscons[syscon_count].phandle = node->phandle;
                syscons[syscon_count].reg = node->reg;
                syscon_count++;
            }
            if (node->has_reg && node->is_sedna_syscon && board.syscon == 0) {
                board.syscon = node->reg;
                board.syscon_irq = node->irq;
            }
            break;
        case KIND_POWEROFF:
            poweroff.regmap = node->regmap;
            poweroff.offset = node->offset;
            poweroff.value = node->value;
            poweroff.present = 1;
            break;
        case KIND_REBOOT:
            reboot_info.regmap = node->regmap;
            reboot_info.offset = node->offset;
            reboot_info.value = node->value;
            reboot_info.present = 1;
            break;
        case KIND_RESERVED:
            if (node->has_reg && board.reserved_size == 0) {
                board.reserved_base = node->reg;
                board.reserved_size = node->size;
            }
            break;
    }
}

static u64 syscon_address(const u32 phandle) {
    for (int i = 0; i < syscon_count; i++) {
        if (syscons[i].phandle == phandle) {
            return syscons[i].reg;
        }
    }
    return 0;
}

static void prop(struct node *node, const struct node *parent, const char *name, const u8 *value, const u32 len) {
    if (strcmp(name, "reg") == 0) {
        const int cells = parent->address_cells + parent->size_cells;
        if (len >= (u32) cells * 4) {
            node->reg = read_cells(value, parent->address_cells);
            node->size = read_cells(value + 4 * parent->address_cells, parent->size_cells);
            node->has_reg = 1;
        }
    } else if (strcmp(name, "compatible") == 0) {
        node->kind = classify((const char *) value, len);
        node->is_sedna_syscon = has_string((const char *) value, len, "sedna,syscon");
    } else if (strcmp(name, "device_type") == 0 && len >= 7 && memcmp(value, "memory", 7) == 0) {
        node->kind = KIND_MEMORY;
    } else if (strcmp(name, "#address-cells") == 0 && len == 4) {
        node->address_cells = (int) be32(value);
    } else if (strcmp(name, "#size-cells") == 0 && len == 4) {
        node->size_cells = (int) be32(value);
    } else if (strcmp(name, "interrupts") == 0 && len >= 4) {
        node->irq = be32(value);
    } else if (strcmp(name, "phandle") == 0 && len == 4) {
        node->phandle = be32(value);
    } else if (strcmp(name, "regmap") == 0 && len == 4) {
        node->regmap = be32(value);
    } else if (strcmp(name, "offset") == 0 && len == 4) {
        node->offset = be32(value);
    } else if (strcmp(name, "value") == 0 && len == 4) {
        node->value = be32(value);
    } else if (strcmp(name, "interrupts-extended") == 0) {
        /* Pairs of (interrupt controller, interrupt). For devices, the first is the PLIC source. */
        if (len >= 8) {
            node->irq = be32(value + 4);
        }
        /* For the PLIC, the pairs are per context, cause 9 is the S-mode external interrupt. */
        for (u32 i = 0; i + 8 <= len; i += 8) {
            if (be32(value + i + 4) == 9) {
                node->plic_context = (int) (i / 8);
                break;
            }
        }
    } else if (strcmp(name, "timebase-frequency") == 0 && len == 4) {
        board.timebase = be32(value);
    } else if (strcmp(name, "clock-frequency") == 0 && len == 4) {
        node->clock = be32(value);
    } else if (node->is_chosen && strcmp(name, "stdout-path") == 0) {
        const char *at = memchr(value, '@', len);
        if (at != NULL) {
            stdout_reg = strtoull(at + 1, NULL, 16);
        }
    }
}

/* The console is the UART stdout-path names, else the first one. The others are serial ports. */
static void assign_uarts(void) {
    int console = 0;
    for (int i = 0; i < uart_count; i++) {
        if (stdout_reg != 0 && uarts[i].reg == stdout_reg) {
            console = i;
        }
    }
    for (int i = 0; i < uart_count; i++) {
        if (i == console) {
            board.uart = uarts[i].reg;
            board.uart_irq = uarts[i].irq;
        } else if (board.serial_count < MAX_SERIAL) {
            board.serial[board.serial_count] = uarts[i].reg;
            board.serial_irq[board.serial_count] = uarts[i].irq;
            board.serial_clock[board.serial_count] = uarts[i].clock;
            board.serial_count++;
        }
    }
}

int fdt_scan(void *dtb) {
    if (be32(dtb) != FDT_MAGIC) {
        return -1;
    }
    board.dtb = dtb;
    board.dtb_size = be32((u8 *) dtb + 4);
    board.plic_context = -1;

    const u8 *structure = (const u8 *) dtb + be32((u8 *) dtb + 8);
    const char *strings = (const char *) dtb + be32((u8 *) dtb + 12);

    struct node stack[MAX_DEPTH + 1];
    memset(&stack[0], 0, sizeof(stack[0]));
    stack[0].address_cells = 2;
    stack[0].size_cells = 1;
    int depth = 0; /* stack[depth] is the node being read, stack[depth - 1] its parent */

    u32 offset = 0;
    for (;;) {
        const u32 token = be32(structure + offset);
        offset += 4;
        switch (token) {
            case FDT_BEGIN_NODE: {
                const char *name = (const char *) structure + offset;
                const u32 length = (u32) strlen(name);
                offset += (length + 4) & ~3u;
                if (depth >= MAX_DEPTH) {
                    return -1;
                }
                depth++;
                struct node *node = &stack[depth];
                memset(node, 0, sizeof(*node));
                node->address_cells = 2;
                node->size_cells = 1;
                node->plic_context = -1;
                node->in_reserved = stack[depth - 1].in_reserved || strcmp(name, "reserved-memory") == 0;
                node->is_chosen = depth == 2 && strcmp(name, "chosen") == 0; /* depth 1 is the root */
                if (stack[depth - 1].in_reserved) {
                    node->kind = KIND_RESERVED;
                }
                break;
            }
            case FDT_END_NODE:
                if (depth > 1) {
                    commit(&stack[depth]);
                }
                depth--;
                break;
            case FDT_PROP: {
                const u32 length = be32(structure + offset);
                const char *name = strings + be32(structure + offset + 4);
                const u8 *value = structure + offset + 8;
                offset += 8 + ((length + 3) & ~3u);
                if (depth > 0) {
                    const int kind = stack[depth].kind;
                    prop(&stack[depth], &stack[depth - 1], name, value, length);
                    if (kind == KIND_RESERVED) {
                        stack[depth].kind = KIND_RESERVED;
                    }
                }
                break;
            }
            case FDT_NOP:
                break;
            default:
                assign_uarts();
                if (poweroff.present) {
                    const u64 base = syscon_address(poweroff.regmap);
                    if (base != 0) {
                        board.poweroff_addr = base + poweroff.offset;
                        board.poweroff_value = poweroff.value;
                    }
                }
                if (reboot_info.present) {
                    const u64 base = syscon_address(reboot_info.regmap);
                    if (base != 0) {
                        board.reboot_addr = base + reboot_info.offset;
                        board.reboot_value = reboot_info.value;
                    }
                }
                return board.ram_size != 0 ? 0 : -1;
        }
    }
}
