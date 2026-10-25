local shell = require("shell")

local USAGE = [[
Usage: rmdir <directory>...
Removes empty directories.
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
  local stat = fs.stat(arg)
  local reason
  if not stat then
    reason = "No such file or directory"
  elseif not stat.isDir then
    reason = "Not a directory"
  elseif #fs.list(arg) > 0 then
    reason = "Directory not empty"
  else
    local _, removeReason = fs.remove(arg)
    reason = removeReason
  end
  if reason then
    io.stderr:write("rmdir: failed to remove '", arg, "': ", reason, "\n")
    code = 1
  end
end
return code
