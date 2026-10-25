local shell = require("shell")
local term = require("term")

local USAGE = [[
Usage: ls [-a] [-l] [path...]
Lists directories, or the current one.
  -a  include names that start with "."
  -l  print size and modification time
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "al")
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "."
end

local function list(arg)
  local path = shell.resolve(arg)
  local stat, reason = fs.stat(path)
  if not stat then
    io.stderr:write("ls: cannot access '", arg, "': ", reason, "\n")
    return false
  end
  local names = {}
  if not stat.isDir then
    names[1] = arg
  else
    for _, name in ipairs(fs.list(path)) do
      if options.a or name:sub(1, 1) ~= "." then
        names[#names + 1] = name
      end
    end
    table.sort(names)
  end
  if options.l then
    for _, name in ipairs(names) do
      local entry = stat.isDir and fs.stat(path .. "/" .. name) or stat
      print(string.format("%8d %s %s", entry.size, os.date("%Y-%m-%d %H:%M", entry.mtime), name))
    end
  elseif not io.output().tty then
    for _, name in ipairs(names) do
      print(name)
    end
  else
    local width = 0
    for _, name in ipairs(names) do
      width = math.max(width, #name + 2)
    end
    local columns = math.max(1, (term.width() - 1) // math.max(width, 1))
    local rows = (#names + columns - 1) // columns
    for row = 1, rows do
      local line = {}
      for column = 1, columns do
        local name = names[(column - 1) * rows + row]
        if name then
          line[#line + 1] = name .. string.rep(" ", width - #name)
        end
      end
      print((table.concat(line):gsub("%s+$", "")))
    end
  end
  return true
end

local code = 0
for i, arg in ipairs(args) do
  if #args > 1 then
    print((i > 1 and "\n" or "") .. arg .. ":")
  end
  if not list(arg) then
    code = 1
  end
end
return code
