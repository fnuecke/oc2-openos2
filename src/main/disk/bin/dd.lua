local shell = require("shell")
local medium = require("medium")

local USAGE = [[
Usage: dd [-y] [-f] if=<path> of=<path> [bs=n] [count=n] [skip=n] [seek=n]
Copies a file or a whole medium, e.g. /dev/vdb, to a file or a whole medium.
  -y       do not ask before writing to a medium
  -f       overwrite a medium that is not blank, its data is lost
  bs=n     block size in bytes, default 512, with K and M suffixes
  count=n  number of blocks to copy
  skip=n   blocks to skip in the input
  seek=n   blocks to skip in the output
]]

local SIZES = {[""] = 1, K = 1024, M = 1024 * 1024}

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "yf")
if invalid then
  return invalid
end
local operands = {}
for _, arg in ipairs(args) do
  local key, value = arg:match("^(%a+)=(.*)$")
  if not key then
    return shell.usage(USAGE)
  end
  operands[key] = value
end
local function number(key, default)
  local digits, suffix = (operands[key] or ""):match("^(%d+)([KM]?)$")
  if operands[key] == nil then
    return default
  end
  return digits and tonumber(digits) * SIZES[suffix]
end
local bs, count, skip, seek = number("bs", 512), number("count"), number("skip", 0), number("seek", 0)
if not operands["if"] or not operands.of or not bs or bs == 0 or (operands.count and not count)
    or not skip or not seek then
  return shell.usage(USAGE)
end
for key in pairs(operands) do
  if not ({["if"] = 1, of = 1, bs = 1, count = 1, skip = 1, seek = 1})[key] then
    return shell.usage(USAGE)
  end
end

local function fail(path, reason)
  io.stderr:write("dd: ", path and path .. ": " or "", reason, "\n")
  return 1
end
local input, output = shell.resolve(operands["if"]), shell.resolve(operands.of)
local source, sourceReason = medium.find(input, true)
local target, targetReason = medium.find(output, true)
if input == output or (source and target and source.index == target.index) then
  return fail(output, "Input and output are the same file")
end
if not source and input:find("^/dev/vd") then
  return fail(input, sourceReason)
elseif not target and output:find("^/dev/vd") then
  return fail(output, targetReason)
end
if (source or target) and bs % 512 ~= 0 then
  return fail(source and input or output, "Block size must be a multiple of 512 for a medium")
end
if output == "/" or (target and target.path == "/") then
  return fail(output, "Is the boot medium")
end
if target then
  local ok, reason = medium.check(target, options.f)
  if not ok then
    return fail(output, reason)
  end
end

local reader, reason
if source then
  reader, reason = fs.openRaw(input, "r")
else
  reader, reason = io.open(input, "rb")
end
if not reader then
  return fail(source and input, reason)
end
local from <close> = reader
if skip > 0 then
  local ok, skipped = from:seek("set", skip * bs)
  if not ok then
    return fail(input, skipped)
  end
end
if target and not options.y and not medium.confirm(target) then
  print("nothing copied")
  return 1
end
local writer
if target then
  writer, reason = fs.openRaw(output, "w", {force = options.f, serial = target.serial})
elseif seek > 0 and fs.exists(output) then
  writer, reason = io.open(output, "r+b")
else
  writer, reason = io.open(output, "wb")
end
if not writer then
  return fail(target and output, reason)
end
local to <close> = writer
if seek > 0 then
  local ok, sought = to:seek("set", seek * bs)
  if not ok then
    return fail(output, sought)
  end
end

local copied, blocks, code, started = 0, 0, 0, computer.uptime()
while not count or blocks < count do
  local data, readReason = from:read(bs)
  if not data then
    if readReason then
      code = fail(input, readReason)
    end
    break
  end
  if target and #data % 512 ~= 0 then
    data = data .. string.rep("\0", 512 - #data % 512)
  end
  local ok, writeReason = to:write(data)
  if not ok then
    code = fail(output, writeReason)
    break
  end
  copied, blocks = copied + #data, blocks + 1
end
local seconds = computer.uptime() - started
print(string.format("%d bytes copied, %.3f s, %.0f kB/s", copied, seconds, seconds > 0 and copied / seconds / 1000 or 0))
return code
