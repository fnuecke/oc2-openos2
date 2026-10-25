/* SPDX-License-Identifier: MIT */
/* Lua state setup and the kernel's Lua modules: console, computer, fs, and minimal io and os. */

#include <stdlib.h>
#include <string.h>
#include "drivers/console.h"
#include "drivers/rtc.h"
#include "drivers/serial.h"
#include "drivers/virtio/virtio.h"
#include "fs/fs.h"
#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"
#include "lualib.h"

#define FILE_META "openos2.file"
#define PORT_OPEN_TIMEOUT 1.0
#define CONSOLE_FD (-100)

struct lfile {
    int fd;
};


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
    lua_pushstring(L, fs_error(code));
    return 2;
}

static int ok_or_fail(lua_State *L, const int code) {
    if (code < 0) {
        return fail(L, code);
    }
    lua_pushboolean(L, 1);
    return 1;
}

/* Saturates, so that adding the result to the current time cannot wrap. */
static u64 seconds_to_ticks(const lua_Number seconds) {
    const lua_Number ticks = seconds * (lua_Number) time_frequency();
    if (!(ticks > 0)) {
        return 0;
    }
    return ticks >= 0x1p62 ? 1ull << 62 : (u64) ticks;
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

static u64 now_seconds(void) {
    u64 seconds = 0;
    rtc_seconds_of(0, &seconds);
    return seconds;
}

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
    reboot();
}

/* Returns the deepest stack use since boot and the stack size, in bytes. */
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

/* Owns the backend's buffer, so it is freed even when pushing the names raises an error. */
static int l_listing_gc(lua_State *L) {
    struct fs_listing *listing = luaL_checkudata(L, 1, LISTING_META);
    free(listing->data);
    listing->data = NULL;
    return 0;
}

/* Directories end in "/". The names are collected first, no Lua code runs during the listing. */
static int l_fs_list(lua_State *L) {
    const char *path = opt_path(L, 1, "/");
    struct fs_listing *listing = lua_newuserdatauv(L, sizeof(struct fs_listing), 0);
    memset(listing, 0, sizeof(*listing));
    if (luaL_newmetatable(L, LISTING_META)) {
        lua_pushcfunction(L, l_listing_gc);
        lua_setfield(L, -2, "__gc");
    }
    lua_setmetatable(L, -2);
    const int result = fs_list(path, listing);
    if (result < 0) {
        return fail(L, result);
    }
    lua_newtable(L);
    lua_Integer count = 0;
    for (size_t at = 0; at < listing->length; at += 2 + listing->data[at + 1]) {
        lua_pushlstring(L, (const char *) listing->data + at + 2, listing->data[at + 1]);
        if (listing->data[at]) {
            lua_pushliteral(L, "/");
            lua_concat(L, 2);
        }
        lua_rawseti(L, -2, ++count);
    }
    return 1;
}

static int l_fs_stat(lua_State *L) {
    struct fs_stat st;
    const int result = fs_stat(check_path(L, 1), &st);
    if (result < 0) {
        return fail(L, result);
    }
    lua_createtable(L, 0, 3);
    lua_pushinteger(L, (lua_Integer) st.size);
    lua_setfield(L, -2, "size");
    lua_pushboolean(L, st.is_dir);
    lua_setfield(L, -2, "isDir");
    lua_pushinteger(L, st.mtime);
    lua_setfield(L, -2, "mtime");
    return 1;
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

static void mount_entry(void *ctx, const struct fs_mount *mount) {
    lua_State *L = ctx;
    lua_createtable(L, 0, 8);
    lua_pushstring(L, mount->path);
    lua_setfield(L, -2, "path");
    if (mount->index >= 0) {
        lua_pushinteger(L, mount->index);
        lua_setfield(L, -2, "index");
    }
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
    static const char *const media[] = {"unknown", "blank", "unsupported", "ext2", "9p"};
    lua_pushstring(L, media[mount->medium]);
    lua_setfield(L, -2, "medium");
    if (mount->serial != 0) {
        lua_pushinteger(L, mount->serial);
        lua_setfield(L, -2, "serial");
    }
    lua_rawseti(L, -2, (lua_Integer) lua_rawlen(L, -2) + 1);
}

static int l_fs_mounts(lua_State *L) {
    lua_newtable(L);
    fs_mounts(mount_entry, L);
    return 1;
}

static u32 opt_serial(lua_State *L, const int index) {
    return (u32) luaL_optinteger(L, index, 0);
}

static void raw_options(lua_State *L, const int index, int *force, u32 *serial) {
    *force = 0;
    *serial = 0;
    if (!lua_isnoneornil(L, index)) {
        luaL_checktype(L, index, LUA_TTABLE);
        lua_getfield(L, index, "force");
        *force = lua_toboolean(L, -1);
        lua_getfield(L, index, "serial");
        *serial = opt_serial(L, -1);
        lua_pop(L, 2);
    }
}

/* fs.format(path [, {label = ..., force = ..., serial = ...}]), see fs_format */
static int l_fs_format(lua_State *L) {
    const char *path = check_path(L, 1);
    const char *label = NULL;
    int force;
    u32 serial;
    raw_options(L, 2, &force, &serial);
    if (!lua_isnoneornil(L, 2)) {
        lua_getfield(L, 2, "label");
        label = lua_isnil(L, -1) ? NULL : check_path(L, -1);
    }
    return ok_or_fail(L, fs_format(path, label, force, serial));
}

/* fs.label(path [, label [, serial]]) -> the label, "" if none; with a label, sets it. */
static int l_fs_label(lua_State *L) {
    const char *path = check_path(L, 1);
    if (!lua_isnoneornil(L, 2)) {
        const int result = fs_set_label(path, check_path(L, 2), opt_serial(L, 3));
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

static int l_fs_open_raw(lua_State *L);

static const luaL_Reg fs_lib[] = {
    {"list", l_fs_list},
    {"stat", l_fs_stat},
    {"exists", l_fs_exists},
    {"mkdir", l_fs_mkdir},
    {"remove", l_fs_remove},
    {"rename", l_fs_rename},
    {"info", l_fs_info},
    {"mounts", l_fs_mounts},
    {"format", l_fs_format},
    {"label", l_fs_label},
    {"openRaw", l_fs_open_raw},
    {NULL, NULL},
};

/* ---------------------------------------------------------------------------------------- */
/* io */

static struct lfile *check_file(lua_State *L, const int index) {
    struct lfile *f = luaL_checkudata(L, index, FILE_META);
    if (f->fd == -1) {
        luaL_error(L, "attempt to use a closed file");
    }
    return f;
}

static struct lfile *new_file(lua_State *L, const int fd) {
    struct lfile *f = lua_newuserdatauv(L, sizeof(struct lfile), 0);
    f->fd = fd;
    luaL_setmetatable(L, FILE_META);
    return f;
}

static int console_unreadable(lua_State *L) {
    return luaL_error(L, "Bad file descriptor");
}

static int read_error(lua_State *L, const int code) {
    luaL_pushfail(L);
    lua_pushstring(L, fs_error(code));
    return -1;
}

static int read_line(lua_State *L, const struct lfile *f, const int keep_newline) {
    if (f->fd == CONSOLE_FD) {
        return console_unreadable(L);
    }
    luaL_Buffer b;
    luaL_buffinit(L, &b);
    int any = 0;
    for (;;) {
        char chunk[128];
        const long count = fs_read(f->fd, chunk, sizeof(chunk));
        if (count < 0) {
            return read_error(L, (int) count);
        }
        if (count == 0) {
            break;
        }
        any = 1;
        const char *newline = memchr(chunk, '\n', (size_t) count);
        if (newline != NULL) {
            const long used = newline - chunk + 1;
            luaL_addlstring(&b, chunk, (size_t) (keep_newline ? used : used - 1));
            fs_seek(f->fd, used - count, 1);
            break;
        }
        luaL_addlstring(&b, chunk, (size_t) count);
    }
    luaL_pushresult(&b);
    if (!any) {
        lua_pop(L, 1);
        luaL_pushfail(L);
    }
    return 1;
}

static int read_bytes(lua_State *L, const struct lfile *f, size_t count, const int all) {
    if (f->fd == CONSOLE_FD) {
        return console_unreadable(L);
    }
    if (count == 0 && !all) {
        /* Like Lua's io: "" unless at the end of the file. */
        char probe;
        const long got = fs_read(f->fd, &probe, 1);
        if (got < 0) {
            return read_error(L, (int) got);
        }
        if (got == 0) {
            luaL_pushfail(L);
        } else {
            fs_seek(f->fd, -1, 1);
            lua_pushliteral(L, "");
        }
        return 1;
    }
    luaL_Buffer b;
    luaL_buffinit(L, &b);
    int any = 0;
    while (all || count > 0) {
        char chunk[512];
        const size_t want = all || count > sizeof(chunk) ? sizeof(chunk) : count;
        const long got = fs_read(f->fd, chunk, want);
        if (got < 0) {
            return read_error(L, (int) got);
        }
        if (got == 0) {
            break;
        }
        any = 1;
        luaL_addlstring(&b, chunk, (size_t) got);
        if (!all) {
            count -= (size_t) got;
        }
    }
    luaL_pushresult(&b);
    if (!any && !all && count > 0) {
        lua_pop(L, 1);
        luaL_pushfail(L);
    }
    return 1;
}

static int read_formats(lua_State *L, const struct lfile *f, const int first) {
    const int n = lua_gettop(L);
    if (n < first) {
        return read_line(L, f, 0);
    }
    for (int i = first; i <= n; i++) {
        int status;
        if (lua_type(L, i) == LUA_TNUMBER) {
            status = read_bytes(L, f, (size_t) luaL_checkinteger(L, i), 0);
        } else {
            const char *format = luaL_checkstring(L, i);
            if (*format == '*') {
                format++;
            }
            switch (*format) {
                case 'l': status = read_line(L, f, 0); break;
                case 'L': status = read_line(L, f, 1); break;
                case 'a': status = read_bytes(L, f, 0, 1); break;
                default: return luaL_argerror(L, i, "invalid format");
            }
        }
        if (status < 0) {
            return 2;
        }
        if (lua_isnil(L, -1)) {
            return i - first + 1;
        }
    }
    return n - first + 1;
}

static int write_args(lua_State *L, const struct lfile *f, const int first) {
    const int n = lua_gettop(L);
    for (int i = first; i <= n; i++) {
        size_t len;
        const char *s = luaL_checklstring(L, i, &len);
        if (f->fd == CONSOLE_FD) {
            console_write(s, len);
            continue;
        }
        const long written = fs_write(f->fd, s, len);
        if (written < 0) {
            return fail(L, (int) written);
        }
        if ((size_t) written != len) {
            const long rest = fs_write(f->fd, s + written, len - (size_t) written);
            if (rest < 0 || (size_t) rest != len - (size_t) written) {
                return fail(L, rest < 0 ? (int) rest : FS_NO_SPACE);
            }
        }
    }
    lua_pushvalue(L, first - 1);
    return 1;
}

static int l_file_read(lua_State *L) {
    return read_formats(L, check_file(L, 1), 2);
}

static int l_file_write(lua_State *L) {
    return write_args(L, check_file(L, 1), 2);
}

static int l_file_seek(lua_State *L) {
    static const char *const modes[] = {"set", "cur", "end", NULL};
    const struct lfile *f = check_file(L, 1);
    const int whence = luaL_checkoption(L, 2, "cur", modes);
    const lua_Integer offset = luaL_optinteger(L, 3, 0);
    if (f->fd == CONSOLE_FD) {
        return fail(L, FS_INVALID);
    }
    const long pos = fs_seek(f->fd, (long) offset, whence);
    if (pos < 0) {
        return fail(L, (int) pos);
    }
    lua_pushinteger(L, pos);
    return 1;
}

static int l_file_close(lua_State *L) {
    struct lfile *f = check_file(L, 1);
    if (f->fd != CONSOLE_FD) {
        fs_close(f->fd);
        f->fd = -1;
    }
    lua_pushboolean(L, 1);
    return 1;
}

static int l_file_gc(lua_State *L) {
    struct lfile *f = luaL_checkudata(L, 1, FILE_META);
    if (f->fd >= 0) {
        fs_close(f->fd);
        f->fd = -1;
    }
    return 0;
}

static int l_file_tostring(lua_State *L) {
    const struct lfile *f = luaL_checkudata(L, 1, FILE_META);
    if (f->fd == -1) {
        lua_pushliteral(L, "file (closed)");
    } else {
        lua_pushfstring(L, "file (%p)", (void *) f);
    }
    return 1;
}

static int lines_next(lua_State *L) {
    struct lfile *f = lua_touserdata(L, lua_upvalueindex(1));
    if (f->fd == -1) {
        return luaL_error(L, "file is already closed");
    }
    lua_settop(L, 0);
    lua_pushvalue(L, lua_upvalueindex(1));
    lua_pushvalue(L, lua_upvalueindex(2));
    if (read_formats(L, f, 2) == 2 && lua_isnil(L, -2)) {
        return luaL_error(L, "%s", lua_tostring(L, -1)); /* like Lua's io.lines */
    }
    if (lua_isnil(L, -1) && lua_toboolean(L, lua_upvalueindex(3))) {
        fs_close(f->fd);
        f->fd = -1;
    }
    return 1;
}

static int push_lines(lua_State *L, const int file_index, const int close_at_end) {
    const int has_format = !lua_isnoneornil(L, file_index + 1);
    lua_pushvalue(L, file_index);
    if (has_format) {
        lua_pushvalue(L, file_index + 1);
    } else {
        lua_pushliteral(L, "l");
    }
    lua_pushboolean(L, close_at_end);
    lua_pushcclosure(L, lines_next, 3);
    return 1;
}

static int l_file_lines(lua_State *L) {
    check_file(L, 1);
    return push_lines(L, 1, 0);
}

static int open_flags(const char *mode) {
    int flags;
    switch (mode[0]) {
        case 'r': flags = O_READ; break;
        case 'w': flags = O_WRITE | O_CREATE | O_TRUNCATE; break;
        case 'a': flags = O_WRITE | O_CREATE | O_APPEND; break;
        default: return -1;
    }
    for (const char *p = mode + 1; *p; p++) {
        if (*p == '+') {
            flags |= O_READ | O_WRITE;
        } else if (*p != 'b') {
            return -1;
        }
    }
    return flags;
}

static int l_io_open(lua_State *L) {
    const char *path = check_path(L, 1);
    const int flags = open_flags(luaL_optstring(L, 2, "r"));
    luaL_argcheck(L, flags >= 0, 2, "invalid mode");
    struct lfile *f = new_file(L, -1); /* allocated first, so an out of memory error cannot leak the fd */
    const int fd = fs_open(path, flags);
    if (fd < 0) {
        char normalized[FS_PATH_LENGTH];
        luaL_pushfail(L);
        lua_pushfstring(L, "%s: %s", fs_normalize(path, normalized) == 0 ? normalized : path, fs_error(fd));
        return 2;
    }
    f->fd = fd;
    return 1;
}

/* fs.openRaw(path [, mode [, {force = ..., serial = ...}]]): mode "r" or "w", see fs_open_raw */
static int l_fs_open_raw(lua_State *L) {
    const char *path = check_path(L, 1);
    const char *mode = luaL_optstring(L, 2, "r");
    luaL_argcheck(L, strcmp(mode, "r") == 0 || strcmp(mode, "w") == 0, 2, "invalid mode");
    int force;
    u32 serial;
    raw_options(L, 3, &force, &serial);
    struct lfile *f = new_file(L, -1);
    const int fd = fs_open_raw(path, mode[0] == 'w' ? O_WRITE : O_READ, force, serial);
    if (fd < 0) {
        return fail(L, fd);
    }
    f->fd = fd;
    return 1;
}

static int l_io_lines(lua_State *L) {
    const char *path = check_path(L, 1);
    struct lfile *f = new_file(L, -1);
    const int fd = fs_open(path, O_READ);
    if (fd < 0) {
        return luaL_error(L, "%s: %s", path, fs_error(fd));
    }
    f->fd = fd;
    lua_replace(L, 1);
    return push_lines(L, 1, 1);
}

static int l_io_write(lua_State *L) {
    console_write_args(L, 1);
    lua_getfield(L, lua_upvalueindex(1), "stdout");
    return 1;
}

static int l_io_read(lua_State *L) {
    static struct lfile console = {CONSOLE_FD};
    return read_formats(L, &console, 1);
}

static int l_io_close(lua_State *L) {
    return l_file_close(L);
}

static int l_io_type(lua_State *L) {
    luaL_checkany(L, 1);
    const struct lfile *f = luaL_testudata(L, 1, FILE_META);
    if (f == NULL) {
        luaL_pushfail(L);
    } else if (f->fd == -1) {
        lua_pushliteral(L, "closed file");
    } else {
        lua_pushliteral(L, "file");
    }
    return 1;
}

static const luaL_Reg file_methods[] = {
    {"read", l_file_read},
    {"write", l_file_write},
    {"lines", l_file_lines},
    {"seek", l_file_seek},
    {"close", l_file_close},
    {NULL, NULL},
};

static const luaL_Reg file_meta[] = {
    {"__gc", l_file_gc},
    {"__close", l_file_gc},
    {"__tostring", l_file_tostring},
    {NULL, NULL},
};

static const luaL_Reg io_lib[] = {
    {"open", l_io_open},
    {"lines", l_io_lines},
    {"read", l_io_read},
    {"close", l_io_close},
    {"type", l_io_type},
    {NULL, NULL},
};

static int open_io(lua_State *L) {
    luaL_newmetatable(L, FILE_META);
    luaL_setfuncs(L, file_meta, 0);
    luaL_newlib(L, file_methods);
    lua_setfield(L, -2, "__index");
    lua_pop(L, 1);

    luaL_newlib(L, io_lib);
    lua_pushvalue(L, -1);
    lua_pushcclosure(L, l_io_write, 1);
    lua_setfield(L, -2, "write");
    new_file(L, CONSOLE_FD);
    lua_setfield(L, -2, "stdout");
    new_file(L, CONSOLE_FD);
    lua_setfield(L, -2, "stdin");
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* os */

/* days_from_civil and civil_from_time follow Howard Hinnant's public domain date algorithms. */
static long days_from_civil(long y, const long m, const long d) {
    y -= m <= 2;
    const long era = (y >= 0 ? y : y - 399) / 400;
    const long yoe = y - era * 400;
    const long doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    const long doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

struct date {
    long year, month, day, hour, min, sec, wday, yday;
};

static void civil_from_time(const long t, struct date *out) {
    long days = t / 86400;
    long rest = t % 86400;
    if (rest < 0) {
        rest += 86400;
        days--;
    }
    out->hour = rest / 3600;
    out->min = rest / 60 % 60;
    out->sec = rest % 60;
    out->wday = ((days % 7) + 11) % 7 + 1; /* 1970-01-01 was a Thursday; Sunday is 1 */

    const long z = days + 719468;
    const long era = (z >= 0 ? z : z - 146096) / 146097;
    const long doe = z - era * 146097;
    const long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    const long doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    const long mp = (5 * doy + 2) / 153;
    out->day = doy - (153 * mp + 2) / 5 + 1;
    out->month = mp < 10 ? mp + 3 : mp - 9;
    out->year = yoe + era * 400 + (out->month <= 2);
    out->yday = days - days_from_civil(out->year, 1, 1) + 1;
}

static long get_field(lua_State *L, const char *key, const long fallback) {
    const int type = lua_getfield(L, 1, key);
    long value = fallback;
    if (type != LUA_TNIL) {
        int is_number;
        value = (long) lua_tointegerx(L, -1, &is_number);
        if (!is_number) {
            luaL_error(L, "field '%s' is not an integer", key);
        }
    } else if (fallback < 0) {
        luaL_error(L, "field '%s' missing in date table", key);
    }
    lua_pop(L, 1);
    return value;
}

static int l_os_time(lua_State *L) {
    if (lua_isnoneornil(L, 1)) {
        lua_pushinteger(L, (lua_Integer) now_seconds());
        return 1;
    }
    luaL_checktype(L, 1, LUA_TTABLE);
    const long year = get_field(L, "year", -1);
    const long month = get_field(L, "month", -1);
    const long day = get_field(L, "day", -1);
    const long hour = get_field(L, "hour", 12);
    const long min = get_field(L, "min", 0);
    const long sec = get_field(L, "sec", 0);
    /* Normalize the month so days_from_civil sees 1..12; day, hour, min and sec may overflow. */
    const long m0 = month - 1;
    const long y = year + (m0 >= 0 ? m0 / 12 : (m0 - 11) / 12);
    const long m = m0 - (y - year) * 12 + 1;
    const long days = days_from_civil(y, m, 1) + day - 1;
    lua_pushinteger(L, days * 86400 + hour * 3600 + min * 60 + sec);
    return 1;
}

static int l_os_clock(lua_State *L) {
    return l_uptime(L);
}

static void set_date_field(lua_State *L, const char *key, const long value) {
    lua_pushinteger(L, value);
    lua_setfield(L, -2, key);
}

static int l_os_date(lua_State *L) {
    const char *format = luaL_optstring(L, 1, "%c");
    const long t = lua_isnoneornil(L, 2) ? (long) now_seconds() : (long) luaL_checkinteger(L, 2);
    if (*format == '!') {
        format++; /* always UTC */
    }
    struct date d;
    civil_from_time(t, &d);
    if (strcmp(format, "*t") == 0) {
        lua_createtable(L, 0, 9);
        set_date_field(L, "year", d.year);
        set_date_field(L, "month", d.month);
        set_date_field(L, "day", d.day);
        set_date_field(L, "hour", d.hour);
        set_date_field(L, "min", d.min);
        set_date_field(L, "sec", d.sec);
        set_date_field(L, "wday", d.wday);
        set_date_field(L, "yday", d.yday);
        lua_pushboolean(L, 0);
        lua_setfield(L, -2, "isdst");
        return 1;
    }
    luaL_Buffer b;
    luaL_buffinit(L, &b);
    for (const char *p = format; *p; p++) {
        if (*p != '%') {
            luaL_addchar(&b, *p);
            continue;
        }
        char part[32];
        switch (*++p) {
            case 'Y': snprintf(part, sizeof(part), "%ld", d.year); break;
            case 'm': snprintf(part, sizeof(part), "%02ld", d.month); break;
            case 'd': snprintf(part, sizeof(part), "%02ld", d.day); break;
            case 'H': snprintf(part, sizeof(part), "%02ld", d.hour); break;
            case 'M': snprintf(part, sizeof(part), "%02ld", d.min); break;
            case 'S': snprintf(part, sizeof(part), "%02ld", d.sec); break;
            case 'j': snprintf(part, sizeof(part), "%03ld", d.yday); break;
            case 'x':
            case 'F': snprintf(part, sizeof(part), "%ld-%02ld-%02ld", d.year, d.month, d.day); break;
            case 'X':
            case 'T': snprintf(part, sizeof(part), "%02ld:%02ld:%02ld", d.hour, d.min, d.sec); break;
            case 'c':
                snprintf(part, sizeof(part), "%ld-%02ld-%02ld %02ld:%02ld:%02ld",
                    d.year, d.month, d.day, d.hour, d.min, d.sec);
                break;
            case '%': strcpy(part, "%"); break;
            default: return luaL_error(L, "invalid conversion specifier '%%%c'", *p ? *p : ' ');
        }
        luaL_addstring(&b, part);
    }
    luaL_pushresult(&b);
    return 1;
}

static int l_os_difftime(lua_State *L) {
    lua_pushnumber(L, (lua_Number) (luaL_checkinteger(L, 1) - luaL_optinteger(L, 2, 0)));
    return 1;
}

/* There is no environment. OC2_BUS_SOCKET = "none" makes oc2's bus.lua use the ports directly. */
static int l_os_getenv(lua_State *L) {
    if (strcmp(luaL_checkstring(L, 1), "OC2_BUS_SOCKET") == 0) {
        lua_pushliteral(L, "none");
    } else {
        luaL_pushfail(L);
    }
    return 1;
}

static int l_os_exit(lua_State *L) {
    (void) L;
    power_off();
}

/*
 * os.execute(command) only knows "stty", which oc2's serial library runs to set a serial port up:
 * a number sets the baud rate of the port after -F, raw mode and no echo are how the ports always
 * work, and other words are ignored. Returns like Lua's os.execute, other commands exit with 127.
 */
/* There is no shell in the kernel: like Lua built without one. /init.lua runs commands. */
static int l_os_execute(lua_State *L) {
    if (lua_isnoneornil(L, 1)) {
        lua_pushboolean(L, 0);
        return 1;
    }
    luaL_checkstring(L, 1);
    luaL_pushfail(L);
    lua_pushliteral(L, "no shell");
    lua_pushinteger(L, 127);
    return 3;
}

static const luaL_Reg os_lib[] = {
    {"time", l_os_time},
    {"clock", l_os_clock},
    {"date", l_os_date},
    {"difftime", l_os_difftime},
    {"getenv", l_os_getenv},
    {"exit", l_os_exit},
    {"remove", l_fs_remove},
    {"rename", l_fs_rename},
    {"execute", l_os_execute},
    {NULL, NULL},
};

/* ---------------------------------------------------------------------------------------- */
/* ports: named virtio-console ports, opened through posix.fcntl as /dev/<name> */

/* Returns whether a port with this name exists, waiting up to the timeout for the handshake. */
static int l_ports_find(lua_State *L) {
    const char *name = luaL_checkstring(L, 1);
    const u64 deadline = time_now() + seconds_to_ticks(luaL_optnumber(L, 2, PORT_OPEN_TIMEOUT));
    lua_pushboolean(L, vcon_find(name, deadline) >= 0);
    return 1;
}

static void list_port(void *ctx, const int port, const char *name) {
    (void) port;
    lua_State *L = ctx;
    lua_pushstring(L, name);
    lua_rawseti(L, -2, (lua_Integer) lua_rawlen(L, -2) + 1);
}

static int l_ports_list(lua_State *L) {
    lua_newtable(L);
    vcon_ports(list_port, L);
    return 1;
}

static const luaL_Reg ports_lib[] = {
    {"find", l_ports_find},
    {"list", l_ports_list},
    {NULL, NULL},
};

static int open_ports(lua_State *L) {
    luaL_newlib(L, ports_lib);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* serial: the baud rates of /dev/ttyS<n>, for stty */

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

static int open_serial(lua_State *L) {
    luaL_newlib(L, serial_lib);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* Startup */

static int open_console(lua_State *L) {
    luaL_newlib(L, console_lib);
    return 1;
}

static int open_computer(lua_State *L) {
    luaL_newlib(L, computer_lib);
    return 1;
}

static int open_fs(lua_State *L) {
    luaL_newlib(L, fs_lib);
    return 1;
}

static int open_os(lua_State *L) {
    luaL_newlib(L, os_lib);
    return 1;
}

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
    {"console", open_console},
    {"computer", open_computer},
    {"fs", open_fs},
    {LUA_IOLIBNAME, open_io},
    {LUA_OSLIBNAME, open_os},
    {NULL, NULL},
};

/* Loaded by require only. */
static const luaL_Reg preloaded[] = {
    {"ports", open_ports},
    {"serial", open_serial},
    {"cjson", luaopen_cjson},
    {"posix.errno", luaopen_posix_errno},
    {"posix.fcntl", luaopen_posix_fcntl},
    {"posix.unistd", luaopen_posix_unistd},
    {"posix.poll", luaopen_posix_poll},
    {"posix.time", luaopen_posix_time},
    {"posix.signal", luaopen_posix_signal},
    {"posix.sys.stat", luaopen_posix_sys_stat},
    {"posix.sys.socket", luaopen_posix_sys_socket},
    {"posix.dirent", luaopen_posix_dirent},
    {NULL, NULL},
};

/* Used when there is no /init.lua, or it fails. */
static const char fallback_repl[] =
    "local function readline()\n"
    "  local line = ''\n"
    "  while true do\n"
    "    local name, input = computer.pullSignal()\n"
    "    for c in (name == 'console' and input or ''):gmatch('.') do\n"
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
    /* Only the preload and Lua searchers: there are no C modules to load. */
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
