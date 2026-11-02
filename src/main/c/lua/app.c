/* SPDX-License-Identifier: MIT */

#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"
#include "lualib.h"

int luaopen_console(lua_State *L);
int luaopen_computer(lua_State *L);
int luaopen_fs(lua_State *L);
int luaopen_serial(lua_State *L);
int luaopen_cjson(lua_State *L);
int luaopen_posix_errno(lua_State *L);
int luaopen_posix_fcntl(lua_State *L);
int luaopen_posix_unistd(lua_State *L);
int luaopen_posix_poll(lua_State *L);
int luaopen_posix_time(lua_State *L);
int luaopen_posix_signal(lua_State *L);
int luaopen_posix_sys_stat(lua_State *L);
int luaopen_posix_sys_socket(lua_State *L);
int luaopen_posix_dirent(lua_State *L);

static const luaL_Reg libraries[] = {
    {LUA_GNAME, luaopen_base},
    {LUA_LOADLIBNAME, luaopen_package},
    {LUA_COLIBNAME, luaopen_coroutine},
    {LUA_TABLIBNAME, luaopen_table},
    {LUA_STRLIBNAME, luaopen_string},
    {LUA_MATHLIBNAME, luaopen_math},
    {LUA_UTF8LIBNAME, luaopen_utf8},
    {LUA_DBLIBNAME, luaopen_debug},
    {"console", luaopen_console},
    {"computer", luaopen_computer},
    {"fs", luaopen_fs},
    {LUA_IOLIBNAME, luaopen_io},
    {LUA_OSLIBNAME, luaopen_os},
    {NULL, NULL},
};

static const luaL_Reg preloaded[] = {
    {"serial", luaopen_serial},
    {"cjson", luaopen_cjson},
    {"posix.errno", luaopen_posix_errno},
    {"posix.fcntl", luaopen_posix_fcntl},
    {"posix.unistd", luaopen_posix_unistd},
    {"kernel.poll", luaopen_posix_poll}, /* posix.poll and posix.time are in /lib/posix */
    {"kernel.time", luaopen_posix_time},
    {"posix.signal", luaopen_posix_signal},
    {"posix.sys.stat", luaopen_posix_sys_stat},
    {"posix.sys.socket", luaopen_posix_sys_socket},
    {"posix.dirent", luaopen_posix_dirent},
    {NULL, NULL},
};

static const char fallback_repl[] =
    "local fcntl, unistd, poll = require('posix.fcntl'), require('posix.unistd'), require('kernel.poll')\n"
    "local tty = assert(fcntl.open('/dev/ttyS0', fcntl.O_RDONLY))\n"
    "local function readline()\n"
    "  local line = ''\n"
    "  while true do\n"
    "    poll.rpoll(tty, -1)\n"
    "    for c in (unistd.read(tty, 256) or ''):gmatch('.') do\n"
    "      if c == '\\r' or c == '\\n' then console.write('\\n') return line\n"
    "      elseif c == '\\8' or c == '\\127' then\n"
    "        if #line > 0 then line = line:sub(1, -2) console.write('\\8 \\8') end\n"
    "      else line = line .. c console.write(c) end\n"
    "    end\n"
    "  end\n"
    "end\n"
    "while true do\n"
    "  console.write('lua> ')\n"
    "  local line = readline()\n"
    "  local fn, err = load('return ' .. line, '=stdin')\n"
    "  if not fn then fn, err = load(line, '=stdin') end\n"
    "  if fn then\n"
    "    local r = table.pack(xpcall(fn, debug.traceback))\n"
    "    if r.n > 1 then print(table.unpack(r, 2, r.n)) end\n"
    "  else print(err) end\n"
    "end\n";

static int message_handler(lua_State *L) {
    const char *message = lua_tostring(L, 1);
    if (message == NULL) {
        message = luaL_tolstring(L, 1, NULL);
    }
    luaL_traceback(L, L, message, 1);
    return 1;
}

static void print_error(lua_State *L) {
    const char *message = lua_tostring(L, -1);
    console_puts(message != NULL ? message : "(error object is not a string)");
    console_puts("\n");
    lua_pop(L, 1);
}

static void run(lua_State *L) {
    lua_pushcfunction(L, message_handler);
    lua_insert(L, -2);
    if (lua_pcall(L, 0, 0, -2) != LUA_OK) {
        print_error(L);
    }
    lua_pop(L, 1);
}

void signal_init(lua_State *L);
void signal_close(void);

void app_main(void) {
    lua_State *L = luaL_newstate();
    if (L == NULL) {
        kprintf("cannot create state: not enough memory\n");
        return;
    }
    for (const luaL_Reg *lib = libraries; lib->func != NULL; lib++) {
        luaL_requiref(L, lib->name, lib->func, 1);
        lua_pop(L, 1);
    }
    luaL_getsubtable(L, LUA_REGISTRYINDEX, LUA_PRELOAD_TABLE);
    luaL_setfuncs(L, preloaded, 0);
    lua_pop(L, 1);
    lua_getglobal(L, "package");
    lua_getfield(L, -1, "searchers");
    lua_pushnil(L);
    lua_rawseti(L, -2, 4);
    lua_pushnil(L);
    lua_rawseti(L, -2, 3);
    lua_pop(L, 2);
    signal_init(L);

    struct fs_stat st;
    if (fs_stat("/init.lua", &st) == 0) {
        if (luaL_loadfile(L, "/init.lua") == LUA_OK) {
            run(L);
        } else {
            print_error(L);
        }
    }
    if (luaL_loadbuffer(L, fallback_repl, sizeof(fallback_repl) - 1, "=repl") == LUA_OK) {
        run(L);
    }
    signal_close();
    lua_close(L);
}
