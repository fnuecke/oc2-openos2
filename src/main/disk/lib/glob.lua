local shell = require("shell")

local glob = {}

local function toLuaPattern(pattern)
  local result, i = {"^"}, 1
  while i <= #pattern do
    local c = pattern:sub(i, i)
    local close = c == "[" and pattern:find("]", i + (pattern:find("^[!^]?%]", i + 1) and 3 or 2), true)
    if c == "\\" and i < #pattern then
      result[#result + 1] = "%" .. pattern:sub(i + 1, i + 1)
      i = i + 1
    elseif c == "*" then
      result[#result + 1] = ".*"
    elseif c == "?" then
      result[#result + 1] = "."
    elseif close then
      local set = pattern:sub(i + 1, close - 1):gsub("^[!^]", "^")
      result[#result + 1] = "[" .. set:gsub("[%%%]%[]", "%%%0") .. "]"
      i = close
    else
      result[#result + 1] = c:find("%w") and c or "%" .. c
    end
    i = i + 1
  end
  return table.concat(result) .. "$"
end

function glob.match(pattern, name)
  return name:find(toLuaPattern(pattern)) ~= nil
end

local function hasWildcard(segment)
  return segment:gsub("\\.", ""):find("[*?[]") ~= nil
end

local function unescape(segment)
  return (segment:gsub("\\(.)", "%1"))
end

function glob.expand(pattern)
  local absolute, directories = pattern:sub(1, 1) == "/", pattern:sub(-1) == "/"
  local matches = {absolute and "/" or ""}
  for segment in pattern:gmatch("[^/]+") do
    local next = {}
    for _, prefix in ipairs(matches) do
      if hasWildcard(segment) then
        local hidden = segment:find("^\\?%.") ~= nil
        for _, entry in ipairs(fs.list(shell.resolve(prefix == "" and "." or prefix)) or {}) do
          local name = entry:gsub("/$", "")
          if (hidden or name:sub(1, 1) ~= ".") and glob.match(segment, name) then
            next[#next + 1] = prefix .. name .. "/"
          end
        end
      else
        next[#next + 1] = prefix .. unescape(segment) .. "/"
      end
    end
    matches = next
  end
  local results = {}
  for _, match in ipairs(matches) do
    local stat = fs.stat(shell.resolve(match))
    if stat and (stat.isDir or not directories) then
      results[#results + 1] = directories and match or match:sub(1, -2)
    end
  end
  table.sort(results)
  return #results > 0 and results or nil
end

return glob
