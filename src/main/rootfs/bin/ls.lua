local shell = require("shell")
local term = require("term")

local USAGE = [[
Usage: ls [-a] [-l] [path...]
Lists directories, or the current one.
  -a  include names that start with "."
  -l  print size, modification time and link targets
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "al")
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "."
end

local color = term.isTerminal(io.output()) and not os.getenv("NO_COLOR")

local function colored(name, kind)
  if not color then
    return name
  elseif name:sub(-1) == "/" then
    return "\27[01;34m" .. name .. "\27[0m"
  elseif kind == "link" then
    return "\27[01;36m" .. name .. "\27[0m"
  elseif name:sub(-4) == ".lua" then
    return "\27[01;32m" .. name .. "\27[0m"
  end
  return name
end

local function show(names, kinds, pathOf)
  if options.l then
    for _, name in ipairs(names) do
      local entry = fs.lstat(pathOf(name))
      local shown = colored(name, entry.kind)
      if entry.kind == "link" then
        shown = shown .. " -> " .. (fs.readlink(pathOf(name)) or "?")
      end
      print(string.format("%8d %s %s", entry.size, os.date("%Y-%m-%d %H:%M", entry.mtime), shown))
    end
  elseif not term.isTerminal(io.output()) then
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
          line[#line + 1] = colored(name, kinds[name]) .. string.rep(" ", width - #name)
        end
      end
      print((table.concat(line):gsub("%s+$", "")))
    end
  end
end

local files, kinds, dirs, code = {}, {}, {}, 0
for _, arg in ipairs(args) do
  local stat, reason = fs.lstat(shell.resolve(arg))
  kinds[arg] = stat and stat.kind
  if stat and stat.kind == "link" and not options.l then
    stat = fs.stat(shell.resolve(arg)) or stat
  end
  if not stat then
    io.stderr:write("ls: cannot access '", arg, "': ", reason, "\n")
    code = 1
  elseif stat.kind == "directory" then
    dirs[#dirs + 1] = arg
  else
    files[#files + 1] = arg
  end
end
table.sort(files)
table.sort(dirs)
show(files, kinds, shell.resolve)
for i, dir in ipairs(dirs) do
  if #args > 1 then
    print((#files > 0 or i > 1) and "\n" .. dir .. ":" or dir .. ":")
  end
  local path, names = shell.resolve(dir), {}
  local listed, listedKinds = fs.list(path)
  for _, name in ipairs(listed) do
    if options.a or name:sub(1, 1) ~= "." then
      names[#names + 1] = name
    end
  end
  table.sort(names)
  show(names, listedKinds, function(name)
    return path .. "/" .. name
  end)
end
return code
