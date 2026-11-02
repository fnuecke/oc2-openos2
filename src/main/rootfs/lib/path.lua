local path = {}

function path.segments(p)
  local parts = {}
  for part in p:gmatch("[^/]+") do
    if part == ".." then
      parts[#parts] = nil
    elseif part ~= "." then
      parts[#parts + 1] = part
    end
  end
  return parts
end

function path.canonical(p)
  local result = table.concat(path.segments(p), "/")
  return p:sub(1, 1) == "/" and "/" .. result or result
end

function path.concat(...)
  return path.canonical(table.concat({...}, "/"))
end

function path.name(p)
  local parts = path.segments(p)
  return parts[#parts]
end

function path.path(p)
  local parts = path.segments(p)
  local result = table.concat(parts, "/", 1, #parts - 1) .. "/"
  if p:sub(1, 1) == "/" and result:sub(1, 1) ~= "/" then
    return "/" .. result
  end
  return result
end

function path.resolve(p)
  if p:sub(1, 1) ~= "/" then
    p = (os.getenv("PWD") or "/") .. "/" .. p
  end
  return path.canonical(p)
end

return path
