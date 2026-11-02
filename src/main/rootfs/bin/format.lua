local shell = require("shell")
local medium = require("medium")

local USAGE = [[
Usage: format [-y] [-c] <device> [label]
Creates an empty file system on the medium in a drive, e.g. /dev/vdb.
  -y  do not ask before overwriting data
  -c  clear the label
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "yc")
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
local path = shell.resolve(args[1])
local target, reason = medium.find(path)
if not target then
  return fail(reason)
end
if target.medium ~= "blank" and not options.y and not medium.confirm(target) then
  print("not formatted")
  return 1
end
local label = options.c and "" or args[2]
local ok
ok, reason = fs.format(path, {label = label, generation = target.generation})
if not ok then
  return fail(reason)
end
for _, mount in ipairs(fs.mounts()) do
  if medium.device(mount) == path then
    print(string.format("%s on %s: %d KiB, %d KiB free", path, mount.path, mount.size // 1024,
      mount.free // 1024))
  end
end
