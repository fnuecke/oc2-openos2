local shell = require("shell")
local term = require("term")

local USAGE = [[
Usage: motd
Prints a random tip from /usr/share/tips.txt.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end

local tips = {}
local file = io.open("/usr/share/tips.txt")
if file then
  for line in file:lines() do
    if line:find("%S") then
      tips[#tips + 1] = line
    end
  end
  file:close()
end

local width = term.width() - 5
local lines = {}
local tip = tips[math.random(math.max(#tips, 1))] or ""
while #tip > 0 do
  local line = #tip <= width and tip or tip:sub(1, width + 1):match("^(.*%S)%s") or tip:sub(1, width)
  lines[#lines + 1] = line
  tip = tip:sub(#line + 1):match("^%s*(.*)$")
end
if #lines == 0 then
  return
end
local inner = 0
for _, line in ipairs(lines) do
  inner = math.max(inner, #line)
end

io.write("\27(0l", ("q"):rep(inner + 2), "k\27(B\n")
for _, line in ipairs(lines) do
  io.write("\27(0x\27(B ", line, (" "):rep(inner - #line), " \27(0x\27(B\n")
end
io.write("\27(0m", ("q"):rep(inner + 2), "j\27(B\n")
