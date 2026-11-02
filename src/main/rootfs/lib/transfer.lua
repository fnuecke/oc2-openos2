local paths = require("path")

local transfer = {}

local CHUNK_SIZE = 4096

local function real(path)
  local dir = paths.path(path)
  return fs.realpath(path) or (fs.realpath(dir) or dir):gsub("/$", "") .. "/" .. (paths.name(path) or "")
end

local function isDir(path)
  local stat = fs.stat(path)
  return stat ~= nil and stat.kind == "directory"
end

local function copyFile(from, to)
  if real(from) == real(to) then
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
  local closed, closeReason = output:close()
  if ok and not closed then
    ok, reason = false, closeReason
  end
  if not ok then
    return nil, failed .. ": " .. tostring(reason)
  end
  return true
end

local function copyLink(from, to)
  local target, reason = fs.readlink(from)
  if not target then
    return nil, from .. ": " .. reason
  end
  local existing = fs.lstat(to)
  if from ~= to and existing and existing.kind ~= "directory" then
    fs.remove(to)
  end
  local ok
  ok, reason = fs.symlink(target, to)
  if not ok then
    return nil, to .. ": " .. reason
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
  local stat = recursive and fs.lstat(from)
  if stat and stat.kind == "link" then
    return copyLink(from, to)
  elseif not isDir(from) then
    return copyFile(from, to)
  end
  if not recursive then
    return nil, "-r not specified; omitting directory '" .. from .. "'"
  end
  local realFrom, realTo = real(from), real(to)
  if (realTo .. "/"):sub(1, #realFrom + 1) == realFrom .. "/" then
    return nil, "cannot copy a directory, '" .. from .. "', into itself, '" .. to .. "'"
  end
  if not isDir(to) then
    local ok, reason = fs.mkdir(to)
    if not ok then
      return nil, to .. ": " .. reason
    end
  end
  local names, kinds = fs.list(from)
  for _, entry in ipairs(names) do
    if kinds[entry] ~= "device" then
      local name = entry:gsub("/$", "")
      local ok, reason = transfer.copy(from .. "/" .. name, to .. "/" .. name, true)
      if not ok then
        return nil, reason
      end
    end
  end
  return true
end

function transfer.remove(path, recursive)
  local stat = recursive and fs.lstat(path)
  if stat and stat.kind == "directory" then
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
  if transfer.mountOf(real(paths.path(from))) == transfer.mountOf(real(paths.path(to))) then
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
