/* SPDX-License-Identifier: MIT */
/* Goldfish real-time clocks. */

#include "internal.h"
#include "rtc.h"

u64 rtc_read(const int index) {
    const u64 rtc = board.rtc[index];
    const u64 low = mmio_read32(rtc);
    const u64 high = mmio_read32(rtc + 4);
    return (high << 32) | low;
}

u64 rtc_seconds(void) {
    return board.rtc_count > 0 ? rtc_read(0) / 1000000000ull : 0;
}

bool rtc_seconds_of(const int index, u64 *seconds) {
    if (index < 0 || index >= board.rtc_count) {
        return 0;
    }
    *seconds = rtc_read(index) / 1000000000ull;
    return 1;
}
