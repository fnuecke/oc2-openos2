local path = require("path")
local glob = require("glob")
local shell = require("shell")

local USAGE = [[
Usage: find [path...] [-x] [-name pattern] [-type f|d|l]
Prints the files and directories below each path, or the current one.
  -x             stay on the file system of each path
  -name pattern  only names that match, with wildcards as in sh
  -type f|d|l    only files, directories or symbolic links
]]

local args = {...}
if args[1] == "-h" or args[1] == "--help" then
  io.write(USAGE)
  return 0
end
local paths, pattern, kind, mounts = {}, nil, nil, nil
local i = 1
while i <= #args do
  local arg = args[i]
  if (arg == "-name" or arg == "-type") and not args[i + 1] then
    io.stderr:write("find: missing argument to '", arg, "'\n")
    return 1
  elseif arg == "-x" or arg == "-xdev" then
    mounts = require("medium").mountPoints()
  elseif arg == "-name" then
    pattern = args[i + 1]
    i = i + 1
  elseif arg == "-type" and (args[i + 1] == "f" or args[i + 1] == "d" or args[i + 1] == "l") then
    kind = args[i + 1]
    i = i + 1
  elseif arg:sub(1, 1) == "-" then
    io.stderr:write("find: unknown predicate '", arg, "'\n", USAGE)
    return 1
  else
    paths[#paths + 1] = arg
  end
  i = i + 1
end
if #paths == 0 then
  paths[1] = "."
end

local top
local TYPES = {directory = "d", link = "l"}

local function visit(name, entryKind)
  local base = path.name(name) or name
  if (not pattern or glob.match(pattern, base)) and (not kind or (TYPES[entryKind] or "f") == kind) then
    print(name)
  end
  if entryKind == "directory" and not (mounts and mounts[shell.resolve(name)] and name ~= top) then
    local entries, kinds = fs.list(shell.resolve(name))
    entries = entries or {}
    table.sort(entries)
    for _, entry in ipairs(entries) do
      visit(name:gsub("/$", "") .. "/" .. entry:gsub("/$", ""), kinds[entry])
    end
  end
end

local code = 0
for _, arg in ipairs(paths) do
  top = arg
  local stat, reason = fs.lstat(shell.resolve(arg))
  if not stat then
    io.stderr:write("find: '", arg, "': ", reason, "\n")
    code = 1
  else
    visit(arg, stat.kind)
  end
end
return code
