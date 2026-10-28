local USAGE = [=[
Usage: lua [script [arg...]]
Runs a Lua script, or the interactive Lua prompt.
]=]

local args = table.pack(...)
if args[1] == "-h" or args[1] == "--help" then
  io.write(USAGE)
  return 0
end
if args.n == 0 then
  require("repl").run(setmetatable({}, {__index = _G}))
  return
end
local env = setmetatable({arg = {[0] = args[1], table.unpack(args, 2, args.n)}}, {__index = _G})
local program, reason = loadfile(args[1], "bt", env)
if not program then
  io.stderr:write("lua: ", reason, "\n")
  return 1
end
return program(table.unpack(args, 2, args.n))
