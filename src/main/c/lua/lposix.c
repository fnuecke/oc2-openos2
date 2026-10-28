/* SPDX-License-Identifier: MIT */
/*
 * The subset of luaposix that oc2's Lua libraries use, so they run unchanged.
 * File descriptors are virtio-console ports, opened as /dev/<port name> or /dev/vport0p<n>,
 * and serial ports, opened as /dev/ttyS<n>.
 */

#include <errno.h>
#include <string.h>
#include "drivers/serial.h"
#include "drivers/virtio/virtio.h"
#include "fs/fs.h"
#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"

#define O_RDONLY_ 0
#define O_WRONLY_ 1
#define O_RDWR_ 2
#define O_NONBLOCK_ 04000
#define O_CLOEXEC_ 02000000

#define FD_BASE 3
#define SERIAL_FD_BASE (FD_BASE + VCON_MAX_PORTS)
#define READ_MAX 4096
#define POLL_MAX 8

/* Registry table that keeps strings alive while the host still reads them, see vcon_send. */
#define IN_FLIGHT "openos2.posix.inflight"

static int fail_errno(lua_State *L, const int number) {
    const char *message;
    switch (number) {
        case EAGAIN: message = "Resource temporarily unavailable"; break;
        case ENOENT: message = "No such file or directory"; break;
        case EBADF: message = "Bad file descriptor"; break;
        default: message = "Function not implemented"; break;
    }
    luaL_pushfail(L);
    lua_pushstring(L, message);
    lua_pushinteger(L, number);
    return 3;
}

static int not_supported(lua_State *L) {
    return fail_errno(L, ENOSYS);
}

static int port_of(lua_State *L, const int index) {
    const lua_Integer fd = luaL_checkinteger(L, index);
    return fd >= FD_BASE && fd < FD_BASE + VCON_MAX_PORTS ? (int) (fd - FD_BASE) : -1;
}

static int serial_of_fd(const lua_Integer fd) {
    return fd >= SERIAL_FD_BASE && fd < SERIAL_FD_BASE + serial_count() ? (int) (fd - SERIAL_FD_BASE) : -1;
}

static int serial_of(lua_State *L, const int index) {
    return serial_of_fd(luaL_checkinteger(L, index));
}

static int valid_fd(lua_State *L, const int index) {
    return port_of(L, index) >= 0 || serial_of(L, index) >= 0;
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
    luaL_optinteger(L, 2, 0);
    if (strncmp(path, "/dev/", 5) != 0) {
        return fail_errno(L, ENOENT);
    }
    const char *name = path + 5;
    const int serial = serial_find(name);
    if (serial >= 0) {
        lua_pushinteger(L, SERIAL_FD_BASE + serial);
        return 1;
    }
    const int port = vcon_open(name, time_now() + time_frequency());
    if (port < 0 || port >= VCON_MAX_PORTS) {
        return fail_errno(L, ENOENT);
    }
    lua_pushinteger(L, FD_BASE + port);
    return 1;
}

static int l_fcntl(lua_State *L) {
    if (!valid_fd(L, 1)) {
        return fail_errno(L, EBADF);
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
    set_constant(L, "O_RDONLY", O_RDONLY_);
    set_constant(L, "O_WRONLY", O_WRONLY_);
    set_constant(L, "O_RDWR", O_RDWR_);
    set_constant(L, "O_NONBLOCK", O_NONBLOCK_);
    set_constant(L, "O_CLOEXEC", O_CLOEXEC_);
    set_constant(L, "F_GETFD", 1);
    set_constant(L, "F_SETFD", 2);
    set_constant(L, "F_GETFL", 3);
    set_constant(L, "F_SETFL", 4);
    set_constant(L, "FD_CLOEXEC", 1);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* posix.unistd */

static int l_read(lua_State *L) {
    const int port = port_of(L, 1);
    const int serial = serial_of(L, 1);
    lua_Integer count = luaL_checkinteger(L, 2);
    if (port < 0 && serial < 0) {
        return fail_errno(L, EBADF);
    }
    if (count > READ_MAX) {
        count = READ_MAX;
    }
    luaL_Buffer b;
    char *buffer = luaL_buffinitsize(L, &b, (size_t) count);
    const long got = count <= 0 ? 0
        : serial >= 0 ? (long) serial_read(serial, buffer, (size_t) count)
        : vcon_read(port, buffer, (size_t) count, 0);
    if (got == 0 && count > 0) {
        return fail_errno(L, EAGAIN);
    }
    luaL_pushresultsize(&b, (size_t) got);
    return 1;
}

static void reclaim(lua_State *L, const int port) {
    luaL_getsubtable(L, LUA_REGISTRYINDEX, IN_FLIGHT);
    int head;
    while ((head = vcon_reclaim(port)) >= 0) {
        lua_pushnil(L);
        lua_rawseti(L, -2, port * 256 + head);
    }
    lua_pop(L, 1);
}

static int l_write(lua_State *L) {
    const int port = port_of(L, 1);
    const int serial = serial_of(L, 1);
    size_t len;
    const char *data = luaL_checklstring(L, 2, &len);
    if (serial >= 0) {
        const size_t written = serial_write(serial, data, len);
        if (written == 0 && len > 0) {
            return fail_errno(L, EAGAIN);
        }
        lua_pushinteger(L, (lua_Integer) written);
        return 1;
    }
    if (port < 0) {
        return fail_errno(L, EBADF);
    }
    reclaim(L, port);
    if (len == 0) {
        lua_pushinteger(L, 0);
        return 1;
    }
    const int head = vcon_send(port, data, len);
    if (head < 0) {
        return fail_errno(L, EAGAIN);
    }
    luaL_getsubtable(L, LUA_REGISTRYINDEX, IN_FLIGHT);
    lua_pushvalue(L, 2);
    lua_rawseti(L, -2, port * 256 + head);
    lua_pop(L, 1);
    lua_pushinteger(L, (lua_Integer) len);
    return 1;
}

static int l_close(lua_State *L) {
    if (!valid_fd(L, 1)) {
        return fail_errno(L, EBADF);
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

struct poll_set {
    int count;
    lua_Integer fds[POLL_MAX];
    int want_in[POLL_MAX];
    int want_out[POLL_MAX];
};

static int fd_valid(const lua_Integer fd) {
    return (fd >= FD_BASE && fd < FD_BASE + VCON_MAX_PORTS) || serial_of_fd(fd) >= 0;
}

static int fd_readable(const lua_Integer fd) {
    const int serial = serial_of_fd(fd);
    return serial >= 0 ? serial_readable(serial) : vcon_readable((int) (fd - FD_BASE));
}

static int fd_writable(const lua_Integer fd) {
    const int serial = serial_of_fd(fd);
    return serial >= 0 ? serial_writable(serial) : vcon_writable((int) (fd - FD_BASE));
}

static bool poll_ready(void *ctx) {
    const struct poll_set *set = ctx;
    for (int i = 0; i < set->count; i++) {
        if (!fd_valid(set->fds[i])
            || (set->want_in[i] && fd_readable(set->fds[i]))
            || (set->want_out[i] && fd_writable(set->fds[i]))) {
            return 1;
        }
    }
    return 0;
}

static int l_rpoll(lua_State *L) {
    struct poll_set set = {1, {luaL_checkinteger(L, 1)}, {1}, {0}};
    if (!fd_valid(set.fds[0])) {
        return fail_errno(L, EBADF);
    }
    lua_pushinteger(L, wait_until(poll_ready, &set, deadline_ms(luaL_optinteger(L, 2, -1))) ? 1 : 0);
    return 1;
}

static int l_poll(lua_State *L) {
    luaL_checktype(L, 1, LUA_TTABLE);
    struct poll_set set = {0};
    lua_pushnil(L);
    while (lua_next(L, 1) != 0 && set.count < POLL_MAX) {
        set.fds[set.count] = lua_isinteger(L, -2) ? lua_tointeger(L, -2) : -1;
        lua_getfield(L, -1, "events");
        if (lua_istable(L, -1)) {
            set.want_in[set.count] = lua_getfield(L, -1, "IN") != LUA_TNIL && lua_toboolean(L, -1);
            set.want_out[set.count] = lua_getfield(L, -2, "OUT") != LUA_TNIL && lua_toboolean(L, -1);
            lua_pop(L, 2);
        }
        lua_pop(L, 2);
        set.count++;
    }
    wait_until(poll_ready, &set, deadline_ms(luaL_optinteger(L, 2, -1)));

    int ready = 0;
    int i = 0;
    lua_pushnil(L);
    while (lua_next(L, 1) != 0 && i < set.count) {
        const int valid = fd_valid(set.fds[i]);
        const int in = valid && set.want_in[i] && fd_readable(set.fds[i]);
        const int out = valid && set.want_out[i] && fd_writable(set.fds[i]);
        lua_createtable(L, 0, 3);
        lua_pushboolean(L, in);
        lua_setfield(L, -2, "IN");
        lua_pushboolean(L, out);
        lua_setfield(L, -2, "OUT");
        lua_pushboolean(L, !valid);
        lua_setfield(L, -2, "NVAL");
        lua_setfield(L, -2, "revents");
        ready += in || out || !valid;
        lua_pop(L, 1);
        i++;
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
        return luaL_error(L, "%s: %s", path, fs_error(result));
    }
    lua_createtable(L, 8, 0);
    lua_pushliteral(L, ".");
    lua_rawseti(L, -2, 1);
    lua_pushliteral(L, "..");
    lua_rawseti(L, -2, 2);
    int n = 2;
    for (size_t at = 0; at < listing->length; at += 2 + listing->data[at + 1]) {
        lua_pushlstring(L, (const char *) listing->data + at + 2, listing->data[at + 1]);
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
