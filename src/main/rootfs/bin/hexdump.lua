local shell = require("shell")

local USAGE = [[
Usage: hexdump [-C] [file...]
Prints the bytes of the files, or of the input, as hexadecimal numbers and
characters, 16 per line. Repeated lines are shown as one "*".
  -C  this format, the only one
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "C")
if invalid then
  return invalid
end
if #args == 0 then
  args[1] = "-"
end

local FULL = "%08x  " .. ("%02x "):rep(8) .. " " .. ("%02x "):rep(8) .. " |"
local offset, pending, previous, squeezing = 0, "", nil, false
local function dump(line)
  if line == previous then
    if not squeezing then
      print("*")
      squeezing = true
    end
  elseif #line == 16 then
    print(string.format(FULL, offset, line:byte(1, 16)) .. line:gsub("[^\32-\126]", ".") .. "|")
    previous, squeezing = line, false
  else
    local hex = {}
    for i = 1, #line do
      hex[#hex + 1] = string.format(i == 8 and "%02x  " or "%02x ", line:byte(i))
    end
    print(string.format("%08x  %-49s |%s|", offset, table.concat(hex), (line:gsub("[^\32-\126]", "."))))
  end
  offset = offset + #line
end

local code = 0
for _, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path, "rb")
  end
  if not file then
    io.stderr:write("hexdump: ", reason, "\n")
    code = 1
  else
    while true do
      local chunk, readReason = file:read(path == "-" and "L" or 4096)
      if not chunk then
        if readReason then
          io.stderr:write("hexdump: ", path, ": ", readReason, "\n")
          code = 1
        end
        break
      end
      pending = pending .. chunk
      local whole = #pending - #pending % 16
      for i = 1, whole, 16 do
        dump(pending:sub(i, i + 15))
      end
      pending = pending:sub(whole + 1)
    end
    if path ~= "-" then
      file:close()
    end
  end
end
if #pending > 0 then
  dump(pending)
end
if offset > 0 then
  print(string.format("%08x", offset))
end
return code
