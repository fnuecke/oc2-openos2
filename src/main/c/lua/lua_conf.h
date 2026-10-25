/* SPDX-License-Identifier: MIT */
#ifndef LUA_CONF_H
#define LUA_CONF_H

#include "../kernel.h"

/* From drivers/console.h, not part of our includes for a clean compile split. */
extern void console_write(const char *s, size_t len);

/* 100 nested C calls (e.g. pcall) peak at 94 KiB of the 128 KiB stack. */
#define LUAI_MAXCCALLS 100

#define lua_getlocaledecpoint() '.'
#define LUA_PATH_DEFAULT "/lib/?.lua;/lib/?/init.lua;/mnt/builtin/lib/lua/?.lua;/mnt/builtin/lib/lua/?/init.lua"
#define LUA_CPATH_DEFAULT ""
#define lua_writestring(s, l) console_write((s), (l))
#define lua_writeline() console_write("\n", 1)
#define lua_writestringerror(s, p) kprintf((s), (p))

#endif
