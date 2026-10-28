local medium = require("medium")
local shell = require("shell")

local USAGE = [[
Usage: umount <drive>
Detaches a drive, e.g. /dev/vdb or /mnt/1, so its medium can be removed safely.
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
reason = target and target.unmounted and "Not mounted" or reason
if not target or target.unmounted then
  io.stderr:write("umount: ", args[1], ": ", reason, "\n")
  return 1
end
local ok
ok, reason = fs.unmount(target.path, target.serial)
if not ok then
  io.stderr:write("umount: ", args[1], ": ", reason, "\n")
  return 1
end
