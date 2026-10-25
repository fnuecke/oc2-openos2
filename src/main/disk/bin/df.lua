local shell = require("shell")

local USAGE = [[
Usage: df
Prints the size and free space of each mounted file system, in KiB.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
print(string.format("%-16s %9s %9s %9s %4s", "Mount", "Size", "Used", "Free", "Use"))
for _, mount in ipairs(fs.mounts()) do
  if mount.medium == "ext2" or mount.medium == "9p" then
    local used = mount.size - mount.free
    print(string.format("%-16s %9d %9d %9d %3d%%", mount.path, mount.size // 1024, used // 1024,
      mount.free // 1024, mount.size > 0 and used * 100 // mount.size or 0))
  else
    print(string.format("%-16s %9d  %s", mount.path, mount.size // 1024,
      mount.medium == "blank" and "not formatted" or mount.medium))
  end
end
