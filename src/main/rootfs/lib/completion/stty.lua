local completion = require("completion")

return function(args, word)
  if args[#args - (word ~= "" and 1 or 0)] ~= "-F" then
    return {}
  end
  local ports = {}
  for _, name in ipairs(fs.list("/dev") or {}) do
    if name:find("^ttyS%d+$") then
      ports[#ports + 1] = "/dev/" .. name
    end
  end
  return completion.matches(word, ports)
end
