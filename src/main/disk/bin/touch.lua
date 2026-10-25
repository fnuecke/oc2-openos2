local shell = require("shell")

local USAGE = [[
Usage: touch <file>...
Creates the files that do not exist yet, empty. Existing files are left as they
are.
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
for _, arg in ipairs(args) do
  if not fs.exists(arg) then
    local file, reason = io.open(arg, "a")
    if file then
      file:close()
    else
      io.stderr:write("touch: cannot touch '", arg, "': ", (reason:gsub("^.-: ", "")), "\n")
      code = 1
    end
  end
end
return code
