local shell = require("shell")

local USAGE = [[
Usage: time <program> [arg...]
Runs the program and prints how long it took. There is one program running at a
time, so this is also the processor time it used.
]]

local name = ...
if name == "-h" or name == "--help" then
  io.write(USAGE)
  return 0
end
if not name then
  return shell.usage(USAGE)
end
local path = shell.which(name)
if not path then
  io.stderr:write("time: cannot run ", name, ": No such file or directory\n")
  return 127
end
local start = computer.uptime()
local code = shell.run(path, select(2, ...))
io.stderr:write(string.format("real %.3f s\n", computer.uptime() - start))
return code
