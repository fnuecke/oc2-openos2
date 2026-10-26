local shell = require("shell")

local USAGE = [[
Usage: tree [-a] [-x] [path...]
Prints the directories below each path, or the current one, as a tree.
  -a  include names that start with "."
  -x  stay on the file system of each path
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "ax")
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "."
end

local dirs, files = 0, 0
local mounts = options.x and require("medium").mountPoints()

local function visit(dir, indent)
  local names = {}
  for _, name in ipairs(fs.list(shell.resolve(dir)) or {}) do
    if options.a or name:sub(1, 1) ~= "." then
      names[#names + 1] = name
    end
  end
  table.sort(names)
  for i, name in ipairs(names) do
    local last = i == #names
    local isDir = name:sub(-1) == "/"
    print(indent .. (last and "`-- " or "|-- ") .. name:gsub("/$", ""))
    if isDir then
      dirs = dirs + 1
      if not (mounts and mounts[shell.resolve(dir .. "/" .. name)]) then
        visit(dir .. "/" .. name, indent .. (last and "    " or "|   "))
      end
    else
      files = files + 1
    end
  end
end

local code = 0
for _, arg in ipairs(args) do
  local stat, reason = fs.stat(shell.resolve(arg))
  if not stat or not stat.isDir then
    io.stderr:write("tree: ", arg, ": ", reason or "Not a directory", "\n")
    code = 1
  else
    print(arg)
    visit(arg, "")
  end
end
print(string.format("\n%d director%s, %d file%s", dirs, dirs == 1 and "y" or "ies", files, files == 1 and "" or "s"))
return code
