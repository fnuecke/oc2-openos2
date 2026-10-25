local shell = require("shell")

local USAGE = [[
Usage: sleep <seconds>
Waits for the number of seconds, which may have a fraction. Ctrl+C ends the
wait.
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
