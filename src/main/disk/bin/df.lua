local medium = require("medium")
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
print(string.format("%-12s %9s %9s %9s %4s %s", "Filesystem", "1K-blocks", "Used", "Available", "Use%", "Mounted on"))
for _, mount in ipairs(fs.mounts()) do
  local name = mount.index and medium.device(mount.index) or mount.label
  if mount.medium == "ext2" or mount.medium == "9p" then
    local used = mount.size - mount.free
    print(string.format("%-12s %9d %9d %9d %3d%% %s", name, mount.size // 1024, used // 1024,
      mount.free // 1024, mount.size > 0 and used * 100 // mount.size or 0, mount.path))
  else
    print(string.format("%-12s %9d %9s %9s %4s %s (%s)", name, mount.size // 1024, "-", "-", "-", mount.path,
      mount.medium == "blank" and "not formatted" or mount.medium))
  end
end
