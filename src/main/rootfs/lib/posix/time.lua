local kernel = require("kernel.time")
local poll = require("posix.poll")

local time = {}
for name, value in pairs(kernel) do
  time[name] = value
end

function time.nanosleep(request)
  if not poll.canWait() then
    return kernel.nanosleep(request)
  end
  os.sleep((request.tv_sec or 0) + (request.tv_nsec or 0) / 1e9)
  return 0
end

return time
