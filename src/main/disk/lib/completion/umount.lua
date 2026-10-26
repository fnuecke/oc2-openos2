return function(args, word)
  local given = {}
  for i = 2, #args - (word ~= "" and 1 or 0) do
    given[args[i]] = true
  end
  local candidates = {}
  for _, mount in ipairs(fs.mounts()) do
    if mount.index and mount.path ~= "/" and mount.path:sub(1, #word) == word and not given[mount.path] then
      candidates[#candidates + 1] = mount.path
    end
  end
  return candidates
end
