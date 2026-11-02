local event = {}

local MAX_SKIPPED = 64
local QUEUE_SIZE = 64

local skipped = {}

function event.pullFiltered(timeout, filter)
  if type(timeout) == "function" then
    timeout, filter = nil, timeout
  end
  local threads = package.loaded.thread
  local inThread = threads and threads.current()
  for i, signal in ipairs(inThread and {} or skipped) do
    if filter(table.unpack(signal, 1, signal.n)) then
      table.remove(skipped, i)
      return table.unpack(signal, 1, signal.n)
    end
  end
  local deadline = timeout and computer.uptime() + timeout
  local left = QUEUE_SIZE
  while true do
    local signal = table.pack(computer.pullSignal(deadline and math.max(0, deadline - computer.uptime())))
    if signal.n == 0 then
      return
    end
    if filter(table.unpack(signal, 1, signal.n)) then
      return table.unpack(signal, 1, signal.n)
    end
    if not inThread then
      if #skipped == MAX_SKIPPED then
        table.remove(skipped, 1)
      end
      skipped[#skipped + 1] = signal
    end
    left = left - 1
    if deadline and computer.uptime() >= deadline and (timeout > 0 or left == 0) then
      return
    end
  end
end

function event.pull(timeout, name)
  return event.pullFiltered(timeout, function(signalName)
    return name == nil or signalName == name
  end)
end

event.push = computer.pushSignal

return event
