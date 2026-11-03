local shell = require("shell")
local term = require("term")

local USAGE = [[
Usage: watch [-n <seconds>] <command> [arg...]
Clears the screen and runs the command, again and again, until Ctrl+C.
  -n seconds  time between runs, default 2
]]

local args = {...}
if args[1] == "-h" or args[1] == "--help" then
  io.write(USAGE)
  return 0
end
local seconds = 2
if args[1] == "-n" then
  seconds = tonumber(args[2])
  table.remove(args, 1)
  table.remove(args, 1)
elseif args[1] and args[1]:match("^%-n.") then
  seconds = tonumber(args[1]:sub(3))
  table.remove(args, 1)
end
if not seconds or seconds <= 0 or #args == 0 then
  return shell.usage(USAGE)
end

local command = table.concat(args, " ")
local presses = console.interrupts()
while true do
  local header = string.format("Every %gs: %s", seconds, command)
  local date = os.date("%F %T")
  io.write("\27[2J\27[H", header, string.rep(" ", term.width() - 1 - #header - #date), date, "\n\n")
  os.execute(command)
  if console.interrupts() ~= presses then
    return 130
  end
  io.flush()
  computer.sleep(seconds)
end
