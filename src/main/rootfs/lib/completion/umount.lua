local completion = require("completion")

return function(args, word)
  return completion.operand(args, word) == 1 and completion.drives(word, function(mount)
    return not mount.unmounted and mount.path ~= "/"
  end) or {}
end
