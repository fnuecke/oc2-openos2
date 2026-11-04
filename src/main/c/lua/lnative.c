/* SPDX-License-Identifier: MIT */

#include <string.h>
#include "lauxlib.h"
#include "lua.h"

#define MAX_ARGUMENTS 8

typedef struct {
    u64 a0, a1;
} results;

typedef results (*entry)(u64, u64, u64, u64, u64, u64, u64, u64);

static int l_call(lua_State *L) {
    const int n = lua_gettop(L);
    luaL_argcheck(L, n <= MAX_ARGUMENTS, MAX_ARGUMENTS + 1, "too many arguments");
    u64 a[MAX_ARGUMENTS] = {0};
    for (int i = 1; i <= n; i++) {
        a[i - 1] = lua_type(L, i) == LUA_TSTRING ? (u64) lua_tostring(L, i) : (u64) luaL_checkinteger(L, i);
    }
    const entry code = (entry) lua_touserdata(L, lua_upvalueindex(1));
    const results r = code(a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
    lua_pushinteger(L, (lua_Integer) r.a0);
    lua_pushinteger(L, (lua_Integer) r.a1);
    return 2;
}

static int l_load(lua_State *L) {
    size_t len;
    const char *bytes = luaL_checklstring(L, 1, &len);
    luaL_argcheck(L, len > 0 && len % 2 == 0, 1, "invalid instruction data");
    memcpy(lua_newuserdatauv(L, len, 0), bytes, len);
    lua_pushcclosure(L, l_call, 1);
    return 1;
}

static const luaL_Reg native_lib[] = {
    {"load", l_load},
    {NULL, NULL},
};

int luaopen_native(lua_State *L) {
    luaL_newlib(L, native_lib);
    return 1;
}
