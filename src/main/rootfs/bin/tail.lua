local shell = require("shell")

local USAGE = [[
Usage: tail [-f] [-n [+]<lines> | -<lines>] [file...]
Prints the last lines of the files, or of the input.
  -n lines   number of lines, default 10
  -n +line   start at the line
  -f         keep printing what is added to the files, until Ctrl+C
]]

local raw = {}
for _, arg in ipairs({...}) do
  local count = arg:match("^%-n(%+?%d+)$") or arg:match("^%-(%d+)$")
  if count then
    raw[#raw + 1], raw[#raw + 2] = "-n", count
  else
    raw[#raw + 1] = arg
  end
end
local args, options = shell.parse(table.unpack(raw))
local invalid = shell.check(options, USAGE, "nf")
if invalid then
  return invalid
end
local count, from = 10, nil
if options.n then
  local value = table.remove(args, 1) or ""
  from, count = value:match("^%+(%d+)$"), tonumber(value:match("^%d+$"))
  from = tonumber(from)
  if not from and not count then
    return shell.usage(USAGE)
  end
end
if #args == 0 then
  args[1] = "-"
end

local function lastFromEnd(file)
  local position, chunks, ends = file:seek("end"), {}, 0
  while position > 0 and ends <= count do
    local size = math.min(4096, position)
    position = position - size
    file:seek("set", position)
    local chunk = file:read(size)
    table.insert(chunks, 1, chunk)
    ends = ends + select(2, chunk:gsub("\n", ""))
  end
  local data, starts = table.concat(chunks), {1}
  for start in data:gmatch("\n()") do
    starts[#starts + 1] = start
  end
  if starts[#starts] > #data then
    starts[#starts] = nil
  end
  io.write(data:sub(starts[math.max(1, #starts - count + 1)] or 1))
  file:seek("end")
end

local function lastFromStream(file)
  local lines, first, last = {}, 1, 0
  for line in file:lines("L") do
    last = last + 1
    lines[last] = line
    if last - first >= count then
      lines[first], first = nil, first + 1
    end
  end
  io.write(table.concat(lines, "", first, last))
end

local function fromLine(file)
  local number = 1
  for line in file:lines("L") do
    if number >= from then
      io.write(line)
    end
    number = number + 1
  end
end

local followed, code = {}, 0
for i, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path, "rb")
  end
  if not file then
    io.stderr:write("tail: cannot open '", path, "' for reading: ", (reason:gsub("^.-: ", "")), "\n")
    code = 1
  else
    if #args > 1 then
      print((i > 1 and "\n" or "") .. "==> " .. path .. " <==")
    end
    if from then
      fromLine(file)
    elseif path ~= "-" and file:seek("end") then
      if count > 0 then
        lastFromEnd(file)
      end
    elseif count > 0 then
      lastFromStream(file)
    end
    if options.f and path ~= "-" then
      followed[#followed + 1] = {file = file, path = path}
    elseif path ~= "-" then
      file:close()
    end
  end
end
io.flush()

local current = followed[#followed]
while #followed > 0 do
  computer.sleep(1)
  for _, entry in ipairs(followed) do
    for data in entry.file:lines(4096) do
      if entry ~= current and #args > 1 then
        print("\n==> " .. entry.path .. " <==")
      end
      current = entry
      io.write(data)
    end
  end
  io.flush()
end
return code
