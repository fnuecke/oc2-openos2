local USAGE = [[
Usage: luac [-p] [-s] [-o output] file.lua
Compiles a Lua file to bytecode. The output is the file with .luac instead of
.lua, unless given. The shell runs file.luac in place of file.lua.
  -o output  write the bytecode to output
  -p         only check the syntax, write nothing
  -s         strip debug information
]]

local args = {...}
local options, input, output = {}, nil, nil
local i = 1
while i <= #args do
  local arg = args[i]
  if arg == "-h" or arg == "--help" then
    io.write(USAGE)
    return 0
  elseif arg == "-o" and args[i + 1] then
    output, i = args[i + 1], i + 1
  elseif arg == "-p" or arg == "-s" then
    options[arg] = true
  elseif arg:sub(1, 1) == "-" or input then
    io.stderr:write(arg:sub(1, 1) == "-" and "unknown option: " .. arg .. "\n" or "", USAGE)
    return 1
  else
    input = arg
  end
  i = i + 1
end
if not input then
  io.stderr:write(USAGE)
  return 1
end
local source = io.open(input)
if not source then
  io.stderr:write("luac: cannot open ", input, "\n")
  return 1
end
local chunk, reason = load(source:read("a"), "@" .. input, "t")
source:close()
if not chunk then
  io.stderr:write("luac: ", reason, "\n")
  return 1
end
if options["-p"] then
  return 0
end
output = output or input:gsub("%.lua$", "") .. ".luac"
local file, openReason = io.open(output, "w")
if not file then
  io.stderr:write("luac: cannot open ", output, ": ", (openReason:gsub("^.-: ", "")), "\n")
  return 1
end
local ok, writeReason = file:write(string.dump(chunk, options["-s"]))
file:close()
if not ok then
  io.stderr:write("luac: ", output, ": ", writeReason, "\n")
  return 1
end
return 0
