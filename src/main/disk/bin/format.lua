local shell = require("shell")
local medium = require("medium")

local USAGE = [[
Usage: format [-y] [-f] [-c] <path> [label]
Writes a new, empty file system on the medium mounted at <path>, e.g. /mnt/1.
All data on it is lost. Only blank media are formatted without -f. -y does not
ask first. The medium keeps its label, -c clears it, a label argument sets it,
see label -h.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "yfc")
if invalid then
  return invalid
end
if #args < 1 or #args > 2 or (options.c and #args > 1) then
  return shell.usage(USAGE)
end
local function fail(reason)
  io.stderr:write("format: ", args[1], ": ", reason, "\n")
  return 1
end
local target, reason = medium.find(shell.resolve(args[1]))
if not target then
  return fail(reason)
end
local ok
ok, reason = medium.check(target, options.f)
if not ok then
  return fail(reason)
end
if not options.y and not medium.confirm(target) then
  print("not formatted")
  return 1
end
local label = options.c and "" or args[2]
ok, reason = fs.format(target.path, {label = label, force = options.f, serial = target.serial})
if not ok then
  return fail(reason)
end
for _, mount in ipairs(fs.mounts()) do
  if mount.index == target.index then
    print(string.format("%s: %d KiB, %d KiB free", mount.path, mount.size // 1024, mount.free // 1024))
  end
end
