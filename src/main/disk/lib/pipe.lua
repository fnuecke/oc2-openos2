-- Pipes between the stages of a shell pipeline, loaded by the first one. Each stage runs in its own
-- coroutine with its own streams. Writing to a full pipe and reading from an empty one yield
-- system.BLOCKED, which coroutine.resume passes up to the scheduler in pipe.run, which then runs the
-- other stages. When no stage can go on, the scheduler passes the yield up to its own resumer, e.g.
-- an outer pipeline or the thread it runs in.

local shell = require("shell")
local system = require("system")

local pipe = {}

local CAPACITY = 4096
local BROKEN = {exit = 141}
local BLOCKED = system.BLOCKED

local transfers = 0

local Reader, Writer = {}, {}
Reader.__index, Writer.__index = Reader, Writer

function pipe.create()
  local state = {data = "", offset = 1, queue = {}, queued = 0, writerClosed = false, readerClosed = false}
  return setmetatable({pipe = state}, Reader), setmetatable({pipe = state}, Writer)
end

local function available(state)
  return #state.data - state.offset + 1 + state.queued
end

function Writer:write(...)
  local state = self.pipe
  for i = 1, select("#", ...) do
    while available(state) >= CAPACITY and not state.readerClosed do
      coroutine.yield(BLOCKED)
    end
    if state.readerClosed then
      error(BROKEN, 0)
    end
    local text = tostring((select(i, ...)))
    state.queue[#state.queue + 1] = text
    state.queued = state.queued + #text
    transfers = transfers + 1
  end
  return self
end

function Writer:close()
  self.pipe.writerClosed = true
  transfers = transfers + 1
  return true
end

function Writer:flush()
  return self
end

function Writer:setvbuf()
  return true
end

function Writer:seek()
  return nil, "Illegal seek"
end

local function fill(state)
  if state.queued > 0 then
    local queue = state.queue
    state.data = state.data:sub(state.offset) .. table.concat(queue)
    state.offset, state.queued = 1, 0
    for i = #queue, 1, -1 do
      queue[i] = nil
    end
  end
end

local function take(state, count)
  local taken = state.data:sub(state.offset, state.offset + count - 1)
  state.offset = state.offset + #taken
  transfers = transfers + 1
  return taken
end

local function await(state, ready, arg)
  fill(state)
  while not ready(state, arg) and not state.writerClosed do
    coroutine.yield(BLOCKED)
    fill(state)
  end
  return available(state) > 0
end

local function hasLine(state)
  return state.data:find("\n", state.offset, true) ~= nil
end

local function hasBytes(state, count)
  return available(state) >= count
end

local function never()
  return false
end

local FORMATS = {l = "l", L = "L", a = "a", ["*l"] = "l", ["*L"] = "L", ["*a"] = "a"}

local function readOne(state, format)
  if type(format) == "number" then
    if not await(state, hasBytes, format) then
      return format == 0 and "" or nil
    end
    return take(state, format)
  end
  format = FORMATS[format or "l"] or (format or "l"):gsub("^%*", ""):sub(1, 1)
  if format == "a" then
    await(state, never)
    return take(state, available(state))
  elseif format == "l" or format == "L" then
    if not await(state, hasLine) then
      return nil
    end
    local newline = state.data:find("\n", state.offset, true)
    local line = take(state, newline and newline - state.offset + 1 or available(state))
    return format == "l" and newline and line:sub(1, -2) or line
  end
  error("bad argument #1 to 'read' (invalid format)")
end

function Reader:read(...)
  local count = select("#", ...)
  if count <= 1 then
    return readOne(self.pipe, (...))
  end
  local results = {}
  for i = 1, count do
    results[i] = readOne(self.pipe, (select(i, ...)))
    if results[i] == nil then
      return table.unpack(results, 1, i)
    end
  end
  return table.unpack(results, 1, count)
end

function Reader:lines(format)
  return function()
    return readOne(self.pipe, format)
  end
end

function Reader:close()
  self.pipe.readerClosed = true
  transfers = transfers + 1
  return true
end

function Reader:seek()
  return nil, "Illegal seek"
end

local function step(stage)
  local active = system.streams(stage.streams)
  local resume = stage.resume or {n = 0}
  stage.resume = nil
  local result = table.pack(system.rawresume(stage.co, table.unpack(resume, 1, resume.n)))
  system.streams(active)
  if not result[1] then
    error(result[2], 0)
  end
  if coroutine.status(stage.co) == "dead" then
    stage.code = result[2]
    if stage.writer then
      stage.writer:close()
    end
    if stage.reader then
      stage.reader:close()
    end
  elseif result[2] == BLOCKED and result[3] == "wait" then
    stage.waiting = table.pack(table.unpack(result, 2, result.n))
  end
end

function pipe.run(commands)
  local parent = system.streams()
  local stages, input = {}, parent.input
  for i, command in ipairs(commands) do
    local stage = {streams = {input = input, output = parent.output, errors = parent.errors,
      process = parent.process, lastCode = parent.lastCode}}
    if i < #commands then
      local reader, writer = pipe.create()
      stage.streams.output, stage.writer, input = writer, writer, reader
    end
    stage.reader = i > 1 and stage.streams.input or nil
    stage.co = coroutine.create(function()
      return shell.execute(command)
    end)
    stages[i] = stage
  end
  local interrupts, interrupted = console.interrupts(), false
  local ok, err = system.rawpcall(function()
    while true do
      local before, running, waiting = transfers, 0, nil
      for _, stage in ipairs(stages) do
        if coroutine.status(stage.co) == "suspended" and not stage.waiting then
          step(stage)
          if stage.code == 130 or console.interrupts() ~= interrupts then
            interrupted = true
            return
          end
        end
        if coroutine.status(stage.co) == "suspended" then
          running = running + 1
          if stage.waiting and (not waiting or stage.waiting[3] < waiting[3]) then
            waiting = stage.waiting
          end
        end
      end
      if running == 0 then
        return
      end
      if transfers == before and coroutine.isyieldable() then
        waiting = waiting or {BLOCKED, n = 1}
        local resumed = table.pack(coroutine.yield(table.unpack(waiting, 1, waiting.n)))
        for _, stage in ipairs(stages) do
          if stage.waiting then
            stage.waiting, stage.resume = nil, resumed
          end
        end
      end
    end
  end)
  for _, stage in ipairs(stages) do
    shell.cleanup(stage.streams)
  end
  if not ok then
    error(err, 0)
  end
  return interrupted and 130 or stages[#stages].code or 0
end

return pipe
