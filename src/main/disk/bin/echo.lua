local shell = require("shell")

local USAGE = [[
Usage: echo [-n] [word...]
Prints the words.
  -n  leave out the newline at the end
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "n")
if invalid then
  return invalid
end
io.write(table.concat(args, " "))
if not options.n then
  io.write("\n")
end
