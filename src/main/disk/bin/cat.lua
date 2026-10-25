local shell = require("shell")

local USAGE = [[
Usage: cat [file...]
Prints the files, or the input if there are none or a file is "-".
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "-"
end
local code = 0
for _, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path)
  end
  if not file then
    io.stderr:write("cat: ", reason, "\n")
    code = 1
  else
    repeat
      local chunk, readReason = file:read(path == "-" and "L" or 4096)
      if chunk then
        io.write(chunk)
      elseif readReason then
        io.stderr:write("cat: ", path, ": ", readReason, "\n")
        code = 1
      end
    until not chunk
    if path ~= "-" then
      file:close()
    end
  end
end
return code
