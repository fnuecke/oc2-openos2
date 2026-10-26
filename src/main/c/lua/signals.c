/* SPDX-License-Identifier: MIT */
/*
 * Signals, Ctrl+C and the watchdog. computer.pullSignal waits for the next signal: kernel signals
 * ("console", "port", "medium") and those queued with computer.pushSignal, in order. Kernel signals
 * enter the queue when collected: in pullSignal, and in pushSignal before the pushed one. A collection
 * queues medium signals, then console input, then ports, then serial ports. Ctrl+C drops queued
 * console input.
 *
 * Ctrl+C and the watchdog work like lua.c's SIGINT handler: the interrupt sets a hook on the
 * running coroutine, which raises the error at its next instruction. The running coroutine is
 * tracked by coroutine.resume and coroutine.wrap, which this replaces.
 */

#include <string.h>
#include "drivers/console.h"
#include "drivers/serial.h"
#include "drivers/virtio/virtio.h"
#include "fs/fs.h"
#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"

#define QUEUE "openos2.signals" /* registry table: first, last and the packed signals between */
#define MAX_QUEUED 64
#define MAX_ARGUMENTS 16
#define MEDIA_POLL_MS 1000
#define POWER_HOOK_COUNT 100000 /* instructions between checks of an unanswered power request */
#define CONSOLE_CHUNK 256

static const char INTERRUPTED[] = "interrupted!";
static const char WATCHDOG[] = "too long without yielding";
static const char SHUTDOWN[] = "shutdown requested";
static const char REBOOT[] = "reboot requested";
static int power_signaled; /* the "shutdown" or "reboot" signal is queued */

static lua_State *main_state;
static lua_State *running; /* the coroutine running now, see resume */
static int background_depth; /* background coroutines in the chain of resumes, see l_set_background */
static volatile int interrupt_deferred; /* Ctrl+C came while background code ran */
static const char *volatile pending_error;

/* ---------------------------------------------------------------------------------------- */
/* Interrupts */

static void drop_console_signals(lua_State *L);

static void power_hook(lua_State *L, lua_Debug *ar) {
    (void) L;
    (void) ar;
    power_check();
}

static void interrupt_hook(lua_State *L, lua_Debug *ar) {
    (void) ar;
    const int power = pending_error == SHUTDOWN || pending_error == REBOOT;
    lua_sethook(L, power ? power_hook : NULL, power ? LUA_MASKCOUNT : 0, POWER_HOOK_COUNT);
    console_clear_interrupt();
    if (pending_error == INTERRUPTED) {
        drop_console_signals(L); /* typed ahead of the Ctrl+C, like the input drivers/console.c still had */
    }
    lua_pushstring(L, pending_error); /* without a position, programs match the message */
    lua_error(L);
}

/* Called from interrupt handlers. lua_sethook is safe there, see lua.c. */
static void raise_in_running(const char *message) {
    pending_error = message;
    lua_sethook(running, interrupt_hook, LUA_MASKCALL | LUA_MASKRET | LUA_MASKCOUNT, 1);
}

static void on_interrupt(void) {
    if (background_depth > 0) {
        interrupt_deferred = 1;
    } else {
        raise_in_running(INTERRUPTED);
    }
}

static void on_watchdog(void) {
    raise_in_running(WATCHDOG);
}

static void on_power_request(void) {
    raise_in_running(power_requested() == POWER_OFF_REQUEST ? SHUTDOWN : REBOOT);
}

static int error_pending(lua_State *L) {
    return lua_gethook(L) == interrupt_hook;
}

/* ---------------------------------------------------------------------------------------- */
/* Coroutines, as in lcorolib.c, with the running coroutine tracked. */

static int resume(lua_State *L, lua_State *co, const int narg) {
    if (!lua_checkstack(co, narg)) {
        lua_pushliteral(L, "too many arguments to resume");
        return -1;
    }
    lua_xmove(L, co, narg);
    lua_State *previous = running;
    const bool background = background_depth > 0 || *(unsigned char *) lua_getextraspace(co) != 0;
    running = co;
    background_depth += background;
    int nres;
    const int status = lua_resume(co, L, narg, &nres);
    running = previous;
    background_depth -= background;
    if (background && background_depth == 0 && interrupt_deferred) {
        /* The resumer raises it, once it is done with the results, see l_set_background. */
        interrupt_deferred = 0;
        console_clear_interrupt();
        drop_console_signals(L);
    }
    if (error_pending(co)) {
        /* Interrupted just as it yielded or ended: the error goes to the resumer instead. */
        lua_sethook(co, NULL, 0, 0);
        lua_sethook(L, interrupt_hook, LUA_MASKCALL | LUA_MASKRET | LUA_MASKCOUNT, 1);
    }
    if (status == LUA_OK || status == LUA_YIELD) {
        if (!lua_checkstack(L, nres + 1)) {
            lua_pop(co, nres);
            lua_pushliteral(L, "too many results to resume");
            return -1;
        }
        lua_xmove(co, L, nres);
        return nres;
    }
    lua_xmove(co, L, 1);
    return -1;
}

static int l_resume(lua_State *L) {
    lua_State *co = lua_tothread(L, 1);
    luaL_argexpected(L, co, 1, "coroutine");
    const int r = resume(L, co, lua_gettop(L) - 1);
    if (r < 0) {
        lua_pushboolean(L, 0);
        lua_insert(L, -2);
        return 2;
    }
    lua_pushboolean(L, 1);
    lua_insert(L, -(r + 1));
    return r + 1;
}

static bool is_kernel_error(const char *message) {
    return strcmp(message, INTERRUPTED) == 0 || strcmp(message, WATCHDOG) == 0
        || strcmp(message, SHUTDOWN) == 0 || strcmp(message, REBOOT) == 0;
}

static int l_wrapped(lua_State *L) {
    lua_State *co = lua_tothread(L, lua_upvalueindex(1));
    const int r = resume(L, co, lua_gettop(L));
    if (r < 0) {
        int status = lua_status(co);
        if (status != LUA_OK && status != LUA_YIELD) {
            status = lua_closethread(co, L);
            lua_xmove(co, L, 1);
        }
        if (status != LUA_ERRMEM && lua_type(L, -1) == LUA_TSTRING && !is_kernel_error(lua_tostring(L, -1))) {
            luaL_where(L, 1);
            lua_insert(L, -2);
            lua_concat(L, 2);
        }
        return lua_error(L);
    }
    return r;
}

/* coroutine.setBackground(co): Ctrl+C does not hit co or the coroutines it resumes. The resumer of co
 * checks console.interrupts() and raises "interrupted!" itself, see /lib/thread.lua. */
static int l_set_background(lua_State *L) {
    lua_State *co = lua_tothread(L, 1);
    luaL_argexpected(L, co, 1, "coroutine");
    *(unsigned char *) lua_getextraspace(co) = 1;
    return 0;
}

static int l_wrap(lua_State *L) {
    luaL_checktype(L, 1, LUA_TFUNCTION);
    lua_State *co = lua_newthread(L);
    lua_pushvalue(L, 1);
    lua_xmove(L, co, 1);
    lua_pushcclosure(L, l_wrapped, 1);
    return 1;
}

/* ---------------------------------------------------------------------------------------- */
/* Signals */

static int queue_index(lua_State *L, const char *field) {
    lua_getfield(L, -1, field);
    const int value = (int) lua_tointeger(L, -1);
    lua_pop(L, 1);
    return value;
}

static int enqueue(lua_State *L, const int n) {
    const int queue = lua_gettop(L) - n;
    const int first = (lua_pushvalue(L, queue), queue_index(L, "first"));
    const int last = queue_index(L, "last");
    lua_pop(L, 1);
    if (last - first >= MAX_QUEUED) {
        lua_pop(L, n);
        return 0;
    }
    lua_createtable(L, n, 1);
    lua_insert(L, queue + 1);
    for (int i = n; i >= 1; i--) {
        lua_rawseti(L, queue + 1, i);
    }
    lua_pushinteger(L, n);
    lua_setfield(L, queue + 1, "n");
    lua_rawseti(L, queue, last);
    lua_pushinteger(L, last + 1);
    lua_setfield(L, queue, "last");
    return 1;
}

static void push_queue(lua_State *L) {
    if (luaL_getsubtable(L, LUA_REGISTRYINDEX, QUEUE) == 0) {
        lua_pushinteger(L, 1);
        lua_setfield(L, -2, "first");
        lua_pushinteger(L, 1);
        lua_setfield(L, -2, "last");
    }
}

static int queue_full(lua_State *L) {
    return queue_index(L, "last") - queue_index(L, "first") >= MAX_QUEUED;
}

static void collect_kernel_signals(lua_State *L) {
    fs_poll_media();
    push_queue(L);
    char path[48];
    const char *what;
    while (!queue_full(L) && fs_take_medium_event(path, sizeof(path), &what)) {
        lua_pushliteral(L, "medium");
        lua_pushstring(L, path);
        lua_pushstring(L, what);
        enqueue(L, 3);
    }
    if (console_has_input() && !queue_full(L)) {
        char input[CONSOLE_CHUNK];
        const size_t count = console_read(input, sizeof(input), 0);
        if (count > 0) {
            lua_pushliteral(L, "console");
            lua_pushlstring(L, input, count);
            enqueue(L, 2);
        }
    }
    while (vcon_signal_ready() && !queue_full(L)) {
        lua_pushliteral(L, "port");
        lua_pushstring(L, vcon_name(vcon_take_signal()));
        enqueue(L, 2);
    }
    while (serial_signal_ready() && !queue_full(L)) {
        lua_pushliteral(L, "serial");
        lua_pushfstring(L, "/dev/%s", serial_name(serial_take_signal()));
        enqueue(L, 2);
    }
    if (power_requested() != 0 && !power_signaled && !queue_full(L)) {
        lua_pushstring(L, power_requested() == POWER_OFF_REQUEST ? "shutdown" : "reboot");
        enqueue(L, 1);
        power_signaled = 1;
    }
    lua_pop(L, 1);
}

static void drop_console_signals(lua_State *L) {
    push_queue(L);
    const int first = queue_index(L, "first");
    const int last = queue_index(L, "last");
    int kept = first;
    for (int i = first; i < last; i++) {
        lua_rawgeti(L, -1, i);
        lua_rawgeti(L, -1, 1);
        const int console = lua_type(L, -1) == LUA_TSTRING && strcmp(lua_tostring(L, -1), "console") == 0;
        lua_pop(L, 1);
        if (console) {
            lua_pop(L, 1);
        } else {
            lua_rawseti(L, -2, kept++);
        }
    }
    for (int i = kept; i < last; i++) {
        lua_pushnil(L);
        lua_rawseti(L, -2, i);
    }
    lua_pushinteger(L, kept);
    lua_setfield(L, -2, "last");
    lua_pop(L, 1);
}

static int dequeue(lua_State *L) {
    push_queue(L);
    const int first = queue_index(L, "first");
    const int last = queue_index(L, "last");
    if (first == last) {
        lua_pop(L, 1);
        return -1;
    }
    lua_rawgeti(L, -1, first);
    lua_pushnil(L);
    lua_rawseti(L, -3, first);
    lua_pushinteger(L, first + 1);
    lua_setfield(L, -3, "first");
    lua_remove(L, -2);
    const int n = (int) (lua_getfield(L, -1, "n"), lua_tointeger(L, -1));
    lua_pop(L, 1);
    luaL_checkstack(L, n, "too many signal values");
    for (int i = 1; i <= n; i++) {
        lua_rawgeti(L, -i, i);
    }
    lua_remove(L, -(n + 1));
    return n;
}

static bool signal_ready(void *ctx) {
    (void) ctx;
    return console_has_input() || fs_medium_event_pending() || vcon_signal_ready() || serial_signal_ready()
        || (power_requested() != 0 && !power_signaled);
}

static u64 ticks(const lua_Number seconds) {
    const lua_Number value = seconds * (lua_Number) time_frequency();
    return !(value > 0) ? 0 : value >= 0x1p62 ? 1ull << 62 : (u64) value;
}

/* computer.pullSignal([timeout]) -> name, ... or nothing on timeout */
static int l_pull_signal(lua_State *L) {
    const u64 deadline = lua_isnoneornil(L, 1) ? TIME_NEVER : time_now() + ticks(luaL_checknumber(L, 1));
    lua_settop(L, 0);
    watchdog_restart(); /* pullSignal counts as a wait, even if it does not need to */
    for (;;) {
        collect_kernel_signals(L);
        const int n = dequeue(L);
        if (n >= 0) {
            return n;
        }
        const u64 now = time_now();
        if (now >= deadline || error_pending(L)) {
            return 0;
        }
        const u64 poll = now + time_frequency() / 1000 * MEDIA_POLL_MS;
        wait_until(signal_ready, NULL, poll < deadline ? poll : deadline);
        if (error_pending(L)) {
            return 0; /* Ctrl+C: the hook raises it as this returns */
        }
    }
}

/* computer.pushSignal(name, ...) -> whether it was queued */
static int l_push_signal(lua_State *L) {
    const int n = lua_gettop(L);
    luaL_checktype(L, 1, LUA_TSTRING);
    luaL_argcheck(L, n <= MAX_ARGUMENTS, MAX_ARGUMENTS + 1, "too many values");
    for (int i = 1; i <= n; i++) {
        const int type = lua_type(L, i);
        luaL_argexpected(L, type == LUA_TNIL || type == LUA_TBOOLEAN || type == LUA_TNUMBER || type == LUA_TSTRING,
            i, "nil, boolean, number or string");
    }
    collect_kernel_signals(L);
    push_queue(L);
    lua_insert(L, 1);
    lua_pushboolean(L, enqueue(L, n));
    return 1;
}

/* computer.watchdog(seconds | nil) */
static int l_watchdog(lua_State *L) {
    if (lua_isnoneornil(L, 1)) {
        watchdog_set(0, NULL);
    } else {
        const lua_Number seconds = luaL_checknumber(L, 1);
        luaL_argcheck(L, seconds > 0, 1, "must be positive");
        watchdog_set(ticks(seconds), on_watchdog);
    }
    return 0;
}

void signal_init(lua_State *L) {
    main_state = L;
    running = L;
    console_set_interrupt_handler(on_interrupt);
    power_set_interrupt_handler(on_power_request);

    lua_getglobal(L, "computer");
    lua_pushcfunction(L, l_pull_signal);
    lua_setfield(L, -2, "pullSignal");
    lua_pushcfunction(L, l_push_signal);
    lua_setfield(L, -2, "pushSignal");
    lua_pushcfunction(L, l_watchdog);
    lua_setfield(L, -2, "watchdog");
    lua_pop(L, 1);

    lua_getglobal(L, "coroutine");
    lua_pushcfunction(L, l_resume);
    lua_setfield(L, -2, "resume");
    lua_pushcfunction(L, l_wrap);
    lua_setfield(L, -2, "wrap");
    lua_pushcfunction(L, l_set_background);
    lua_setfield(L, -2, "setBackground");
    lua_pop(L, 1);
}

void signal_close(void) {
    console_set_interrupt_handler(NULL);
    power_set_interrupt_handler(NULL);
    watchdog_set(0, NULL);
}
