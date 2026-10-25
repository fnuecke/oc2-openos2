-- Shadows oc2's /mnt/builtin/bin/setup-network.lua, which configures Linux's network interfaces.
local USAGE = [[
Usage: setup-network
Only useful on Linux. OpenOS II has no networking; use serial interface cards.
]]

if ... == "-h" or ... == "--help" then
  io.write(USAGE)
  return 0
end
io.stderr:write("setup-network: OpenOS II has no networking\n")
return 1
