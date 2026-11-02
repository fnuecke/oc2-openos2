local medium = require("medium")
local shell = require("shell")

local USAGE = [[
Usage: mount [device]
Lists the mounts, or attaches a drive detached by umount, e.g. /dev/vdb.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args > 1 then
  return shell.usage(USAGE)
end
if args[1] then
  local ok, reason = fs.mount(shell.resolve(args[1]))
  if not ok then
    io.stderr:write("mount: ", args[1], ": ", reason, "\n")
    return 1
  end
  return
end
for _, mount in ipairs(fs.mounts()) do
  local usable = medium.isUsable(mount)
  local other = mount.medium == "blank" and ", not formatted" or not usable and ", " .. mount.medium or ""
  print((medium.device(mount) or mount.label) .. " on " .. mount.path
    .. (usable and " type " .. mount.medium or "") .. " (" .. (mount.readOnly and "ro" or "rw") .. other .. ")")
end
