-- Shadows oc2's /mnt/builtin/bin/oc2busd.lua, the bus daemon for Linux.
local USAGE = [[
Usage: oc2busd
Only useful on Linux, OpenOS II does not need it.
]]

if ... == "-h" or ... == "--help" then
  io.write(USAGE)
  return 0
end
io.stderr:write("oc2busd: not needed on OpenOS II\n")
return 1
