local event = require("event")
local keyboard = require("keyboard")
local shell = require("shell")

local USAGE = [[
Usage: keytest
Prints the keys pressed on a keyboard, quit with Esc.
]]

local _, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
if invalid then
  return invalid
end
if #fs.list("/sys/class/input") == 0 then
  io.stderr:write("keytest: no keyboard\n")
  return 1
end

print("Press keys on a keyboard, quit with Esc.")
while true do
  local _, name, char, code = event.pull(nil, "key_down")
  local shown = char == 0 and "-" or keyboard.isControl(char) and "^" .. string.char((char + 64) % 128)
    or utf8.char(char)
  print(string.format("%s: %-12s %-4s code %d", name, keyboard.keys[code] or "?", shown, code))
  if code == keyboard.keys.escape then
    break
  end
end
