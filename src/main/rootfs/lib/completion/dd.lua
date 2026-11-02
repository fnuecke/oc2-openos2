local completion = require("completion")

return function(_, word)
  local key, path = word:match("^([io]f=)(.*)$")
  if not key then
    return {}
  end
  local candidates = completion.paths(path)
  for i, candidate in ipairs(candidates) do
    candidates[i] = key .. candidate
  end
  return candidates
end
