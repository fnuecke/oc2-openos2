local shell = require("shell")
local transfer = require("transfer")

local USAGE = [[
Usage: rm [-r] [-f] <path>...
Removes files.
  -r  remove directories and their contents
  -f  ignore missing paths
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "rf")
if invalid then
  return invalid
end
if #args == 0 then
  return shell.usage(USAGE)
end
local code = 0
for _, arg in ipairs(args) do
  local path = shell.resolve(arg)
  local stat = fs.lstat(path)
  local ok, reason = true, nil
  if not stat then
    ok, reason = options.f, "cannot remove '" .. arg .. "': No such file or directory"
  elseif stat.kind == "directory" and not options.r then
    ok, reason = false, "cannot remove '" .. arg .. "': Is a directory"
  else
    ok, reason = transfer.remove(path, options.r)
  end
  if not ok then
    io.stderr:write("rm: ", reason, "\n")
    code = 1
  end
end
return code
