/* SPDX-License-Identifier: MIT */
/* Sedna's system controller: power requests from the host, power off and reboot. */
#ifndef SYSCON_H
#define SYSCON_H

#include "kernel.h"

void syscon_init(void);
bool syscon_interrupt(u32 irq);
void syscon_power_off(void);
void syscon_reboot(void);

#endif
