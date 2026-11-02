require("runtime.require")
require("runtime.errors")
require("runtime.os")
require("runtime.paths")
require("runtime.io")

local system = require("system")
local term = require("term")

print(string.format("%s on OpenOS II, %d KiB free, booted in %.2f s",
  _VERSION, computer.freeMemory() // 1024, computer.uptime()))

local function handleError(err)
  local request = term.powerRequest(err)
  if request then
    if request == "reboot" then
      computer.reboot()
    end
    computer.shutdown()
  end
end

local shell = require("shell")
if fs.exists("/init.d") then
  local names = fs.list("/init.d")
  table.sort(names)
  for _, name in ipairs(names) do
    if name:sub(-4) == ".lua" then
      local ok, err = system.rawpcall(shell.run, "/init.d/" .. name)
      if not ok then
        handleError(err)
      end
    end
  end
end
while true do
  local ok, code = system.rawpcall(shell.execute, "sh")
  if ok and code == 127 then
    ok, code = system.rawpcall(require("repl").run)
  end
  if not ok then
    handleError(code)
  end
end
