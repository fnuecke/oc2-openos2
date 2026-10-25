/* SPDX-License-Identifier: MIT */
/* The platform-level interrupt controller (PLIC). */
#ifndef PLIC_H
#define PLIC_H

#include "kernel.h"

void plic_init(void);
void plic_enable(u32 irq);
void plic_dispatch(void);

#endif
