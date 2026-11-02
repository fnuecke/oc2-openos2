local shell = require("shell")

local USAGE = [[
Usage: clear
Clears the terminal.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
io.write("\27[2J\27[H")
