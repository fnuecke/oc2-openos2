local shell = require("shell")

local USAGE = [[
Usage: shutdown
Turns the computer off.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
computer.shutdown()
