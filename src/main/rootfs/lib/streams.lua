local path = require("path")
local term = require("term")

local STREAMS = {stdin = "input", stdout = "output", stderr = "errors", tty = "console", console = "console"}

local streams = {}

local function name(p)
  p = path.canonical(p)
  return p == "/dev" and "" or p:match("^/dev/(.*)$")
end

function streams.list(list, p)
  local names, kinds = list(p)
  if names and name(p) == "" then
    for entry in pairs(STREAMS) do
      names[#names + 1] = entry
      kinds[entry] = "device"
    end
    table.sort(names)
  end
  return names, kinds
end

function streams.stat(stat, p)
  if STREAMS[name(p)] then
    return {size = 0, kind = "device", mtime = 0}
  end
  return stat(p)
end
streams.lstat = streams.stat

function streams.exists(exists, p)
  return STREAMS[name(p)] ~= nil or exists(p)
end

local function keepOpen()
  return true
end

local function relay(file, stream, first, ...)
  if first == stream then
    return file, ...
  end
  return first, ...
end

local File = {__close = keepOpen}
function File.__index(file, key)
  local stream = file.stream
  local value = stream[key]
  if type(value) ~= "function" then
    return value
  end
  local method = function(_, ...)
    return relay(file, stream, value(stream, ...))
  end
  rawset(file, key, method)
  return method
end

function streams.open(p, mode, record)
  local key = STREAMS[name(p)]
  if not key then
    return nil
  end
  local writing, both = (mode or "r"):find("^[wa]") ~= nil, (mode or ""):find("+", 1, true) ~= nil
  if key == "input" and (writing or both) or (key == "output" or key == "errors") and (not writing or both) then
    return false, path.canonical(p) .. ": Permission denied"
  end
  local stream = record[key] or term.stream
  if stream == term.stream then
    return stream
  elseif type(stream) == "table" and stream.pipe then
    local methods = getmetatable(stream).__index
    return setmetatable({pipe = stream.pipe, close = keepOpen}, {__index = methods, __close = keepOpen})
  end
  return setmetatable({stream = stream, close = keepOpen}, File)
end

return streams
