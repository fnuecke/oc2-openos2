local shell = require("shell")

local USAGE = [[
Usage: date [-w] [format]
Prints the date and time in UTC, by default as "%F %T". The format knows %Y %m
%d %H %M %S %j %F %T %x %X %c and %%. -w prints how long the Minecraft world
has existed instead, where the computer has a world clock: that is the world's
total time, which sleeping or /time set do not change.
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
  io.stderr:write("date: ", tostring(text):gsub("^.-: ", ""), "\n", USAGE)
  return 1
end
print(text)
