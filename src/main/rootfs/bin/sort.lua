local shell = require("shell")

local USAGE = [[
Usage: sort [-n] [-r] [-u] [file...]
Prints the lines of the files, or of the input, sorted.
  -n  compare the numbers at the start of the lines
  -r  reverse the order
  -u  print only the first of lines that compare equal
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "nru")
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "-"
end

local lines = {}
for _, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path)
  end
  if not file then
    io.stderr:write("sort: ", reason, "\n")
    return 2
  end
  while true do
    local line, readReason = file:read("l")
    if not line then
      if path ~= "-" then
        file:close()
      end
      if readReason then
        io.stderr:write("sort: ", path, ": ", readReason, "\n")
        return 2
      end
      break
    end
    lines[#lines + 1] = line
  end
end

if options.n then
  local keys, order = {}, {}
  for i, line in ipairs(lines) do
    keys[i], order[i] = tonumber(line:match("^%s*(%-?%d*%.?%d*)")) or 0, i
  end
  table.sort(order, function(a, b)
    if keys[a] ~= keys[b] then
      return (keys[a] < keys[b]) ~= (options.r == true)
    elseif not options.u and lines[a] ~= lines[b] then
      return (lines[a] < lines[b]) ~= (options.r == true)
    end
    return a < b
  end)
  for j, i in ipairs(order) do
    if not (options.u and j > 1 and keys[order[j - 1]] == keys[i]) then
      io.write(lines[i], "\n")
    end
  end
else
  table.sort(lines, options.r and function(a, b)
    return a > b
  end or nil)
  for i, line in ipairs(lines) do
    if not (options.u and line == lines[i - 1]) then
      io.write(line, "\n")
    end
  end
end
