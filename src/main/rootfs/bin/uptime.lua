local shell = require("shell")

local USAGE = [[
Usage: uptime
Prints how long the computer has been running.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
local seconds = math.floor(computer.uptime())
print(string.format("up %d:%02d:%02d", seconds // 3600, seconds // 60 % 60, seconds % 60))
