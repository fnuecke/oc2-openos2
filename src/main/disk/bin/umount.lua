local medium = require("medium")
local shell = require("shell")

local USAGE = [[
Usage: umount <path>
Detaches the drive mounted at <path>, so its medium can be removed safely.
The drive is mounted again when its medium changes.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args ~= 1 then
  return shell.usage(USAGE)
end
local target, reason = medium.find(shell.resolve(args[1]))
if not target then
  io.stderr:write("umount: ", args[1], ": ", reason, "\n")
  return 1
end
local ok
ok, reason = fs.unmount(target.path, target.serial)
if not ok then
  io.stderr:write("umount: ", args[1], ": ", reason, "\n")
  return 1
end
