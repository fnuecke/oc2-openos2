local shell = require("shell")

local USAGE = [[
Usage: date [-w] [format]
Prints the date and time in UTC.
  -w  print the age of the world instead
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "w")
if invalid then
  return invalid
end
if options.w then
  local seconds = computer.worldTime()
  if not seconds then
    io.stderr:write("date: no world clock\n")
    return 1
  end
  print(string.format("world age: %d days, %02d:%02d", seconds // 86400, seconds // 3600 % 24, seconds // 60 % 60))
  return 0
end
local ok, text = pcall(os.date, args[1] or "%F %T")
if not ok then
  io.stderr:write("date: ", tostring(text):match("%((.*)%)$") or tostring(text), "\n", USAGE)
  return 1
end
print(text)
