/* SPDX-License-Identifier: MIT */

#include <stdlib.h>
#include <string.h>
#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"
#include "lualib.h"

static const char *check_path(lua_State *L, const int index) {
    size_t len;
    const char *path = luaL_checklstring(L, index, &len);
    luaL_argcheck(L, strlen(path) == len, index, "path contains a zero byte");
    return path;
}

static const char *opt_path(lua_State *L, const int index, const char *fallback) {
    return lua_isnoneornil(L, index) ? fallback : check_path(L, index);
}

static int fail(lua_State *L, const int code) {
    luaL_pushfail(L);
    lua_pushstring(L, strerror(-code));
    return 2;
}

static int ok_or_fail(lua_State *L, const int code) {
    if (code < 0) {
        return fail(L, code);
    }
    lua_pushboolean(L, 1);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* console */

static int console_write_args(lua_State *L, const int first) {
    const int n = lua_gettop(L);
    for (int i = first; i <= n; i++) {
        size_t len;
        const char *s = luaL_checklstring(L, i, &len);
        console_write(s, len);
    }
    return 0;
}

static int l_console_write(lua_State *L) {
    return console_write_args(L, 1);
}

static int l_console_interrupts(lua_State *L) {
    lua_pushinteger(L, (lua_Integer) console_interrupts());
    return 1;
}

static const luaL_Reg console_lib[] = {
    {"write", l_console_write},
    {"interrupts", l_console_interrupts},
    {NULL, NULL},
};

/* ---------------------------------------------------------------------------------------- */
/* computer */

static int l_uptime(lua_State *L) {
    lua_pushnumber(L, (lua_Number) time_now() / (lua_Number) time_frequency());
    return 1;
}

static int l_free_memory(lua_State *L) {
    lua_pushinteger(L, (lua_Integer) (heap_total() - heap_used()));
    return 1;
}

static int l_total_memory(lua_State *L) {
    lua_pushinteger(L, (lua_Integer) heap_total());
    return 1;
}

static int l_sleep(lua_State *L) {
    sleep_until(time_now() + seconds_to_ticks(luaL_checknumber(L, 1)));
    return 0;
}

static int l_shutdown(lua_State *L) {
    (void) L;
    power_off();
}

static int l_reboot(lua_State *L) {
    (void) L;
    power_reboot();
}

static int l_stack(lua_State *L) {
    size_t peak, size;
    stack_usage(&peak, &size);
    lua_pushinteger(L, (lua_Integer) peak);
    lua_pushinteger(L, (lua_Integer) size);
    return 2;
}

static int l_world_time(lua_State *L) {
    u64 seconds;
    if (!rtc_seconds_of(1, &seconds)) {
        luaL_pushfail(L);
        return 1;
    }
    lua_pushinteger(L, (lua_Integer) seconds);
    return 1;
}

static const luaL_Reg computer_lib[] = {
    {"uptime", l_uptime},
    {"worldTime", l_world_time},
    {"freeMemory", l_free_memory},
    {"totalMemory", l_total_memory},
    {"sleep", l_sleep},
    {"shutdown", l_shutdown},
    {"reboot", l_reboot},
    {"stack", l_stack},
    {NULL, NULL},
};

/* ---------------------------------------------------------------------------------------- */
/* fs */

#define LISTING_META "openos2.listing"

static const char *const KINDS[] = {"file", "directory", "link", "device"};

static int l_listing_gc(lua_State *L) {
    struct fs_listing *listing = luaL_checkudata(L, 1, LISTING_META);
    free(listing->entries);
    free(listing->names);
    memset(listing, 0, sizeof(*listing));
    return 0;
}

struct fs_listing *push_listing(lua_State *L) {
    struct fs_listing *listing = lua_newuserdatauv(L, sizeof(struct fs_listing), 0);
    memset(listing, 0, sizeof(*listing));
    if (luaL_newmetatable(L, LISTING_META)) {
        lua_pushcfunction(L, l_listing_gc);
        lua_setfield(L, -2, "__gc");
    }
    lua_setmetatable(L, -2);
    return listing;
}

static int l_fs_list(lua_State *L) {
    const char *path = opt_path(L, 1, "/");
    struct fs_listing *listing = push_listing(L);
    const int result = fs_list(path, listing);
    if (result < 0) {
        return fail(L, result);
    }
    const int count = (int) listing->count;
    lua_createtable(L, count, 0);
    lua_createtable(L, 0, count);
    for (int i = 0; i < count; i++) {
        const struct fs_list_entry *entry = &listing->entries[i];
        lua_pushlstring(L, entry->name, entry->name_length);
        if (entry->kind == FS_KIND_DIRECTORY) {
            lua_pushliteral(L, "/");
            lua_concat(L, 2);
        }
        lua_pushvalue(L, -1);
        lua_rawseti(L, -4, i + 1);
        lua_pushstring(L, KINDS[entry->kind]);
        lua_rawset(L, -3);
    }
    return 2;
}

static int push_stat(lua_State *L, const int result, const struct fs_stat *st) {
    if (result < 0) {
        return fail(L, result);
    }
    lua_createtable(L, 0, 3);
    lua_pushinteger(L, (lua_Integer) st->size);
    lua_setfield(L, -2, "size");
    lua_pushstring(L, KINDS[st->kind]);
    lua_setfield(L, -2, "kind");
    lua_pushinteger(L, st->mtime);
    lua_setfield(L, -2, "mtime");
    return 1;
}

static int l_fs_stat(lua_State *L) {
    struct fs_stat st;
    return push_stat(L, fs_stat(check_path(L, 1), &st), &st);
}

static int l_fs_lstat(lua_State *L) {
    struct fs_stat st;
    return push_stat(L, fs_lstat(check_path(L, 1), &st), &st);
}

static int l_fs_readlink(lua_State *L) {
    char target[FS_PATH_LENGTH];
    const long result = fs_readlink(check_path(L, 1), target, sizeof(target));
    if (result < 0) {
        return fail(L, (int) result);
    }
    lua_pushlstring(L, target, (size_t) result);
    return 1;
}

static int l_fs_realpath(lua_State *L) {
    char path[FS_PATH_LENGTH];
    const int result = fs_realpath(check_path(L, 1), path);
    if (result < 0) {
        return fail(L, result);
    }
    lua_pushstring(L, path);
    return 1;
}

static int l_fs_symlink(lua_State *L) {
    return ok_or_fail(L, fs_symlink(check_path(L, 1), check_path(L, 2)));
}

static int l_fs_link(lua_State *L) {
    return ok_or_fail(L, fs_link(check_path(L, 1), check_path(L, 2)));
}

static int l_fs_exists(lua_State *L) {
    struct fs_stat st;
    lua_pushboolean(L, fs_stat(check_path(L, 1), &st) == 0);
    return 1;
}

static int l_fs_mkdir(lua_State *L) {
    return ok_or_fail(L, fs_mkdir(check_path(L, 1)));
}

static int l_fs_remove(lua_State *L) {
    return ok_or_fail(L, fs_remove(check_path(L, 1)));
}

static int l_fs_rename(lua_State *L) {
    return ok_or_fail(L, fs_rename(check_path(L, 1), check_path(L, 2)));
}

static int l_fs_info(lua_State *L) {
    u64 total, free;
    const int result = fs_info(opt_path(L, 1, "/"), &total, &free);
    if (result < 0) {
        return fail(L, result);
    }
    lua_pushinteger(L, (lua_Integer) total);
    lua_pushinteger(L, (lua_Integer) free);
    return 2;
}

static void mount_entry(lua_State *L, const struct fs_mount *mount) {
    if (mount->unmounted && !lua_toboolean(L, 1)) {
        return;
    }
    lua_createtable(L, 0, 9);
    lua_pushstring(L, mount->path);
    lua_setfield(L, -2, "path");
    lua_pushinteger(L, (lua_Integer) mount->index);
    lua_setfield(L, -2, "index");
    if (mount->label != NULL && mount->label[0] != 0) {
        lua_pushstring(L, mount->label);
        lua_setfield(L, -2, "label");
    }
    lua_pushinteger(L, (lua_Integer) mount->total);
    lua_setfield(L, -2, "size");
    lua_pushinteger(L, (lua_Integer) mount->free);
    lua_setfield(L, -2, "free");
    lua_pushboolean(L, !mount->writable);
    lua_setfield(L, -2, "readOnly");
    static const char *const media[] = {"unknown", "blank", "unsupported", "ext2", "9p", "tmpfs", "devfs", "sysfs"};
    lua_pushstring(L, media[mount->medium]);
    lua_setfield(L, -2, "medium");
    if (mount->generation != 0) {
        lua_pushinteger(L, mount->generation);
        lua_setfield(L, -2, "generation");
    }
    if (mount->unmounted) {
        lua_pushboolean(L, 1);
        lua_setfield(L, -2, "unmounted");
    }
    lua_rawseti(L, -2, (lua_Integer) lua_rawlen(L, -2) + 1);
}

static int l_fs_mounts(lua_State *L) {
    lua_settop(L, 1);
    lua_newtable(L);
    struct fs_mount mount;
    for (int i = 0, count = fs_mount_count(); i < count; i++) {
        if (fs_mount_entry(i, &mount)) {
            mount_entry(L, &mount);
        }
    }
    return 1;
}

static u32 opt_generation(lua_State *L, const int index) {
    return (u32) luaL_optinteger(L, index, 0);
}

static int l_fs_format(lua_State *L) {
    const char *path = check_path(L, 1);
    const char *label = NULL;
    u32 generation = 0;
    if (!lua_isnoneornil(L, 2)) {
        luaL_checktype(L, 2, LUA_TTABLE);
        lua_getfield(L, 2, "generation");
        generation = opt_generation(L, -1);
        lua_getfield(L, 2, "label");
        label = lua_isnil(L, -1) ? NULL : check_path(L, -1);
    }
    return ok_or_fail(L, fs_format(path, label, generation));
}

static int l_fs_unmount(lua_State *L) {
    return ok_or_fail(L, fs_unmount(check_path(L, 1), opt_generation(L, 2)));
}

static int l_fs_mount(lua_State *L) {
    return ok_or_fail(L, fs_mount(check_path(L, 1)));
}

static int l_fs_label(lua_State *L) {
    const char *path = check_path(L, 1);
    if (!lua_isnoneornil(L, 2)) {
        const int result = fs_set_label(path, check_path(L, 2), opt_generation(L, 3));
        if (result < 0) {
            return fail(L, result);
        }
        lua_settop(L, 2);
        return 1;
    }
    char label[FS_LABEL_LENGTH + 1];
    const int result = fs_label(path, label);
    if (result < 0) {
        return fail(L, result);
    }
    lua_pushstring(L, label);
    return 1;
}

static const luaL_Reg fs_lib[] = {
    {"list", l_fs_list},
    {"stat", l_fs_stat},
    {"lstat", l_fs_lstat},
    {"readlink", l_fs_readlink},
    {"realpath", l_fs_realpath},
    {"symlink", l_fs_symlink},
    {"link", l_fs_link},
    {"exists", l_fs_exists},
    {"mkdir", l_fs_mkdir},
    {"remove", l_fs_remove},
    {"rename", l_fs_rename},
    {"info", l_fs_info},
    {"mounts", l_fs_mounts},
    {"format", l_fs_format},
    {"label", l_fs_label},
    {"unmount", l_fs_unmount},
    {"mount", l_fs_mount},
    {NULL, NULL},
};

/* ---------------------------------------------------------------------------------------- */
/* serial */

static int check_serial(lua_State *L) {
    const char *device = luaL_checkstring(L, 1);
    if (strncmp(device, "/dev/", 5) == 0) {
        device += 5;
    }
    const int index = serial_find(device);
    luaL_argcheck(L, index >= 0, 1, "no such device");
    return index;
}

static int l_serial_set_baud(lua_State *L) {
    const int index = check_serial(L);
    const lua_Integer baud = luaL_checkinteger(L, 2);
    if (baud <= 0 || baud > 0xFFFFFFFF || serial_set_baud(index, (u32) baud) != 0) {
        luaL_pushfail(L);
        lua_pushliteral(L, "Invalid argument");
        return 2;
    }
    lua_pushboolean(L, 1);
    return 1;
}

static int l_serial_get_baud(lua_State *L) {
    lua_pushinteger(L, serial_baud(check_serial(L)));
    return 1;
}

static const luaL_Reg serial_lib[] = {
    {"setBaud", l_serial_set_baud},
    {"getBaud", l_serial_get_baud},
    {NULL, NULL},
};

int luaopen_serial(lua_State *L) {
    luaL_newlib(L, serial_lib);
    return 1;
}

int luaopen_console(lua_State *L) {
    luaL_newlib(L, console_lib);
    return 1;
}

int luaopen_computer(lua_State *L) {
    luaL_newlib(L, computer_lib);
    return 1;
}

int luaopen_fs(lua_State *L) {
    luaL_newlib(L, fs_lib);
    return 1;
}
