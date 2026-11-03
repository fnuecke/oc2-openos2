local completion = require("completion")

return function(args, word)
  local position = completion.operand(args, word) - (args[2] == "-n" and 1 or 0)
  return position == 1 and completion.commands(word, true) or completion.paths(word)
end
