local event = require("event")
local system = require("system")

local term = {}

local MAX_HISTORY = 50
local ESCAPE_TIMEOUT = 0.05

local pending, position = "", 1
local pendingInterrupts = 0 -- console.interrupts() when pending was pulled
local lastWasCR = false
local column = 0

local function nextByte(timeout)
  if console.interrupts() ~= pendingInterrupts then
    pending, position = "", 1 -- typed ahead of a Ctrl+C
  end
  while position > #pending do
    local _, data = event.pull(timeout, "console")
    if data then
      pending, position, pendingInterrupts = data, 1, console.interrupts()
    elseif timeout then
      return nil
    end
  end
  local c = pending:sub(position, position)
  position = position + 1
  return c
end

local CSI_KEYS = {
  A = "up", B = "down", C = "right", D = "left", H = "home", F = "end",
  P = "f1", Q = "f2", R = "f3", S = "f4"}
local TILDE_KEYS = {"home", "insert", "delete", "end", "pageup", "pagedown", "home", "end"}

local function escapeKey()
  local kind = nextByte(ESCAPE_TIMEOUT)
  if not kind then
    return "escape"
  elseif kind == "\8" or kind == "\127" then
    return "alt-backspace"
  elseif kind ~= "[" and kind ~= "O" then
    return "alt-" .. kind
  end
  local parameters, final = "", nextByte(ESCAPE_TIMEOUT)
  while final and final:find("^[%d;]") do
    parameters = parameters .. final
    final = nextByte(ESCAPE_TIMEOUT)
  end
  local code, modifier = parameters:match("^(%d*);?(%d*)$")
  code, modifier = tonumber(code), (tonumber(modifier) or 1) - 1
  local name
  if final == "~" then
    name = TILDE_KEYS[code]
    if not name and code and code >= 11 and code <= 24 and code ~= 16 and code ~= 22 then
      name = "f" .. code - 10 - (code > 16 and 1 or 0) - (code > 22 and 1 or 0)
    end
  else
    name = CSI_KEYS[final]
  end
  if not name then
    return "unknown"
  end
  return (modifier & 1 ~= 0 and "shift-" or "") .. (modifier & 2 ~= 0 and "alt-" or "")
    .. (modifier & 4 ~= 0 and "ctrl-" or "") .. name
end

-- Returns the next key: a single character for printable input and control characters, or a name
-- such as "enter", "backspace", "tab", "left", "home", "delete", "pageup", "f3" or "escape", with
-- "shift-", "alt-" or "ctrl-" in front for modified keys, e.g. "ctrl-delete". Returns nil
-- after timeout seconds, if given.
function term.key(timeout)
  while true do
    local c = nextByte(timeout)
    if not c then
      return nil
    end
    local wasCR = lastWasCR
    lastWasCR = c == "\r"
    if c == "\r" then
      return "enter"
    elseif c == "\n" then
      if not wasCR then
        return "enter"
      end
    elseif c == "\8" or c == "\127" then
      return "backspace"
    elseif c == "\t" then
      return "tab"
    elseif c == "\27" then
      return escapeKey()
    else
      return c
    end
  end
end

-- The errors the kernel raises in running code, by kind. Programs cannot catch them, see /init.lua.
local SYSTEM_ERRORS = {
  ["interrupted!"] = "interrupt", ["too long without yielding"] = "watchdog",
  ["shutdown requested"] = "shutdown", ["reboot requested"] = "reboot"}

-- Returns "interrupt" for Ctrl+C, "watchdog", "shutdown" or "reboot" for the host's power button, or
-- nil if err is no system error.
function term.systemError(err)
  return SYSTEM_ERRORS[err]
end

function term.isSystemError(err)
  return SYSTEM_ERRORS[err] ~= nil
end

-- Returns "shutdown" or "reboot" for a power request from the host, nil for other errors.
function term.powerRequest(err)
  local kind = SYSTEM_ERRORS[err]
  return (kind == "shutdown" or kind == "reboot") and kind or nil
end

function term.width()
  return tonumber(os.getenv("COLUMNS")) or 80
end

function term.height()
  return tonumber(os.getenv("LINES")) or 24
end

function term.write(...)
  for i = 1, select("#", ...) do
    local s = tostring((select(i, ...)))
    console.write(s)
    local tail = s:match("[^\r\n]*$")
    if #tail < #s then
      column = 0
    end
    local afterHome = tail:match("^.*\27%[[%d;]*H(.*)$")
    if afterHome then
      column, tail = 0, afterHome
    end
    column = column + #tail:gsub("\27%[[%d;?]*%a", "")
  end
end

function term.column()
  return column
end

-- Reads a line with cursor movement and history.
-- Returns nil if Ctrl+D is pressed on an empty line.
function term.read(prompt, history)
  if prompt then
    term.write(prompt)
  end
  if column > term.width() - 10 then
    term.write("\n")
  end
  local width = term.width() - column - 1
  local line, cursor, scroll, shown = "", 0, 0, 0
  local index = history and #history + 1

  local function render()
    if cursor < scroll or cursor > scroll + width then
      scroll = math.max(0, cursor - width // 2)
    end
    local visible = line:sub(scroll + 1, scroll + width)
    local back = #visible - (cursor - scroll)
    console.write(shown > 0 and "\27[" .. shown .. "D" or "", visible, "\27[K",
      back > 0 and "\27[" .. back .. "D" or "")
    shown = cursor - scroll
  end

  local function set(text)
    line, cursor = text, #text
  end

  while true do
    local ok, key = system.rawpcall(term.key)
    if not ok then
      term.write("\n") -- Ctrl+C: continue on the next line
      error(key, 0)
    end
    if key == "enter" then
      term.write("\n")
      if history and line:find("%S") and history[#history] ~= line then
        table.insert(history, line)
        if #history > MAX_HISTORY then
          table.remove(history, 1)
        end
      end
      return line
    elseif key == "\4" then
      if line == "" then
        term.write("\n")
        return nil
      end
    elseif key == "backspace" then
      if cursor > 0 then
        line = line:sub(1, cursor - 1) .. line:sub(cursor + 1)
        cursor = cursor - 1
      end
    elseif key == "delete" then
      line = line:sub(1, cursor) .. line:sub(cursor + 2)
    elseif key == "left" then
      cursor = math.max(0, cursor - 1)
    elseif key == "right" then
      cursor = math.min(#line, cursor + 1)
    elseif key == "home" then
      cursor = 0
    elseif key == "end" then
      cursor = #line
    elseif key == "up" and history and index > 1 then
      index = index - 1
      set(history[index])
    elseif key == "down" and history and index <= #history then
      index = index + 1
      set(history[index] or "")
    elseif #key == 1 and key >= " " then
      line = line:sub(1, cursor) .. key .. line:sub(cursor + 1)
      cursor = cursor + 1
    end
    if #key == 1 and key >= " " and cursor == #line and cursor - scroll == shown + 1
      and shown < width then
      console.write(line:sub(-1))
      shown = shown + 1
    else
      render()
    end
  end
end

local stream = {tty = true}
term.stream = stream

local function readFormat(format)
  if type(format) == "number" then
    local bytes = {}
    for i = 1, format do
      bytes[i] = nextByte()
    end
    return table.concat(bytes)
  end
  format = (format or "l"):gsub("^%*", "")
  if format:sub(1, 1) == "a" then
    local lines = {}
    for line in stream:lines("L") do
      lines[#lines + 1] = line
    end
    return table.concat(lines)
  end
  local line = term.read()
  if line and format:sub(1, 1) == "L" then
    line = line .. "\n"
  end
  return line
end

function stream:read(...)
  local n = select("#", ...)
  if n == 0 then
    return readFormat("l")
  end
  local results = {}
  for i = 1, n do
    results[i] = readFormat((select(i, ...)))
    if results[i] == nil then
      return table.unpack(results, 1, i)
    end
  end
  return table.unpack(results, 1, n)
end

function stream:lines(format)
  return function()
    return self:read(format)
  end
end

function stream:write(...)
  term.write(...)
  return self
end

function stream:flush()
  return self
end

function stream:setvbuf()
  return true
end

function stream:seek()
  return nil, "Illegal seek"
end

function stream:close()
  return true
end

return term
