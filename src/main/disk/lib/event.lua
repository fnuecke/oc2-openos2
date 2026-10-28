local event = {}

local MAX_SKIPPED = 64

local skipped = {}
local interrupts = console.interrupts()

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

-- In a thread, signals passed over are not kept: the foreground gets every signal too, see
-- /lib/thread.lua.
function event.pull(timeout, name)
  local threads = package.loaded.thread
  local inThread = threads and threads.current()
  dropTypedAhead()
  for i, signal in ipairs(inThread and {} or skipped) do
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
    if not inThread then
      if #skipped == MAX_SKIPPED then
        table.remove(skipped, 1)
      end
      skipped[#skipped + 1] = signal
    end
  end
end

event.push = computer.pushSignal

return event
