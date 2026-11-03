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

local function block()
  coroutine.yield(BLOCKED)
end

function pipe.create()
  local state = {data = "", offset = 1, queue = {}, queued = 0, writerClosed = false, readerClosed = false,
    readerWait = block, writerWait = block}
  return setmetatable({pipe = state}, Reader), setmetatable({pipe = state}, Writer)
end

local function available(state)
  return #state.data - state.offset + 1 + state.queued
end

function Writer:write(...)
  local state = self.pipe
  for i = 1, select("#", ...) do
    while available(state) >= CAPACITY and not state.readerClosed do
      state.writerWait()
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
  while not ready(state, arg) and available(state) < CAPACITY and not state.writerClosed do
    state.readerWait()
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

local MAX_NUMERAL = 200

local function numeral(state)
  local data, offset = state.data, state.offset
  local text, digits, exponent = data:match("^[+-]?0[xX]%x*%.?%x*", offset), "%x", "^[pP][+-]?%d*"
  if not text then
    text, digits, exponent = data:match("^[+-]?%d*%.?%d*", offset), "%d", "^[eE][+-]?%d*"
  end
  if text:gsub("^[+-]?0[xX]", ""):find(digits) then
    text = text .. (data:match(exponent, offset + #text) or "")
  end
  return text:sub(1, MAX_NUMERAL)
end

local function hasNonSpace(state)
  return state.data:find("%S", state.offset) ~= nil
end

local function hasNumeral(state)
  local length = #numeral(state)
  return length == MAX_NUMERAL or state.offset + length <= #state.data
end

local function readNumber(state)
  local start
  repeat
    local more = await(state, hasNonSpace)
    start = state.data:find("%S", state.offset)
    take(state, (start or #state.data + 1) - state.offset)
  until start or not more
  if not start then
    return nil
  end
  await(state, hasNumeral)
  return tonumber(take(state, #numeral(state)))
end

local function readBytes(state, count)
  if count == 0 then
    fill(state)
    return (available(state) > 0 or not state.writerClosed) and "" or nil
  end
  local parts
  while count > 0 and await(state, hasBytes, count) do
    local part = take(state, math.min(count, available(state)))
    count = count - #part
    if count == 0 and not parts then
      return part
    end
    parts = parts or {}
    parts[#parts + 1] = part
  end
  return parts and table.concat(parts)
end

local function readLine(state, keepNewline)
  local parts
  while await(state, hasLine) do
    local newline = state.data:find("\n", state.offset, true)
    local part = take(state, newline and newline - state.offset + 1 or available(state))
    if newline then
      part = keepNewline and part or part:sub(1, -2)
      return parts and table.concat(parts) .. part or part
    end
    parts = parts or {}
    parts[#parts + 1] = part
  end
  return parts and table.concat(parts)
end

local function readAll(state)
  local parts = {}
  while await(state, never) do
    parts[#parts + 1] = take(state, available(state))
  end
  return table.concat(parts)
end

local FORMATS = {l = "l", L = "L", a = "a", n = "n", ["*l"] = "l", ["*L"] = "L", ["*a"] = "a", ["*n"] = "n"}

local function readOne(state, format)
  if type(format) == "number" then
    return readBytes(state, format)
  end
  format = FORMATS[format or "l"] or (format or "l"):gsub("^%*", ""):sub(1, 1)
  if format == "n" then
    return readNumber(state)
  elseif format == "a" then
    return readAll(state)
  elseif format == "l" or format == "L" then
    return readLine(state, format == "L")
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

Reader.seek, Reader.setvbuf, Reader.flush = Writer.seek, Writer.setvbuf, Writer.flush

local function step(stage)
  local active = system.streams(stage.streams)
  local resume = stage.resume or {n = 0}
  stage.resume = nil
  local result = table.pack(system.rawresume(stage.co, table.unpack(resume, 1, resume.n)))
  system.streams(active)
  if not result[1] then
    coroutine.close(stage.co)
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

local function inherit(parent)
  return {input = parent.input, output = parent.output, errors = parent.errors, process = parent.process,
    lastCode = parent.lastCode}
end

function pipe.run(commands)
  local parent = system.streams()
  local stages, input = {}, parent.input
  for i, command in ipairs(commands) do
    local stage = {streams = inherit(parent)}
    stage.streams.input = input
    if i < #commands then
      local reader, writer = pipe.create()
      stage.streams.output, stage.writer, input = writer, writer, reader
    end
    stage.reader = i > 1 and stage.streams.input or nil
    stage.co = coroutine.create(function()
      return shell.subshell(command)
    end)
    stages[i] = stage
  end
  local _ <close> = setmetatable({}, {__close = function()
    for _, stage in ipairs(stages) do
      if coroutine.status(stage.co) == "suspended" then
        coroutine.close(stage.co)
      end
      shell.cleanup(stage.streams)
    end
  end})
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
  if not ok then
    error(err, 0)
  end
  return interrupted and 130 or stages[#stages].code or 0
end

local function drive(command)
  if command.waiting then
    local waiting = command.waiting
    command.waiting = nil
    if coroutine.isyieldable() then
      command.resume = table.pack(coroutine.yield(table.unpack(waiting, 1, waiting.n)))
    end
  end
  local before, interrupts = transfers, console.interrupts()
  step(command)
  if console.interrupts() ~= interrupts and not (package.loaded.thread and package.loaded.thread.current()) then
    error("interrupted!", 0)
  end
  if coroutine.status(command.co) == "suspended" and not command.waiting and transfers == before
      and coroutine.isyieldable() then
    coroutine.yield(BLOCKED)
  end
end

local KILLED = 143

local function finish(command)
  if not command.finished then
    command.finished = true
    if coroutine.status(command.co) == "suspended" then
      coroutine.close(command.co)
      command.code = KILLED
    end
    (command.writer or command.reader):close()
    shell.cleanup(command.streams)
    if command.files[#command.files] == command.cleanup then
      command.files[#command.files] = nil
    end
  end
end

local function closeCommand(file, close)
  local command = file.command
  close(file)
  while coroutine.status(command.co) == "suspended" do
    drive(command)
  end
  finish(command)
  return command.code == 0 or nil, "exit", command.code
end

local CommandReader = setmetatable({}, {__index = Reader})
local CommandWriter = setmetatable({}, {__index = Writer})
CommandReader.__index, CommandWriter.__index = CommandReader, CommandWriter

function CommandReader:close()
  return closeCommand(self, Reader.close)
end

function CommandWriter:close()
  return closeCommand(self, Writer.close)
end

CommandReader.__close, CommandWriter.__close = CommandReader.close, CommandWriter.close

function CommandWriter:write(...)
  local ok, err = system.rawpcall(Writer.write, self, ...)
  if ok then
    return self
  elseif err == BROKEN then
    return nil, "Broken pipe", 32
  end
  error(err, 0)
end

function pipe.popen(line, mode)
  mode = mode or "r"
  if type(line) ~= "string" then
    error("bad argument #1 to 'popen' (string expected, got " .. type(line) .. ")", 3)
  elseif mode ~= "r" and mode ~= "w" then
    error("bad argument #2 to 'popen' (invalid mode)", 3)
  end
  local parent = system.streams()
  parent.files = parent.files or {}
  local reader, writer = pipe.create()
  local command = {files = parent.files, streams = inherit(parent)}
  command.co = coroutine.create(function()
    return shell.subshell(line)
  end)
  command.cleanup = {close = function()
    finish(command)
  end}
  local file
  if mode == "r" then
    command.streams.output, command.writer = writer, writer
    file = setmetatable({pipe = reader.pipe, command = command}, CommandReader)
    reader.pipe.readerWait = function()
      drive(command)
    end
  else
    command.streams.input, command.reader = reader, reader
    file = setmetatable({pipe = writer.pipe, command = command}, CommandWriter)
    writer.pipe.writerWait = function()
      drive(command)
    end
  end
  parent.files[#parent.files + 1] = command.cleanup
  return file
end

return pipe
