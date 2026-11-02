local completion = require("completion")

return function(args, word)
  return completion.operand(args, word) == 1 and completion.commands(word, true) or completion.paths(word)
end
