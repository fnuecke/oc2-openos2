local shell = require("shell")

local USAGE = [[
Usage: free
Prints how much memory is used and free.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
collectgarbage()
local total, free = computer.totalMemory(), computer.freeMemory()
print(string.format("total %d KiB, used %d KiB, free %d KiB, lua %d KiB",
  total // 1024, (total - free) // 1024, free // 1024, collectgarbage("count") // 1))
