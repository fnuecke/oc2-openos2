local medium = require("medium")

return function(args, word)
  local given = {}
  for i = 2, #args - (word ~= "" and 1 or 0) do
    given[args[i]] = true
  end
  local candidates = {}
  for _, mount in ipairs(fs.mounts()) do
    local device = mount.index and medium.device(mount.index)
    for _, name in ipairs(device and mount.path ~= "/" and {device, mount.path} or {}) do
      if name:sub(1, #word) == word and not given[name] then
        candidates[#candidates + 1] = name
      end
    end
  end
  return candidates
end
