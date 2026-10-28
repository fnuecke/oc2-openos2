local shell = require("shell")
local system = require("system")
local term = require("term")

local USAGE = [[
Usage: less [file]
Shows a file, or the input, one screen at a time. q quits.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end

local file, reason = io.stdin, nil
if args[1] then
  file, reason = io.open(args[1])
end
if not file then
  io.stderr:write("less: ", reason, "\n")
  return 1
end
local width, height = term.width() - 1, term.height() - 1 -- the last column can make it wrap

local rows = {}
while true do
  local line, readReason = file:read("l")
  if not line then
    if readReason then
      io.stderr:write("less: ", readReason, "\n")
      return 1
    end
    break
  end
  line = line:gsub("\t", "  ")
  repeat
    rows[#rows + 1] = line:sub(1, width)
    line = line:sub(width + 1)
  until line == ""
end
if args[1] then
  file:close()
end

if not term.isTerminal(io.output()) or #rows <= height then
  for _, row in ipairs(rows) do
    print(row)
  end
  return 0
end

local top = 1
local last = math.max(1, #rows - height + 1)
local function draw()
  local output = {"\27[H"}
  for i = 0, height - 1 do
    output[#output + 1] = (rows[top + i] or "") .. "\27[K\r\n"
  end
  output[#output + 1] = "\27[7m" .. (top >= last and "(END)" or string.format("%d%%", (top + height - 1) * 100 // #rows)) .. "\27[0m\27[K"
  io.write(table.concat(output))
end

io.write("\27[?1049h")
local ok, err = system.rawpcall(function()
  while true do
    draw()
    local key = term.key()
    if key == "q" or key == "Q" then
      return
    elseif key == " " or key == "pagedown" or key == "f" then
      top = top + height
    elseif key == "b" or key == "pageup" then
      top = top - height
    elseif key == "enter" or key == "down" or key == "j" then
      top = top + 1
    elseif key == "up" or key == "k" then
      top = top - 1
    elseif key == "g" or key == "home" then
      top = 1
    elseif key == "G" or key == "end" then
      top = last
    end
    top = math.max(1, math.min(top, last))
  end
end)
io.write("\27[?1049l")
if not ok and term.systemError(err) ~= "interrupt" then
  error(err, 0)
end
