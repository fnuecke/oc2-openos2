local shell = require("shell")

local USAGE = [[
Usage: head [-n <lines> | -<lines>] [file...]
Prints the first 10 lines, or as many as given, of the files or the input.
]]

-- "-n 5", "-n5" and "-5" are the same.
local raw = {}
for _, arg in ipairs({...}) do
  local count = arg:match("^%-n(%d+)$") or arg:match("^%-(%d+)$")
  if count then
    raw[#raw + 1], raw[#raw + 2] = "-n", count
  else
    raw[#raw + 1] = arg
  end
end
local args, options = shell.parse(table.unpack(raw))
local invalid = shell.check(options, USAGE, "n")
if invalid then
  return invalid
end
local count = 10
if options.n then
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
    for _ = 1, count do
      local line, readReason = file:read("L")
      if not line then
        if readReason then
          io.stderr:write("head: ", path, ": ", readReason, "\n")
          code = 1
        end
        break
      end
      io.write(line)
    end
    if path ~= "-" then
      file:close()
    end
  end
end
return code
