local transfer = {}

local CHUNK_SIZE = 4096

local function isDir(path)
  local stat = fs.stat(path)
  return stat ~= nil and stat.isDir
end

local function copyFile(from, to)
  if from == to then
    return nil, "'" .. from .. "' and '" .. to .. "' are the same file"
  end
  local input, reason = io.open(from, "r")
  if not input then
    return nil, reason
  end
  local output
  output, reason = io.open(to, "w")
  if not output then
    input:close()
    return nil, reason
  end
  local ok, failed = true, to
  while ok do
    local chunk, readReason = input:read(CHUNK_SIZE)
    if not chunk then
      ok, reason, failed = readReason == nil, readReason, from
      break
    end
    ok, reason = output:write(chunk)
  end
  input:close()
  output:close()
  if not ok then
    return nil, failed .. ": " .. tostring(reason)
  end
  return true
end

function transfer.mountOf(path)
  local best = "/"
  for _, mount in ipairs(fs.mounts()) do
    local prefix = mount.path == "/" and "/" or mount.path .. "/"
    if (path .. "/"):sub(1, #prefix) == prefix and #mount.path > #best then
      best = mount.path
    end
  end
  return best
end

function transfer.copy(from, to, recursive)
  if not isDir(from) then
    return copyFile(from, to)
  end
  if not recursive then
    return nil, "-r not specified; omitting directory '" .. from .. "'"
  end
  if (to .. "/"):sub(1, #from + 1) == from .. "/" then
    return nil, "cannot copy a directory, '" .. from .. "', into itself, '" .. to .. "'"
  end
  if not isDir(to) then
    local ok, reason = fs.mkdir(to)
    if not ok then
      return nil, to .. ": " .. reason
    end
  end
  for _, name in ipairs(fs.list(from)) do
    name = name:gsub("/$", "")
    local stat = fs.stat(from .. "/" .. name)
    if not (stat and stat.isDevice) then
      local ok, reason = transfer.copy(from .. "/" .. name, to .. "/" .. name, true)
      if not ok then
        return nil, reason
      end
    end
  end
  return true
end

function transfer.remove(path, recursive)
  if recursive and isDir(path) then
    for _, name in ipairs(fs.list(path)) do
      local ok, reason = transfer.remove(path .. "/" .. name:gsub("/$", ""), true)
      if not ok then
        return nil, reason
      end
    end
  end
  local ok, reason = fs.remove(path)
  if not ok then
    return nil, path .. ": " .. reason
  end
  return true
end

function transfer.move(from, to)
  if transfer.mountOf(from) == transfer.mountOf(to) then
    local ok, reason = fs.rename(from, to)
    if not ok then
      return nil, from .. ": " .. reason
    end
    return true
  end
  local ok, reason = transfer.copy(from, to, true)
  if not ok then
    return nil, reason
  end
  return transfer.remove(from, true)
end

return transfer
