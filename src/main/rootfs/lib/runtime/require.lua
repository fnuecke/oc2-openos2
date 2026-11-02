-- require with yield support for device bus calls in module init code.
function require(name)
  if type(name) ~= "string" then
    error("bad argument #1 to 'require' (string expected, got " .. type(name) .. ")", 2)
  end
  local module = package.loaded[name]
  if module ~= nil then
    return module
  end
  local reasons = {}
  for _, searcher in ipairs(package.searchers) do
    local loader, data = searcher(name)
    if type(loader) == "function" then
      local result = loader(name, data)
      if result ~= nil then
        package.loaded[name] = result
      elseif package.loaded[name] == nil then
        package.loaded[name] = true
      end
      return package.loaded[name], data
    end
    reasons[#reasons + 1] = loader
  end
  error("module '" .. name .. "' not found:\n\t" .. table.concat(reasons, "\n\t"), 2)
end
