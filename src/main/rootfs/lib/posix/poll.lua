local event = require("event")
local kernel = require("kernel.poll")
local BLOCKED = require("system").BLOCKED

local poll = {}

local function wakes(name)
  return name == "port" or name == "serial" or name == "bus"
end

local function inThread()
  local threads = package.loaded.thread
  return threads and threads.current()
end

function poll.canWait()
  return not inThread() or coroutine.isyieldable()
end

function poll.wake(deadline)
  if deadline and computer.uptime() >= deadline then
    return false
  end
  event.pullFiltered(deadline and deadline - computer.uptime(), wakes)
  return true
end

local function wait(check, target, timeout)
  timeout = timeout or -1
  if timeout ~= 0 and inThread() then
    coroutine.yield(BLOCKED, "wait", 0)
  end
  local deadline = timeout >= 0 and computer.uptime() + timeout / 1000 or nil
  repeat
    local ready, reason, number = check(target, 0)
    if ready ~= 0 then
      return ready, reason, number
    end
  until timeout == 0 or not poll.wake(deadline)
  return 0
end

function poll.rpoll(fd, timeout)
  if not poll.canWait() then
    return kernel.rpoll(fd, timeout)
  end
  return wait(kernel.rpoll, fd, timeout)
end

function poll.poll(fds, timeout)
  if not poll.canWait() then
    return kernel.poll(fds, timeout)
  end
  return wait(kernel.poll, fds, timeout)
end

return poll
