local completion = require("completion")
local medium = require("medium")

return function(args, word)
  return completion.operand(args, word) == 1 and completion.drives(word, medium.isUsable) or {}
end
