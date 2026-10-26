-- A small screen editor. Tabs are replaced with two spaces when loading.

local shell = require("shell")
local system = require("system")
local term = require("term")

local USAGE = [[
Usage: edit <file>
Edits a text file.
  Ctrl+S          save
  Ctrl+W          close, twice with unsaved changes
  Ctrl+K          cut one or more lines
  Ctrl+U          paste the cut lines above the current line
  Ctrl+F          find; Enter or Escape end search mode
  Ctrl+G, Ctrl+N, F3   find the next match
  Ctrl+Del, Shift+Del  delete the line
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if not args[1] then
  return shell.usage(USAGE)
end
local path = shell.resolve(args[1])
local name = require("path").name(path) or path
local stat = fs.stat(path)
if stat and stat.isDir then
  io.stderr:write("edit: ", args[1], ": Is a directory\n")
  return 1
end

local lines = {}
if stat then
  local file, reason = io.open(path)
  local text
  if file then
    text, reason = file:read("a")
    file:close()
  end
  if not text then
    io.stderr:write("edit: ", tostring(reason), "\n")
    return 1
  end
  for line in text:gmatch("[^\n]*\n?") do
    if line ~= "" then
      lines[#lines + 1] = line:gsub("\r?\n$", ""):gsub("\t", "  ")
    end
  end
  text = nil -- the register keeps the text reachable until reused, even out of scope
  collectgarbage() -- loading leaves about as much garbage as the text takes
end
if #lines == 0 then
  lines[1] = ""
end

-- Writing the last column can make the terminal wrap, so lines and the status line end before it.
local width, height = term.width() - 1, term.height() - 1
local cx, cy, top, left = 1, 1, 1, 1
local dirty, closeArmed, running = false, false, true
local HELP = "^S Save  ^W Close  ^F Find  ^K Cut  ^U Uncut"
local message = HELP
local cutBuffer, cutting = {}, false -- cutting ends when the cursor moves to another line
local cutAbove = false -- the last cut took the last line, so the next one is the line above it
local findText = ""
local output = {}
local shownStatus = ""

local function emit(...)
  for i = 1, select("#", ...) do
    output[#output + 1] = select(i, ...)
  end
end

local function drawLine(row, from)
  from = from or left
  local line = lines[top + row - 1] or ""
  emit("\27[", row, ";", from - left + 1, "H", line:sub(from, left + width - 1), "\27[K")
end

local function drawStatus()
  local info = string.format(" %s%s  %s", name, dirty and " *" or "", message)
  if #cutBuffer > 0 then
    info = string.format(" (#%d)%s", #cutBuffer, info)
  end
  local position = string.format("%d:%d ", cy, cx)
  local status = info:sub(1, width - #position)
  status = status .. string.rep(" ", width - #status - #position) .. position
  local first = 1
  while first <= #status and status:byte(first) == shownStatus:byte(first) do
    first = first + 1
  end
  first = math.min(first, #status - #position + 1)
  emit("\27[", height + 1, ";", first, "H\27[7m", status:sub(first), "\27[0m")
  shownStatus = status
end

-- Moves top and left so the cursor is visible. Returns whether they changed. Horizontal scrolling
-- centers the cursor, so typing past the edge redraws the screen once every half width.
local function scrollToCursor()
  local oldTop, oldLeft = top, left
  top = math.min(math.max(top, cy - height + 1), cy)
  if cx < left or cx >= left + width then
    left = math.max(1, cx - width // 2)
  end
  return top ~= oldTop or left ~= oldLeft
end

local SAVE_BATCH = 64 -- lines per write, so a big file needs no copy of itself in memory

-- Writes a temporary file next to the original and renames it over the original, so a failed
-- save leaves the original as it was. The temporary file gets a name that is not taken yet.
local function save()
  local temp = path .. "~"
  for i = 1, math.huge do
    if not fs.exists(temp) then
      break
    end
    temp = path .. "~" .. i
  end
  local created, size = false, 0
  collectgarbage() -- room for the batches, also when the text nearly fills the memory
  local ok, reason = system.rawpcall(function()
    local file = assert(io.open(temp, "w"))
    created = true
    local written, writeReason = true, nil
    if #lines > 1 or lines[1] ~= "" then
      for first = 1, #lines, SAVE_BATCH do
        local chunk = table.concat(lines, "\n", first, math.min(first + SAVE_BATCH - 1, #lines)) .. "\n"
        written, writeReason = file:write(chunk)
        if not written then
          break
        end
        size = size + #chunk
      end
    end
    file:close()
    assert(written, writeReason)
    assert(fs.rename(temp, path))
  end)
  if ok then
    dirty = false
    message = "saved " .. math.max(0, size - 1) .. " bytes"
  else
    if created then
      fs.remove(temp)
    end
    if term.powerRequest(reason) then
      error(reason, 0)
    end
    message = "not saved: " .. tostring(reason):gsub("^[^:]*:%d+: ", "")
  end
end

local function redraw()
  for row = 1, height do
    drawLine(row)
  end
end

local function flush()
  drawStatus()
  emit("\27[", cy - top + 1, ";", cx - left + 1, "H")
  console.write(table.concat(output))
  output = {}
end

local function search(text, x, y)
  for i = 0, #lines do
    local row = (y - 1 + i) % #lines + 1
    local column = lines[row]:find(text, i == 0 and x or 1, true)
    if column then
      cx, cy = column, row
      return true
    end
  end
  return false
end

local function findNext()
  if not search(findText, cx + 1, cy) then
    message = "\"" .. findText .. "\" not found"
  end
end

local function find()
  local startX, startY = cx, cy
  findText = ""
  while true do
    message = "Find: " .. findText
    if scrollToCursor() then
      redraw()
    end
    flush()
    local key = term.key()
    if key == "enter" or key == "escape" then
      break
    elseif key == "backspace" then
      findText = findText:sub(1, -2)
    elseif key == "\6" or key == "\7" or key == "\14" or key == "f3" then
      startX, startY = cx + 1, cy
    elseif #key == 1 and key >= " " then
      findText = findText .. key
    end
    if findText ~= "" and not search(findText, startX, startY) then
      cx, cy = startX, startY
    end
  end
  message = HELP
end

local function deleteLine()
  if #lines > 1 then
    table.remove(lines, cy)
  else
    lines[1] = ""
  end
  cy, cx = math.min(cy, #lines), 1
end

local WHEEL_LINES = 3

local function step()
  local key, data, cut = term.key()
  local line = lines[cy]
  local lineBefore = cy
  local changed, full = false, false
  local from = cx
  message = HELP
  if key == "\23" then
    if dirty and not closeArmed then
      closeArmed = true
      message = "unsaved changes, Ctrl+W again to discard them"
    else
      running = false
    end
    return
  end
  closeArmed = false
  local insert = key == "tab" and "  " or #key == 1 and key >= " " and key
  if insert then
    lines[cy] = line:sub(1, cx - 1) .. insert .. line:sub(cx)
    cx = cx + #insert
    changed = true
  elseif key == "enter" then
    lines[cy] = line:sub(1, cx - 1)
    table.insert(lines, cy + 1, line:sub(cx))
    cy, cx, full = cy + 1, 1, true
  elseif key == "backspace" then
    if cx > 1 then
      lines[cy] = line:sub(1, cx - 2) .. line:sub(cx)
      cx, from, changed = cx - 1, cx - 1, true
    elseif cy > 1 then
      cx = #lines[cy - 1] + 1
      lines[cy - 1] = lines[cy - 1] .. line
      table.remove(lines, cy)
      cy, full = cy - 1, true
    end
  elseif key == "delete" then
    if cx <= #line then
      lines[cy] = line:sub(1, cx - 1) .. line:sub(cx + 1)
      changed = true
    elseif cy < #lines then
      lines[cy] = line .. table.remove(lines, cy + 1)
      full = true
    end
  elseif key == "\11" then
    if not cutting then
      cutBuffer, cutAbove = {}, false
    end
    table.insert(cutBuffer, cutAbove and 1 or #cutBuffer + 1, line)
    local wasLast = cy == #lines and #lines > 1
    deleteLine()
    cutting, full, lineBefore, cutAbove = true, true, cy, wasLast
  elseif key == "\21" then
    if #lines == 1 and lines[1] == "" and #cutBuffer > 0 then
      table.remove(lines) -- an empty document is one empty line, which the paste replaces
      cy = 1
    end
    for i, cut in ipairs(cutBuffer) do
      table.insert(lines, cy + i - 1, cut)
    end
    cy, cx, full = math.min(cy + #cutBuffer, #lines), 1, #cutBuffer > 0
  elseif key == "ctrl-delete" or key == "shift-delete" then
    deleteLine()
    full = true
  elseif key == "\6" then
    find()
    full = true
  elseif key == "\7" or key == "\14" or key == "f3" then
    if findText == "" then
      find()
    else
      findNext()
    end
    full = true
  elseif key == "\19" then
    save()
  elseif key == "left" then
    if cx > 1 then
      cx = cx - 1
    elseif cy > 1 then
      cy = cy - 1
      cx = #lines[cy] + 1
    end
  elseif key == "right" then
    if cx <= #line then
      cx = cx + 1
    elseif cy < #lines then
      cy, cx = cy + 1, 1
    end
  elseif key == "up" or key == "down" or key == "pageup" or key == "pagedown" then
    local delta = ({up = -1, down = 1, pageup = -height, pagedown = height})[key]
    cy = math.min(math.max(cy + delta, 1), #lines)
    cx = math.min(cx, #lines[cy] + 1)
  elseif key == "home" then
    cx = 1
  elseif key == "end" then
    cx = #line + 1
  elseif key == "ctrl-left" then
    if cx > 1 then
      cx = term.previousWord(line, cx - 1) + 1
    elseif cy > 1 then
      cy = cy - 1
      cx = #lines[cy] + 1
    end
  elseif key == "ctrl-right" then
    if cx <= #line then
      cx = term.nextWord(line, cx - 1) + 1
    elseif cy < #lines then
      cy, cx = cy + 1, 1
    end
  elseif key == "mouse" then
    if data.button == 64 or data.button == 65 then
      cy = math.min(math.max(cy + (data.button == 64 and -WHEEL_LINES or WHEEL_LINES), 1), #lines)
      cx = math.min(cx, #lines[cy] + 1)
    elseif data.button == 0 and data.pressed and data.y <= height then
      cy = math.min(top + data.y - 1, #lines)
      cx = math.min(left + data.x - 1, #lines[cy] + 1)
    end
  elseif key == "paste" then
    local pasted = {}
    for text in (data:gsub("\r\n?", "\n"):gsub("\t", "  ") .. "\n"):gmatch("(.-)\n") do
      pasted[#pasted + 1] = (text:gsub("%c", " "))
    end
    local after = line:sub(cx)
    lines[cy] = line:sub(1, cx - 1) .. pasted[1]
    for i = 2, #pasted do
      table.insert(lines, cy + i - 1, pasted[i])
    end
    cy = cy + #pasted - 1
    cx = #lines[cy] + 1
    lines[cy] = lines[cy] .. after
    changed, full = true, true
    if cut then
      message = "paste cut to 16 KiB"
    end
  end
  if cy ~= lineBefore then
    cutting = false
  end
  dirty = dirty or changed or (full and key ~= "\6" and key ~= "\7" and key ~= "\14" and key ~= "f3")
  if scrollToCursor() or full then
    for row = 1, height do
      drawLine(row)
    end
  elseif changed then
    drawLine(cy - top + 1, from)
  end
end

local MODES = {"1049", "1000", "1006", "1007", "2004"}
local function setModes(suffix, first, last, step)
  for i = first, last, step do
    console.write("\27[?", MODES[i], suffix)
  end
end
setModes("h", 1, #MODES, 1)
local ok, err = system.rawpcall(function()
  redraw()
  while running do
    flush()
    local stepOk, stepErr = system.rawpcall(step)
    if not stepOk and term.powerRequest(stepErr) then
      error(stepErr, 0) -- unsaved changes are lost, as with any power off
    end
    if not stepOk then
      -- Errors, e.g. running out of memory or Ctrl+C, end the key, not the editor and its text.
      message = term.systemError(stepErr) == "interrupt" and "Ctrl+W to close"
        or "error: " .. tostring(stepErr):gsub("^[^:]*:%d+: ", "")
    end
  end
end)
setModes("l", #MODES, 1, -1)
if not ok then
  error(err, 0)
end
