local medium = require("medium")
local shell = require("shell")

local USAGE = [[
Usage: umount <device>
Detaches a drive, e.g. /dev/vdb, so its medium can be removed safely.
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #args ~= 1 then
  return shell.usage(USAGE)
end
local path = shell.resolve(args[1])
local target, reason = medium.find(path)
reason = target and target.unmounted and "Not mounted" or reason
if not target or target.unmounted then
  io.stderr:write("umount: ", args[1], ": ", reason, "\n")
  return 1
end
local ok
ok, reason = fs.unmount(path, target.generation)
if not ok then
  io.stderr:write("umount: ", args[1], ": ", reason, "\n")
  return 1
end
