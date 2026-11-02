/* SPDX-License-Identifier: MIT */

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"

#define READ_MAX 4096

static int fail_errno(lua_State *L, const int number) {
    luaL_pushfail(L);
    lua_pushstring(L, strerror(number));
    lua_pushinteger(L, number);
    return 3;
}

static int fail_code(lua_State *L, const int code) {
    return fail_errno(L, -code);
}

static int not_supported(lua_State *L) {
    return fail_errno(L, ENOSYS);
}

static int check_fd(lua_State *L, const int index) {
    const lua_Integer fd = luaL_checkinteger(L, index);
    return fd >= 0 && fd <= 0x7FFFFFFF ? (int) fd : -1;
}

static u64 deadline_ms(const lua_Integer ms) {
    return ms < 0 ? TIME_NEVER : time_now() + (u64) ms * (time_frequency() / 1000);
}

static void set_constant(lua_State *L, const char *name, const lua_Integer value) {
    lua_pushinteger(L, value);
    lua_setfield(L, -2, name);
}

/* ---------------------------------------------------------------------------------------- */
/* posix.errno */

int luaopen_posix_errno(lua_State *L) {
    lua_newtable(L);
    set_constant(L, "EINTR", EINTR);
    set_constant(L, "EAGAIN", EAGAIN);
    set_constant(L, "EWOULDBLOCK", EAGAIN);
    set_constant(L, "ENOENT", ENOENT);
    set_constant(L, "EBADF", EBADF);
    set_constant(L, "ENOSYS", ENOSYS);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.fcntl */

static int l_open(lua_State *L) {
    const char *path = luaL_checkstring(L, 1);
    const lua_Integer mode = luaL_optinteger(L, 2, 0) & (O_RDONLY | O_WRONLY | O_RDWR);
    const int flags = mode == O_RDONLY ? FS_O_READ : mode == O_WRONLY ? FS_O_WRITE : FS_O_READ | FS_O_WRITE;
    const int fd = fs_open(path, flags | FS_O_NONBLOCK);
    if (fd < 0) {
        return fail_code(L, fd);
    }
    lua_pushinteger(L, fd);
    return 1;
}

static int l_fcntl(lua_State *L) {
    const int result = fs_poll(check_fd(L, 1));
    if (result < 0) {
        return fail_code(L, result);
    }
    lua_pushinteger(L, 0);
    return 1;
}

static const luaL_Reg fcntl_lib[] = {
    {"open", l_open},
    {"fcntl", l_fcntl},
    {NULL, NULL},
};

int luaopen_posix_fcntl(lua_State *L) {
    luaL_newlib(L, fcntl_lib);
    set_constant(L, "O_RDONLY", O_RDONLY);
    set_constant(L, "O_WRONLY", O_WRONLY);
    set_constant(L, "O_RDWR", O_RDWR);
    set_constant(L, "O_NONBLOCK", O_NONBLOCK);
    set_constant(L, "O_CLOEXEC", O_CLOEXEC);
    set_constant(L, "F_GETFD", F_GETFD);
    set_constant(L, "F_SETFD", F_SETFD);
    set_constant(L, "F_GETFL", F_GETFL);
    set_constant(L, "F_SETFL", F_SETFL);
    set_constant(L, "FD_CLOEXEC", FD_CLOEXEC);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.unistd */

static int l_read(lua_State *L) {
    const int fd = check_fd(L, 1);
    lua_Integer count = luaL_checkinteger(L, 2);
    if (count > READ_MAX) {
        count = READ_MAX;
    }
    luaL_Buffer b;
    char *buffer = luaL_buffinitsize(L, &b, count > 0 ? (size_t) count : 0);
    const long got = count > 0 ? fs_read(fd, buffer, (size_t) count) : 0;
    if (got < 0) {
        return fail_code(L, (int) got);
    }
    luaL_pushresultsize(&b, (size_t) got);
    return 1;
}

static int l_write(lua_State *L) {
    const int fd = check_fd(L, 1);
    size_t len;
    const char *data = luaL_checklstring(L, 2, &len);
    const long written = fs_write(fd, data, len);
    if (written < 0) {
        return fail_code(L, (int) written);
    }
    lua_pushinteger(L, written);
    return 1;
}

static int l_close(lua_State *L) {
    const int result = fs_close(check_fd(L, 1));
    if (result < 0) {
        return fail_code(L, result);
    }
    lua_pushinteger(L, 0);
    return 1;
}

static const luaL_Reg unistd_lib[] = {
    {"read", l_read},
    {"write", l_write},
    {"close", l_close},
    {NULL, NULL},
};

int luaopen_posix_unistd(lua_State *L) {
    luaL_newlib(L, unistd_lib);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.poll */

struct poll_entry {
    int fd;
    int events; /* FS_POLL_IN and FS_POLL_OUT */
};

static int poll_ready_events(const struct poll_entry *entry) {
    const int ready = fs_poll(entry->fd);
    return ready < 0 ? -1 : ready & entry->events;
}

static bool poll_ready(const struct poll_entry *set, const int count) {
    for (int i = 0; i < count; i++) {
        if (poll_ready_events(&set[i]) != 0) {
            return 1;
        }
    }
    return 0;
}

static bool poll_wait(const struct poll_entry *set, const int count, const u64 deadline) {
    u32 seen = event_count();
    while (!poll_ready(set, count)) {
        if (!event_wait(seen, deadline)) {
            return 0;
        }
        seen = event_count();
    }
    return 1;
}

static int l_rpoll(lua_State *L) {
    const struct poll_entry entry = {check_fd(L, 1), FS_POLL_IN};
    const int result = fs_poll(entry.fd);
    if (result < 0) {
        return fail_code(L, result);
    }
    lua_pushinteger(L, poll_wait(&entry, 1, deadline_ms(luaL_optinteger(L, 2, -1))) ? 1 : 0);
    return 1;
}

static int l_poll(lua_State *L) {
    luaL_checktype(L, 1, LUA_TTABLE);
    const u64 deadline = deadline_ms(luaL_optinteger(L, 2, -1));
    int count = 0;
    for (lua_pushnil(L); lua_next(L, 1) != 0; lua_pop(L, 1)) {
        count++;
    }
    struct poll_entry *set = lua_newuserdatauv(L, count * sizeof(*set), 0);
    for (int i = 0; i < count; i++) {
        set[i] = (struct poll_entry) {-1, 0};
    }
    int i = 0;
    for (lua_pushnil(L); i < count && lua_next(L, 1) != 0; lua_pop(L, 1), i++) {
        const lua_Integer fd = lua_isinteger(L, -2) ? lua_tointeger(L, -2) : -1;
        set[i].fd = fd >= 0 && fd <= 0x7FFFFFFF ? (int) fd : -1;
        set[i].events = 0;
        if (lua_getfield(L, -1, "events") == LUA_TTABLE) {
            set[i].events = (lua_getfield(L, -1, "IN") != LUA_TNIL && lua_toboolean(L, -1) ? FS_POLL_IN : 0)
                | (lua_getfield(L, -2, "OUT") != LUA_TNIL && lua_toboolean(L, -1) ? FS_POLL_OUT : 0);
            lua_pop(L, 2);
        }
        lua_pop(L, 1);
    }
    poll_wait(set, count, deadline);

    int ready = 0;
    i = 0;
    for (lua_pushnil(L); i < count && lua_next(L, 1) != 0; lua_pop(L, 1), i++) {
        const int events = poll_ready_events(&set[i]);
        const int valid = events >= 0;
        const int in = valid && (events & FS_POLL_IN);
        const int out = valid && (events & FS_POLL_OUT);
        lua_createtable(L, 0, 3);
        lua_pushboolean(L, in);
        lua_setfield(L, -2, "IN");
        lua_pushboolean(L, out);
        lua_setfield(L, -2, "OUT");
        lua_pushboolean(L, !valid);
        lua_setfield(L, -2, "NVAL");
        lua_setfield(L, -2, "revents");
        ready += in || out || !valid;
    }
    lua_pushinteger(L, ready);
    return 1;
}

static const luaL_Reg poll_lib[] = {
    {"rpoll", l_rpoll},
    {"poll", l_poll},
    {NULL, NULL},
};

int luaopen_posix_poll(lua_State *L) {
    luaL_newlib(L, poll_lib);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.time */

static int l_clock_gettime(lua_State *L) {
    const u64 ticks = time_now();
    lua_createtable(L, 0, 2);
    lua_pushinteger(L, (lua_Integer) (ticks / time_frequency()));
    lua_setfield(L, -2, "tv_sec");
    lua_pushinteger(L, (lua_Integer) ((ticks % time_frequency()) * 1000000000ull / time_frequency()));
    lua_setfield(L, -2, "tv_nsec");
    return 1;
}

static int l_nanosleep(lua_State *L) {
    luaL_checktype(L, 1, LUA_TTABLE);
    lua_getfield(L, 1, "tv_sec");
    lua_getfield(L, 1, "tv_nsec");
    const u64 sec = (u64) luaL_optinteger(L, -2, 0);
    const u64 nsec = (u64) luaL_optinteger(L, -1, 0);
    sleep_until(time_now() + sec * time_frequency() + nsec * (time_frequency() / 1000000) / 1000);
    lua_pushinteger(L, 0);
    return 1;
}

static const luaL_Reg time_lib[] = {
    {"clock_gettime", l_clock_gettime},
    {"nanosleep", l_nanosleep},
    {NULL, NULL},
};

int luaopen_posix_time(lua_State *L) {
    luaL_newlib(L, time_lib);
    set_constant(L, "CLOCK_REALTIME", 0);
    set_constant(L, "CLOCK_MONOTONIC", 1);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.signal, posix.sys.stat, posix.sys.socket */

static int l_signal(lua_State *L) {
    (void) L;
    return 0;
}

static const luaL_Reg signal_lib[] = {
    {"signal", l_signal},
    {NULL, NULL},
};

int luaopen_posix_signal(lua_State *L) {
    luaL_newlib(L, signal_lib);
    set_constant(L, "SIGPIPE", 13);
    set_constant(L, "SIG_IGN", 1);
    set_constant(L, "SIG_DFL", 0);
    return 1;
}

static const luaL_Reg stat_lib[] = {
    {"chmod", not_supported},
    {NULL, NULL},
};

int luaopen_posix_sys_stat(lua_State *L) {
    luaL_newlib(L, stat_lib);
    return 1;
}

static const luaL_Reg socket_lib[] = {
    {"socket", not_supported},
    {"bind", not_supported},
    {"connect", not_supported},
    {"listen", not_supported},
    {"accept", not_supported},
    {"getsockopt", not_supported},
    {NULL, NULL},
};

int luaopen_posix_sys_socket(lua_State *L) {
    luaL_newlib(L, socket_lib);
    set_constant(L, "AF_UNIX", 1);
    set_constant(L, "SOCK_STREAM", 1);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.dirent */

struct fs_listing *push_listing(lua_State *L);

static int l_dir(lua_State *L) {
    const char *path = luaL_checkstring(L, 1);
    struct fs_listing *listing = push_listing(L);
    const int result = fs_list(path, listing);
    if (result != 0) {
        return luaL_error(L, "%s: %s", path, strerror(-result));
    }
    lua_createtable(L, 8, 0);
    lua_pushliteral(L, ".");
    lua_rawseti(L, -2, 1);
    lua_pushliteral(L, "..");
    lua_rawseti(L, -2, 2);
    int n = 2;
    for (size_t i = 0; i < listing->count; i++) {
        lua_pushlstring(L, listing->entries[i].name, listing->entries[i].name_length);
        lua_rawseti(L, -2, ++n);
    }
    return 1;
}

static const luaL_Reg dirent_lib[] = {
    {"dir", l_dir},
    {NULL, NULL},
};

int luaopen_posix_dirent(lua_State *L) {
    luaL_newlib(L, dirent_lib);
    return 1;
}
