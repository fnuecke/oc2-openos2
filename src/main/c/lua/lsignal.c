/* SPDX-License-Identifier: MIT */

#include "kernel.h"
#include "lauxlib.h"
#include "lua.h"

#define QUEUE "openos2.signals"
#define MAX_QUEUED 64
#define MAX_ARGUMENTS 16
#define POWER_HOOK_COUNT 100000 /* instructions between checks of an unanswered power request */

static const char INTERRUPTED[] = "interrupted!";
static const char SHUTDOWN[] = "shutdown requested";
static const char REBOOT[] = "reboot requested";

static lua_State *running;
static int background_depth;
static volatile int interrupt_deferred;
static const char *volatile pending_error;

/* ---------------------------------------------------------------------------------------- */
/* Interrupts */

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
    lua_pushstring(L, pending_error); /* without position info, programs match the message text */
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

static void on_power_request(void) {
    raise_in_running(power_requested() == POWER_OFF_REQUEST ? SHUTDOWN : REBOOT);
}

static int error_pending(lua_State *L) {
    return lua_gethook(L) == interrupt_hook;
}

/* ---------------------------------------------------------------------------------------- */
/* Coroutines, as in lcorolib.c, tracking running coroutine for interrupt hooks. */

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

/* Mark for exclusion from regular Ctrl+C handling. Also see /lib/thread.lua. */
static int l_set_background(lua_State *L) {
    lua_State *co = lua_tothread(L, 1);
    luaL_argexpected(L, co, 1, "coroutine");
    *(unsigned char *) lua_getextraspace(co) = 1;
    return 0;
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
    push_queue(L);
    signal_collect();
    struct signal signal;
    while (!queue_full(L) && signal_next(&signal)) {
        lua_pushstring(L, signal.name);
        int n = 1;
        if (signal.value != NULL) {
            lua_pushlstring(L, signal.value, signal.value_length);
            n++;
        }
        for (size_t i = 0; i < signal.int_value_count; i++) {
            lua_pushinteger(L, signal.int_values[i]);
            n++;
        }
        if (signal.detail != NULL) {
            lua_pushstring(L, signal.detail);
            n++;
        }
        enqueue(L, n);
    }
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

static int l_pull_signal(lua_State *L) {
    const u64 deadline = lua_isnoneornil(L, 1) ? TIME_NEVER : time_now() + seconds_to_ticks(luaL_checknumber(L, 1));
    lua_settop(L, 0);
    for (;;) {
        collect_kernel_signals(L);
        const int n = dequeue(L);
        if (n >= 0) {
            return n;
        }

        if (time_now() >= deadline || error_pending(L)) {
            return 0;
        }

        signal_wait(deadline);
        if (error_pending(L)) {
            return 0;
        }
    }
}

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

void signal_init(lua_State *L) {
    running = L;
    console_set_interrupt_handler(on_interrupt);
    power_set_interrupt_handler(on_power_request);

    lua_getglobal(L, "computer");
    lua_pushcfunction(L, l_pull_signal);
    lua_setfield(L, -2, "pullSignal");
    lua_pushcfunction(L, l_push_signal);
    lua_setfield(L, -2, "pushSignal");
    lua_pop(L, 1);

    lua_getglobal(L, "coroutine");
    lua_pushcfunction(L, l_resume);
    lua_setfield(L, -2, "resume");
    lua_pushcfunction(L, l_set_background);
    lua_setfield(L, -2, "setBackground");
    lua_pop(L, 1);
}

void signal_close(void) {
    console_set_interrupt_handler(NULL);
    power_set_interrupt_handler(NULL);
}
