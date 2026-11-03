local shell = require("shell")

local USAGE = [[
Usage: cat [file...]
Prints the files, or the input.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "-"
end
local function writeError(reason)
  io.stderr:write("cat: write error: ", reason, "\n")
  return 1
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
        local written, writeReason = io.write(chunk)
        if not written then
          return writeError(writeReason)
        end
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
local flushed, flushReason = io.flush()
if not flushed then
  return writeError(flushReason)
end
return code
