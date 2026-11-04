local completion = require("completion")

return function(args, word)
  if args[#args - (word ~= "" and 1 or 0)] ~= "-F" then
    return {}
  end
  return completion.ports(word)
end
