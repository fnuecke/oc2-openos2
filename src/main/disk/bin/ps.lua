local shell = require("shell")

local USAGE = [[
Usage: ps
Lists the running programs and background threads.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
print(string.format("%5s %5s %-8s %s", "PID", "PPID", "STAT", "COMMAND"))
local pids = {}
for pid in pairs(shell.processes) do
  pids[#pids + 1] = pid
end
table.sort(pids)
for _, pid in ipairs(pids) do
  local process = shell.processes[pid]
  local status = process.streams.process == process and "running" or "waiting"
  print(string.format("%5d %5d %-8s %s", pid, process.parent, status, process.command))
end
local thread = package.loaded.thread
if thread then
  for _, t in ipairs(thread.list()) do
    print(string.format("%5s %5d %-8s %s", "t" .. t.id, t.parent, t:status(), t.name))
  end
end
