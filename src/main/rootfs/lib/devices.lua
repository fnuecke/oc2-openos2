-- Shadows oc2's devices.lua for our single process, multiple "threads" approach. Avoids threads
-- breaking each others' calls by making them wait using a single lock.
local bus = require("oc2.bus")
local poll = require("posix.poll")
local system = require("system")

local shared, reason = bus.connect({socketPath = "none"})
assert(shared, "could not open the device bus: " .. tostring(reason))

local DeviceBus = bus.DeviceBus
local FOREGROUND = {}
local NO_DEVICE = "00000000-0000-0000-0000-000000000000"

local queues = setmetatable({}, {__mode = "k"})
local owner, ownerThread, depth = nil, nil, 0
local lockWaiters, eventWaiters = {}, 0
local interrupted = false -- the last call ended early, so the host may have part of a message or payload

local function currentThread()
  local threads = package.loaded.thread
  return threads and threads.current() or FOREGROUND
end

local function alive(co, thread)
  return coroutine.status(co) ~= "dead" and (thread == FOREGROUND or thread:status() ~= "dead")
end

local function queue()
  local streams = system.streams and system.streams()
  local key = streams and (streams.process or streams) or FOREGROUND
  local q = queues[key]
  if not q then
    q = {}
    queues[key] = q
  end
  return q
end

local function remove(list, value)
  for i, other in ipairs(list) do
    if other == value then
      table.remove(list, i)
      return
    end
  end
end

local function release()
  owner, ownerThread = nil, nil
  while lockWaiters[1] and not owner do
    local next = table.remove(lockWaiters, 1)
    if alive(next.co, next.thread) then
      owner, ownerThread, depth = next.co, next.thread, 1
      computer.pushSignal("bus")
    end
  end
end

local held = setmetatable({}, {__close = function()
  depth = depth - 1
  if depth == 0 then
    release()
  end
end})

local function acquire()
  local co = coroutine.running()
  if owner == co then
    depth = depth + 1
    return held
  end
  if owner and not alive(owner, ownerThread) then
    release()
  end
  if not owner then
    owner, ownerThread, depth = co, currentThread(), 1
    return held
  end
  if not poll.canWait() then
    error("the device bus is in use by another thread", 0)
  end
  local me = {co = co, thread = currentThread()}
  lockWaiters[#lockWaiters + 1] = me
  local acquired = false
  local _ <close> = setmetatable({}, {__close = function()
    remove(lockWaiters, me)
    if owner == co and not acquired then
      release()
    end
  end})
  while owner ~= co do
    if not alive(owner, ownerThread) then
      release()
    else
      poll.wake(computer.uptime() + 1)
    end
  end
  acquired = true
  return held
end

local function resync()
  shared:flush()
  shared.requestId = (shared.requestId or 0) % 0x7FFFFFFF + 1
  local id = shared.requestId
  shared.rpc:write({type = "methods", data = NO_DEVICE, id = id})
  local fds = {[shared.rpc.fd] = {events = {IN = true}}, [shared.payload.fd] = {events = {IN = true}}}
  local deadline = computer.uptime() + bus.requestTimeout / 1000
  while computer.uptime() < deadline do
    shared.payload:reset()
    local reply = shared.rpc:read(0)
    if reply and reply.id == id then
      return
    elseif not reply then
      poll.poll(fds, math.max(0, math.ceil((deadline - computer.uptime()) * 1000)))
    end
  end
end

local function finish(...)
  interrupted = false
  return ...
end

local function locked(method)
  return function(self, ...)
    queue()
    local _ <close> = acquire()
    if depth > 1 then
      return method(self, ...)
    end
    if interrupted then
      resync()
    end
    interrupted = true
    return finish(method(self, ...))
  end
end

for _, name in ipairs({"list", "get", "find", "methods", "invoke"}) do
  shared[name] = locked(DeviceBus[name])
end

local function drain()
  local count = 0
  while true do
    local event = DeviceBus.waitEvent(shared, 0)
    if not event then
      break
    end
    count = count + 1
    for _, q in pairs(queues) do
      if #q >= bus.maxPendingEvents then
        table.remove(q, 1)
      end
      q[#q + 1] = event
    end
  end
  if count > 0 and eventWaiters > 0 then
    computer.pushSignal("bus")
  end
end

function shared:pumpEvents()
  local count = DeviceBus.pumpEvents(self)
  drain()
  return count
end

local eventWait = setmetatable({}, {__close = function()
  eventWaiters = eventWaiters - 1
end})

function shared:waitEvent(timeout, eventType)
  local q = queue()
  local deadline = timeout and computer.uptime() + timeout / 1000
  while true do
    drain()
    while q[1] do
      local event = table.remove(q, 1)
      if not eventType or event.type == eventType then
        return event
      end
    end
    if not poll.canWait() then
      local remaining = deadline and math.floor(math.max(0, deadline - computer.uptime()) * 1000)
      local event, reason = DeviceBus.waitEvent(self, remaining)
      if not event then
        return nil, reason
      end
      table.insert(self.pendingEvents, 1, event)
    else
      if deadline and computer.uptime() >= deadline then
        return nil, "timeout"
      end
      eventWaiters = eventWaiters + 1
      local _ <close> = eventWait
      poll.wake(math.min(deadline or math.huge, computer.uptime() + 1))
    end
  end
end

return shared
