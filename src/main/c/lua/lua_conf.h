/* SPDX-License-Identifier: MIT */
#ifndef LUA_CONF_H
#define LUA_CONF_H

#include "kernel.h"

typedef uint8_t u8;
typedef uint16_t u16;
typedef uint32_t u32;
typedef uint64_t u64;

/* 100 nested C calls (e.g. pcall) peak at 94 KiB of the 128 KiB stack. */
#define LUAI_MAXCCALLS 100

#define lua_getlocaledecpoint() '.'
#define LUA_PATH_DEFAULT "/lib/?.lua;/lib/?/init.lua;/mnt/builtin/lib/lua/?.lua;/mnt/builtin/lib/lua/?/init.lua"
#define LUA_CPATH_DEFAULT ""
#define lua_writestring(s, l) console_write((s), (l))
#define lua_writeline() console_write("\n", 1)
#define lua_writestringerror(s, p) kprintf((s), (p))

/* loslib: patches to avoid syscalls/thread pointer accesses. */
#include <locale.h>
#include <time.h>
#define l_system(cmd) ((cmd) == NULL ? 0 : -1)
#define lua_tmpnam(b, e) { strcpy(b, "/tmp/lua_XXXXXX"); e = mkstemp(b); if (e != -1) fs_close(e); e = (e == -1); }
#define LUA_TMPNAMBUFSIZE 32
#define setlocale(c, l) ((void) (c), (l) == NULL || *(l) == 0 || strcmp((l), "C") == 0 ? "C" : NULL)
#define l_gmtime(t, r) gmtime_r(t, r)
#define l_localtime(t, r) gmtime_r(t, r)
#define mktime timegm
extern const struct __locale_struct __c_locale;
#define strftime(s, n, f, t) strftime_l(s, n, f, t, (locale_t) &__c_locale)

#endif
