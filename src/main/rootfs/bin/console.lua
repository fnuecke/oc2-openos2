local serial = require("serial")
local shell = require("shell")
local term = require("term")

local USAGE = [[
Usage: console [device]
Prints the serial port of the console, or moves the console to another one.
For example, console /dev/ttyS1 moves it to the first serial card.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args == 0 then
  print(term.console())
  return 0
elseif #args > 1 then
  return shell.usage(USAGE)
end
local device = shell.resolve(args[1])
if not pcall(serial.getBaud, device) then
  io.stderr:write("console: ", device, ": Not a serial port\n")
  return 1
end
print("Console moved to " .. device)
local ok, reason = term.setConsole(device)
if not ok then
  io.stderr:write("console: ", device, ": ", reason, "\n")
  return 1
end
