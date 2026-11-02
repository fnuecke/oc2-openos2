local shell = require("shell")

local USAGE = [[
Usage: reboot
Restarts the computer.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
computer.reboot()
