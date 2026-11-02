local completion = require("completion")

return function(_, word)
  return completion.paths(word, true)
end
