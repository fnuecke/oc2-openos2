local shell = require("shell")

local USAGE = [[
Usage: yes [string...]
Prints "y", or the strings, on a line over and over, until the reader of a pipe
goes away or Ctrl+C.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
local line = (#args > 0 and table.concat(args, " ") or "y") .. "\n"
local block = line:rep(math.max(1, 512 // #line))
local output = io.output()
while true do
  output:write(block)
end
