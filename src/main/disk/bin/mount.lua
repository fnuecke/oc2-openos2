local shell = require("shell")

local USAGE = [[
Usage: mount
Lists the mounted file systems and their drives. Media are mounted when they
are inserted, at /mnt/<label> or /mnt/<drive number>, see label -h. The
built-in file system is at /mnt/builtin.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
for _, mount in ipairs(fs.mounts()) do
  local medium = mount.medium == "blank" and ", not formatted"
    or (mount.medium == "ext2" or mount.medium == "9p") and "" or ", " .. mount.medium
  print(mount.path .. (mount.index and " on drive " .. mount.index or "") .. " ("
    .. (mount.readOnly and "ro" or "rw") .. medium .. ")")
end
