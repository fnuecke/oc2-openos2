local shell = require("shell")

local USAGE = [[
Usage: tee [-a] [file...]
Copies the input to the output and to the files.
  -a  append to the files
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "a")
if invalid then
  return invalid
end
local files, code = {}, 0
for _, path in ipairs(args) do
  local file, reason = io.open(path, options.a and "a" or "w")
  if file then
    files[#files + 1] = file
  else
    io.stderr:write("tee: ", reason, "\n")
    code = 1
  end
end
while true do
  local line, reason = io.read("L")
  if not line then
    if reason then
      io.stderr:write("tee: ", reason, "\n")
      code = 1
    end
    break
  end
  io.write(line)
  for _, file in ipairs(files) do
    file:write(line)
  end
end
for _, file in ipairs(files) do
  file:close()
end
return code
