local completion = require("completion")

return function(_, word)
  return completion.commands(word, true)
end
