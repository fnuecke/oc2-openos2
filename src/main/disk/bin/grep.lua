local shell = require("shell")

local USAGE = [[
Usage: grep [-i] [-n] [-v] <pattern> [file...]
Prints the lines of the files, or of the input, that match the Lua pattern.
  -i  ignore case
  -n  print line numbers
  -v  print the lines that do not match
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "inv")
if invalid then
  return invalid
end
local pattern = table.remove(args, 1)
if not pattern then
  return shell.usage(USAGE)
end

-- Ignoring case turns each letter into both cases: "a" into "[aA]", and within a set "a" into "aA"
-- and "a-z" into "a-zA-Z".
if options.i then
  local result, inSet, i = {}, false, 1
  while i <= #pattern do
    local c = pattern:sub(i, i)
    if c == "%" then
      result[#result + 1] = pattern:sub(i, i + 1)
      i = i + 1
    elseif c == "[" and not inSet then
      inSet = true
      result[#result + 1] = c
    elseif c == "]" and inSet then
      inSet = false
      result[#result + 1] = c
    elseif inSet and c:match("%a") and pattern:sub(i + 1, i + 1) == "-" and pattern:sub(i + 2, i + 2):match("%a") then
      local last = pattern:sub(i + 2, i + 2)
      result[#result + 1] = c:lower() .. "-" .. last:lower() .. c:upper() .. "-" .. last:upper()
      i = i + 2
    elseif c:match("%a") then
      result[#result + 1] = inSet and c:lower() .. c:upper() or "[" .. c:lower() .. c:upper() .. "]"
    else
      result[#result + 1] = c
    end
    i = i + 1
  end
  pattern = table.concat(result)
end
local valid, problem = pcall(string.find, "", pattern)
if not valid then
  io.stderr:write("grep: ", problem:gsub("^.-: ", ""), "\n")
  return 2
end

if #args == 0 then
  args[1] = "-"
end
local found, code = false, 1
for _, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path)
  end
  if not file then
    io.stderr:write("grep: ", reason, "\n")
    code = 2
  else
    local number = 0
    while true do
      local line, readReason = file:read("l")
      if not line then
        if readReason then
          io.stderr:write("grep: ", path, ": ", readReason, "\n")
          code = 2
        end
        break
      end
      number = number + 1
      if (line:find(pattern) ~= nil) ~= (options.v == true) then
        found = true
        io.write(#args > 1 and path .. ":" or "", options.n and number .. ":" or "", line, "\n")
      end
    end
    if path ~= "-" then
      file:close()
    end
  end
end
return code == 2 and 2 or found and 0 or 1
