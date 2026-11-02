local path = require("path")
local shell = require("shell")

local USAGE = [[
Usage: du [-h] [-s] [-x] [path...]
Prints the size of each file and directory, recursively, in bytes.
  -h  sizes like 1.5K, 234M
  -s  only a total for each argument
  -x  stay on the file system of each path
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "hsx")
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "."
end

local function format(size)
  if not options.h then
    return tostring(size)
  end
  local units, unit = {"", "K", "M", "G"}, 1
  while size >= 1024 and unit < #units do
    size, unit = size / 1024, unit + 1
  end
  return (unit == 1 and tostring(size) or string.format("%.1f", size)) .. units[unit]
end

local function show(size, name)
  print(string.format("%-12s%s", format(size), name))
end

local mounts = options.x and require("medium").mountPoints()
local top

local function visit(name, kind)
  if kind ~= "directory" then
    local stat = fs.lstat(shell.resolve(name))
    return stat and stat.size or 0
  elseif mounts and mounts[shell.resolve(name)] and name ~= top then
    return 0
  end
  local total = 0
  local entries, kinds = fs.list(shell.resolve(name))
  for _, entry in ipairs(entries or {}) do
    total = total + visit(path.concat(name, entry), kinds[entry])
  end
  if not options.s then
    show(total, name)
  end
  return total
end

local code = 0
for _, arg in ipairs(args) do
  local stat, reason = fs.lstat(shell.resolve(arg))
  if not stat then
    io.stderr:write("du: cannot access '", arg, "': ", reason, "\n")
    code = 1
  elseif stat.kind ~= "directory" then
    show(stat.size, arg)
  else
    top = arg
    local total = visit(arg, stat.kind)
    if options.s then
      show(total, arg)
    end
  end
end
return code
