-- Signals from computer.pullSignal. A pull for one signal name keeps the others for later pulls.

local event = {}

local MAX_SKIPPED = 64

local skipped = {} -- signals passed over by a pull with a name, oldest first
local interrupts = console.interrupts()

-- Ctrl+C drops console input typed ahead of it, including what was already pulled.
local function dropTypedAhead()
  local count = console.interrupts()
  if count == interrupts then
    return
  end
  interrupts = count
  for i = #skipped, 1, -1 do
    if skipped[i][1] == "console" then
      table.remove(skipped, i)
    end
  end
end

-- Returns the next signal with the given name, or any signal if name is nil. Returns nothing after
-- timeout seconds, or never times out if timeout is nil.
function event.pull(timeout, name)
  dropTypedAhead()
  for i, signal in ipairs(skipped) do
    if name == nil or signal[1] == name then
      table.remove(skipped, i)
      return table.unpack(signal, 1, signal.n)
    end
  end
  local deadline = timeout and computer.uptime() + timeout
  while true do
    local signal = table.pack(computer.pullSignal(deadline and math.max(0, deadline - computer.uptime())))
    if signal.n == 0 then
      return
    end
    if name == nil or signal[1] == name then
      return table.unpack(signal, 1, signal.n)
    end
    if #skipped == MAX_SKIPPED then
      table.remove(skipped, 1)
    end
    skipped[#skipped + 1] = signal
  end
end

event.push = computer.pushSignal

return event
