local USAGE = [[
Usage: stty -F <device> [speed] [setting...]
Prints or sets the speed of a serial port, e.g. stty -F /dev/ttyS1 115200.
Ports are always raw, so raw, -echo, -crtscts and similar settings are
accepted and change nothing.
]]

local IGNORED = {
  ["raw"] = true, ["-raw"] = true, ["cooked"] = true, ["-cooked"] = true, ["sane"] = true,
  ["echo"] = true, ["-echo"] = true, ["crtscts"] = true, ["-crtscts"] = true,
  ["clocal"] = true, ["-clocal"] = true, ["cread"] = true, ["-cread"] = true, ["cs8"] = true,
  ["parenb"] = true, ["-parenb"] = true, ["cstopb"] = true, ["-cstopb"] = true,
  ["ixon"] = true, ["-ixon"] = true, ["ixoff"] = true, ["-ixoff"] = true,
  ["icanon"] = true, ["-icanon"] = true, ["isig"] = true, ["-isig"] = true,
  ["opost"] = true, ["-opost"] = true, ["icrnl"] = true, ["-icrnl"] = true, ["onlcr"] = true, ["-onlcr"] = true,
}

local args = {...}
if args[1] == "-h" or args[1] == "--help" then
  io.write(USAGE)
  return 0
end
local device, speed
local i = 1
while i <= #args do
  local arg = args[i]
  if arg == "-F" then
    device, i = args[i + 1], i + 1
  elseif arg:match("^%d+$") then
    speed = tonumber(arg)
  elseif not IGNORED[arg] then
    io.stderr:write("stty: invalid argument '", arg, "'\n", USAGE)
    return 1
  end
  i = i + 1
end
if not device then
  io.stderr:write(USAGE)
  return 1
end
local serial = require("serial")
local ok, reason = pcall(serial.getBaud, device)
if not ok then
  io.stderr:write("stty: ", device, ": No such file or directory\n")
  return 1
end
if not speed then
  print("speed " .. serial.getBaud(device) .. " baud")
  return 0
end
ok, reason = serial.setBaud(device, speed)
if not ok then
  io.stderr:write("stty: ", device, ": ", reason, "\n")
  return 1
end
