/* SPDX-License-Identifier: MIT */
/* Goldfish real-time clocks. */
#ifndef RTC_H
#define RTC_H

#include "kernel.h"

/* ---------------------------------------------------------------------------------------- */
/* For the kernel */

u64 rtc_read(int index);
u64 rtc_seconds(void);

/* ---------------------------------------------------------------------------------------- */
/* For the application */

/* Sets *seconds to the seconds since 1970 of the real-time clock at index. Returns 0 without one. */
bool rtc_seconds_of(int index, u64 *seconds);

#endif
