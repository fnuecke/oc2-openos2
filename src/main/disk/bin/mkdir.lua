local shell = require("shell")
local USAGE = [[
Usage: mkdir [-p] <directory>...
Creates directories.
  -p  create missing parents, allow existing directories
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "p")
if invalid then
  return invalid
end
if #args == 0 then
  return shell.usage(USAGE)
end

local function mkdir(path)
  local stat = fs.stat(path)
  if stat and stat.isDir and options.p then
    return true
  end
  if options.p then
    local parent = require("path").path(path)
    if parent ~= "/" then
      local ok, reason = mkdir(parent)
      if not ok then
        return nil, reason
      end
    end
  end
  return fs.mkdir(path)
end

local code = 0
for _, arg in ipairs(args) do
  local ok, reason = mkdir(shell.resolve(arg))
  if not ok then
    io.stderr:write("mkdir: cannot create directory '", arg, "': ", reason, "\n")
    code = 1
  end
end
return code
