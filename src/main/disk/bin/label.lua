local shell = require("shell")

local USAGE = [[
Usage: label [-c] <path> [label]
Prints the label of the medium mounted at <path>, sets it, or with -c clears
it. A labeled medium is mounted at /mnt/<label>, otherwise at /mnt/<drive
number>. Labels are up to 16 of A-Z a-z 0-9 . _ -, and not only digits.
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
local index, serial
for _, mount in ipairs(fs.mounts()) do
  if mount.path == path then
    index, serial = mount.index, mount.serial
  end
end
local new = options.c and "" or args[2]
local label, reason = fs.label(path, new, serial)
if not label then
  io.stderr:write("label: ", args[1], ": ", reason, "\n")
  return 1
end
if new == nil then
  print(label)
  return 0
end
for _, mount in ipairs(fs.mounts()) do
  if index and mount.index == index and mount.path ~= path then
    print("mounted at " .. mount.path)
  end
end
