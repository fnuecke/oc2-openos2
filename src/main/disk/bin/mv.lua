local shell = require("shell")
local transfer = require("transfer")

local USAGE = [[
Usage: mv <from>... <to>
Moves or renames files and directories, also between mounts.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args < 2 then
  return shell.usage(USAGE)
end
local to = shell.resolve(table.remove(args))
local stat = fs.stat(to)
local intoDir = stat ~= nil and stat.isDir
if #args > 1 and not intoDir then
  io.stderr:write("mv: target '", to, "' is not a directory\n")
  return 1
end
local code = 0
for _, arg in ipairs(args) do
  local from = shell.resolve(arg)
  local target = intoDir and shell.resolve(to .. "/" .. require("path").name(from)) or to
  local ok, reason = transfer.move(from, target)
  if not ok then
    io.stderr:write("mv: ", reason, "\n")
    code = 1
  end
end
return code
