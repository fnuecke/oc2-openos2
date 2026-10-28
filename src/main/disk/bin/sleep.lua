local shell = require("shell")

local USAGE = [[
Usage: sleep <seconds>
Waits for a number of seconds.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
local seconds = tonumber(args[1])
if not seconds or seconds < 0 then
  return shell.usage(USAGE)
end
computer.sleep(seconds)
