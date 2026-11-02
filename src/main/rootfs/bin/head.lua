local shell = require("shell")

local USAGE = [[
Usage: head [-n <lines> | -<lines> | -c <bytes>] [file...]
Prints the first lines of the files, or of the input.
  -n lines  number of lines, default 10
  -c bytes  number of bytes
]]

local raw = {}
for _, arg in ipairs({...}) do
  local count = arg:match("^%-n(%d+)$") or arg:match("^%-(%d+)$")
  local bytes = arg:match("^%-c(%d+)$")
  if count or bytes then
    raw[#raw + 1], raw[#raw + 2] = count and "-n" or "-c", count or bytes
  else
    raw[#raw + 1] = arg
  end
end
local args, options = shell.parse(table.unpack(raw))
local invalid = shell.check(options, USAGE, "nc")
if invalid then
  return invalid
end
local count = 10
if options.n and options.c then
  return shell.usage(USAGE)
elseif options.n or options.c then
  count = tonumber(table.remove(args, 1))
  if not count then
    return shell.usage(USAGE)
  end
end
if #args == 0 then
  args[1] = "-"
end
local code = 0
for i, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path)
  end
  if not file then
    io.stderr:write("head: cannot open '", path, "' for reading: ", (reason:gsub("^.-: ", "")), "\n")
    code = 1
  else
    if #args > 1 then
      print((i > 1 and "\n" or "") .. "==> " .. path .. " <==")
    end
    local left = count
    while left > 0 do
      local data, readReason = file:read(options.c and math.min(left, 4096) or "L")
      if not data then
        if readReason then
          io.stderr:write("head: ", path, ": ", readReason, "\n")
          code = 1
        end
        break
      end
      io.write(data)
      left = left - (options.c and #data or 1)
    end
    if path ~= "-" then
      file:close()
    end
  end
end
return code
