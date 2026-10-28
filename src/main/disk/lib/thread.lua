-- Background threads, loaded by the first thread.create. Threads run when the foreground waits for a
-- signal: computer.pullSignal then runs them until they wait too, and gives each a copy of the
-- signal it returns. Like OpenOS's threads, they are coroutines, scheduled cooperatively.

local event = require("event")
local shell = require("shell")
local system = require("system")
local term = require("term")

local thread = {}

local BLOCKED = system.BLOCKED
local threads = {}
local current
local nextId = 0
local rawPull = computer.pullSignal

local Thread = {}
Thread.__index = Thread

local function remove(t)
  for i, other in ipairs(threads) do
    if other == t then
      table.remove(threads, i)
      shell.cleanup(t.streams)
      return
    end
  end
end

-- Ctrl+C does not hit threads, see coroutine.setBackground: one that came while the thread ran is
-- raised here, in the foreground.
local function step(t, ...)
  local previous, interrupts = current, console.interrupts()
  current = t
  local active = system.streams(t.streams)
  local ok, blocked, kind, deadline = system.rawresume(t.co, ...)
  system.streams(active)
  current = previous
  if not ok then
    t.error = blocked
    remove(t)
    if term.powerRequest(blocked) then
      error(blocked, 0)
    end
    io.stderr:write("thread ", t.id, ": ", tostring(blocked), "\n")
  elseif coroutine.status(t.co) == "dead" then
    remove(t)
  else
    t.deadline = blocked == BLOCKED and kind == "wait" and deadline or 0
  end
  if console.interrupts() ~= interrupts then
    error("interrupted!", 0)
  end
end

local function runDue()
  local now = computer.uptime()
  for _, t in ipairs({table.unpack(threads)}) do
    if t.deadline <= now then
      step(t)
    end
  end
end

function computer.pullSignal(timeout)
  if current then
    return coroutine.yield(BLOCKED, "wait", timeout and computer.uptime() + timeout or math.huge)
  end
  local deadline = timeout and computer.uptime() + timeout or math.huge
  while true do
    runDue()
    local wake = deadline
    for _, t in ipairs(threads) do
      wake = math.min(wake, t.deadline)
    end
    local signal = table.pack(rawPull(wake < math.huge and math.max(0, wake - computer.uptime()) or nil))
    if signal.n > 0 then
      for _, t in ipairs({table.unpack(threads)}) do
        step(t, table.unpack(signal, 1, signal.n))
      end
      return table.unpack(signal, 1, signal.n)
    end
    if computer.uptime() >= deadline then
      return
    end
  end
end

function computer.sleep(seconds)
  local deadline = computer.uptime() + (seconds or 0)
  repeat
    event.pull(math.max(0, deadline - computer.uptime()), "thread sleep")
  until computer.uptime() >= deadline
end
os.sleep = computer.sleep

function thread.create(fn, ...)
  local args = table.pack(...)
  local caller = system.streams()
  nextId = nextId + 1
  local t = setmetatable({
    id = nextId,
    parent = caller.process and caller.process.pid or 0,
    name = caller.process and caller.process.command or "thread",
    streams = {input = caller.input, output = caller.output, errors = caller.errors, lastCode = caller.lastCode},
    deadline = 0,
    co = coroutine.create(function()
      return fn(table.unpack(args, 1, args.n))
    end),
  }, Thread)
  coroutine.setBackground(t.co)
  threads[#threads + 1] = t
  return t
end

function thread.list()
  return {table.unpack(threads)}
end

function thread.current()
  return current
end

function Thread:status()
  if self == current then
    return "running"
  end
  return coroutine.status(self.co) == "dead" and "dead" or "waiting"
end

function Thread:kill()
  remove(self)
  self.co = coroutine.create(function() end)
  coroutine.resume(self.co)
end

function Thread:join(timeout)
  local deadline = timeout and computer.uptime() + timeout or math.huge
  while self:status() ~= "dead" and computer.uptime() < deadline do
    event.pull(math.min(0.05, deadline - computer.uptime()), "thread join")
  end
  return self:status() == "dead"
end

return thread
