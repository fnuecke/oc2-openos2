local paths = require("runtime.paths")
local system = require("system")
local term = require("term")

local absolute, isDevice = paths.absolute, paths.isDevice
local canonical = require("path").canonical

local kernelOpen, kernelLines, kernelType = io.open, io.lines, io.type
local streams = {input = term.stream, output = term.stream}
function system.streams(record)
  local active = streams
  streams = record or streams
  return active
end
io.stdin, io.stdout, io.stderr = nil, nil, nil
setmetatable(io, {__index = function(_, key)
  if key == "stdin" then
    return streams.input
  elseif key == "stdout" then
    return streams.output
  elseif key == "stderr" then
    return streams.errors or term.stream
  end
end})

local function openStream(path, mode)
  if isDevice(path) then
    return require("streams").open(path, mode, streams)
  end
end

function io.open(path, mode)
  path = absolute(path)
  local stream, reason = openStream(path, mode)
  if stream ~= nil then
    return stream or nil, reason
  end
  return kernelOpen(type(path) == "string" and canonical(path) or path, mode)
end

local function openIfPath(file, mode)
  if type(file) == "string" then
    file = assert(io.open(file, mode))
    local files = streams.files
    if files then
      files[#files + 1] = file
    end
  end
  return file
end

function io.input(file)
  streams.input = file and openIfPath(file, "r") or streams.input
  return streams.input
end

function io.output(file)
  streams.output = file and openIfPath(file, "w") or streams.output
  return streams.output
end

function io.lines(path, ...)
  if path == nil then
    return streams.input:lines(...)
  end
  path = absolute(path)
  local stream, reason = openStream(path, "r")
  if stream ~= nil then
    return assert(stream, reason):lines(...)
  end
  return kernelLines(type(path) == "string" and canonical(path) or path, ...)
end

function io.popen(command, mode)
  return require("pipe").popen(command, mode)
end

function io.read(...)
  return streams.input:read(...)
end

function io.write(...)
  return streams.output:write(...)
end

function io.flush()
  return streams.output:flush()
end

function io.close(file)
  return (file or streams.output):close()
end

function io.type(file)
  if file == term.stream or (type(file) == "table" and file.pipe) then
    return "file"
  elseif type(file) == "table" and file.stream then
    return io.type(file.stream)
  end
  return kernelType(file)
end

function print(...)
  local output = streams.output
  for i = 1, select("#", ...) do
    if i > 1 then
      output:write("\t")
    end
    output:write(tostring((select(i, ...))))
  end
  output:write("\n")
end
