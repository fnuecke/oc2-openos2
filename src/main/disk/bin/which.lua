local shell = require("shell")

local USAGE = [[
Usage: which <program>...
Prints the file each program name runs, looked up in $PATH.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args == 0 then
  return shell.usage(USAGE)
end
local code = 0
for _, name in ipairs(args) do
  local path = shell.which(name)
  if path then
    print(path)
  else
    io.stderr:write("which: no ", name, " in (", os.getenv("PATH") or "", ")\n")
    code = 1
  end
end
return code
