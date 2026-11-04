local fcntl = require("posix.fcntl")
local poll = require("posix.poll")
local system = require("system")
local unistd = require("posix.unistd")

local term = {}

local MAX_HISTORY = 50
local ESCAPE_TIMEOUT = 0.05
local READ_SIZE = 4096

local consolePath = "/dev/ttyS0"
local tty = assert(fcntl.open(consolePath, fcntl.O_RDONLY))

local pending, position = "", 1
local pendingInterrupts = 0
local lastWasCR = false
local column = 0

local function refill(timeout)
  if console.interrupts() ~= pendingInterrupts then
    pending, position = "", 1
  end
  while position > #pending do
    if poll.rpoll(tty, timeout and math.floor(timeout * 1000) or -1) == 0 and timeout then
      return false
    end
    local data = unistd.read(tty, READ_SIZE)
    if data and data ~= "" then
      pending, position, pendingInterrupts = data, 1, console.interrupts()
    end
  end
  return true
end

local function nextByte(timeout)
  if not refill(timeout) then
    return nil
  end
  local c = pending:sub(position, position)
  position = position + 1
  return c
end

local CSI_KEYS = {
  A = "up", B = "down", C = "right", D = "left", H = "home", F = "end",
  P = "f1", Q = "f2", R = "f3", S = "f4"}
local TILDE_KEYS = {"home", "insert", "delete", "end", "pageup", "pagedown", "home", "end"}

local PASTE_LIMIT = 16384
local PASTE_END = "\27[201~"
local pasting = false

local function readPaste(timeout)
  local parts, size, carry = {}, 0, ""
  pasting = true
  while refill(timeout) do
    local chunk = carry .. pending:sub(position)
    local stop = chunk:find(PASTE_END, 1, true)
    local text = stop and chunk:sub(1, stop - 1) or chunk:sub(1, -#PASTE_END)
    carry = stop and "" or chunk:sub(-#PASTE_END + 1)
    if size < PASTE_LIMIT then
      parts[#parts + 1] = text:sub(1, PASTE_LIMIT - size)
    end
    size = size + #text
    if stop then
      pending, position = chunk:sub(stop + #PASTE_END), 1
      break
    end
    position = #pending + 1
  end
  pasting = false
  return table.concat(parts), size > PASTE_LIMIT
end

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
  local mouse = final == "<"
  if mouse then
    final = nextByte(ESCAPE_TIMEOUT)
  end
  while final and final:find("^[%d;]") do
    parameters = parameters .. final
    final = nextByte(ESCAPE_TIMEOUT)
  end
  if mouse then
    local button, x, y = parameters:match("^(%d+);(%d+);(%d+)$")
    if not button or (final ~= "M" and final ~= "m") then
      return "unknown"
    end
    return "mouse", {button = tonumber(button), x = tonumber(x), y = tonumber(y), pressed = final == "M"}
  end
  if final == "~" and parameters == "200" then
    local text, cut = readPaste()
    return "paste", text, cut
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

function term.key(timeout)
  if pasting then
    readPaste(ESCAPE_TIMEOUT * 10)
  end
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

local SYSTEM_ERRORS = {
  ["interrupted!"] = "interrupt", ["shutdown requested"] = "shutdown", ["reboot requested"] = "reboot",
  ["killed"] = "kill"}

function term.systemError(err)
  return SYSTEM_ERRORS[err]
end

function term.isSystemError(err)
  return SYSTEM_ERRORS[err] ~= nil
end

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
    local tail = s:match("[^\r\n]*$")
    local after = #tail < #s and 0 or column
    local afterHome = tail:match("^.*\27%[[%d;]*H(.*)$")
    if afterHome then
      after, tail = 0, afterHome
    end
    after = after + #tail:gsub("\27%[[%d;?]*%a", "")
    column = math.max(after, 1)
    console.write(s)
    column = after
  end
end

function term.isTerminal(stream)
  return stream == term.stream
end

function term.column()
  return column
end

function term.console()
  return consolePath
end

local pastedLines = {}
local lineRest = "" -- the rest of a line read in part, see readFormat

function term.setConsole(path)
  local fd, reason = fcntl.open(path, fcntl.O_RDONLY)
  if not fd then
    return nil, reason
  end
  if not pcall(require("serial").setConsole, path) then
    unistd.close(fd)
    return nil, "Not a serial port"
  end
  unistd.close(tty)
  tty, consolePath, pending, position = fd, path, "", 1
  pastedLines, lineRest, lastWasCR = {}, "", false
  return true
end

local function isWordByte(line, i)
  return line:sub(i, i):find("^[%w_]") ~= nil
end

function term.previousWord(line, cursor)
  while cursor > 0 and not isWordByte(line, cursor) do
    cursor = cursor - 1
  end
  while cursor > 0 and isWordByte(line, cursor) do
    cursor = cursor - 1
  end
  return cursor
end

function term.nextWord(line, cursor)
  while cursor < #line and not isWordByte(line, cursor + 1) do
    cursor = cursor + 1
  end
  while cursor < #line and isWordByte(line, cursor + 1) do
    cursor = cursor + 1
  end
  return cursor
end

local function addHistory(history, line)
  if history and line:find("%S") and history[#history] ~= line then
    table.insert(history, line)
    if #history > MAX_HISTORY then
      table.remove(history, 1)
    end
  end
end

function term.read(prompt, history, complete)
  if prompt then
    term.write(prompt)
  end
  if lineRest ~= "" and console.interrupts() == pendingInterrupts then
    local line = lineRest:sub(1, -2)
    lineRest = ""
    term.write(line, "\n")
    addHistory(history, line)
    return line
  end
  lineRest = ""
  if #pastedLines > 0 then
    local line = table.remove(pastedLines, 1)
    term.write(line, "\n")
    addHistory(history, line)
    return line
  end
  console.write("\27[?2004h")
  if column > term.width() - 10 then
    term.write("\n")
  end
  local width = term.width() - column - 1
  local line, cursor, scroll, shown = "", 0, 0, 0
  local lastKey
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
    local ok, key, pasted, cut = system.rawpcall(term.key)
    if not ok then
      console.write("\27[?2004l")
      term.write("\n")
      error(key, 0)
    end
    if key == "enter" then
      console.write("\27[?2004l")
      term.write("\n")
      addHistory(history, line)
      return line
    elseif key == "\4" then
      if line == "" then
        console.write("\27[?2004l")
        term.write("\n")
        return nil
      end
    elseif key == "paste" then
      if cut then
        term.write("\n(paste cut to 16 KiB)\n", prompt or "")
        shown, scroll = 0, 0
      end
      local lines = {}
      for text in (pasted:gsub("\r\n?", "\n"):gsub("\n+$", "") .. "\n"):gmatch("(.-)\n") do
        lines[#lines + 1] = (text:gsub("%c", " "))
      end
      local after = line:sub(cursor + 1)
      line = line:sub(1, cursor) .. lines[1]
      cursor = #line
      if #lines == 1 then
        line = line .. after
      else
        for i = 2, #lines do
          pastedLines[#pastedLines + 1] = lines[i]
        end
        pastedLines[#pastedLines] = pastedLines[#pastedLines] .. after
      end
    elseif key == "tab" and complete then
      local newLine, newCursor = complete(line, cursor, lastKey == "tab")
      if newLine then
        line, cursor = newLine, newCursor
      else
        term.write(prompt or "")
        shown, scroll = 0, 0
      end
    elseif key == "ctrl-left" then
      cursor = term.previousWord(line, cursor)
    elseif key == "ctrl-right" then
      cursor = term.nextWord(line, cursor)
    elseif key == "alt-backspace" then
      local start = term.previousWord(line, cursor)
      line, cursor = line:sub(1, start) .. line:sub(cursor + 1), start
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
    lastKey = key
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
    if format <= 0 then
      return ""
    end
    if lineRest == "" then
      local line = term.read()
      if line == nil then
        return nil
      end
      lineRest = line .. "\n"
    end
    local data = lineRest:sub(1, format)
    lineRest = lineRest:sub(format + 1)
    return data
  end
  format = (format or "l"):gsub("^%*", "")
  if format:sub(1, 1) == "a" then
    local lines = {}
    for line in stream:lines("L") do
      lines[#lines + 1] = line
    end
    return table.concat(lines)
  end
  local line
  if lineRest ~= "" then
    line, lineRest = lineRest:sub(1, -2), ""
  else
    line = term.read()
  end
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
setmetatable(stream, {__close = stream.close})

return term
