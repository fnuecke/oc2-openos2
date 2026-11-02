local medium = require("medium")
local shell = require("shell")

local USAGE = [[
Usage: label [-c] <device> [label]
Prints or sets the label of the medium in a drive, e.g. /dev/vdb.
  -c  clear the label
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "c")
if invalid then
  return invalid
end
if #args < 1 or #args > (options.c and 1 or 2) then
  return shell.usage(USAGE)
end
local path = shell.resolve(args[1])
local target = medium.find(path)
local generation = target and target.generation
local new = options.c and "" or args[2]
local label, reason = fs.label(path, new, generation)
if not label then
  io.stderr:write("label: ", args[1], ": ", reason, "\n")
  return 1
end
if new == nil then
  print(label)
  return 0
end
for _, mount in ipairs(fs.mounts()) do
  if target and medium.device(mount) == path and mount.path ~= target.path then
    print("mounted at " .. mount.path)
  end
end
