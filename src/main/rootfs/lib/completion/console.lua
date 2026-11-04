local completion = require("completion")

return function(args, word)
  return completion.operand(args, word) == 1 and completion.ports(word) or {}
end
