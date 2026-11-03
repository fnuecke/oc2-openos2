local shell = require("shell")
local medium = require("medium")

local USAGE = [[
Usage: dd if=<path> of=<path> [bs=n] [count=n] [skip=n] [seek=n]
Copies a file or a whole medium, e.g. /dev/vdb, to a file or a whole medium.
  bs=n     block size in bytes, default 512, with K and M suffixes
  count=n  number of blocks to copy
  skip=n   blocks to skip in the input
  seek=n   blocks to skip in the output
]]

local SIZES = {[""] = 1, K = 1024, M = 1024 * 1024}

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE)
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
  if not ({["if"] = 1, of = 1, bs = 1, count = 1, skip = 1, seek = 1, conv = 1})[key] then
    return shell.usage(USAGE)
  end
end
for flag in (operands.conv or ""):gmatch("[^,]+") do
  if flag ~= "fsync" and flag ~= "fdatasync" then -- each write reaches the medium before it returns
    return shell.usage(USAGE)
  end
end

local function fail(path, reason)
  io.stderr:write("dd: ", path and path .. ": " or "", reason, "\n")
  return 1
end
local input, output = shell.resolve(operands["if"]), shell.resolve(operands.of)
local source, sourceReason = medium.find(input)
local target, targetReason = medium.find(output)
if input == output then
  return fail(output, "Input and output are the same file")
end
if not source and input:find("^/dev/vd") then
  return fail(input, sourceReason)
elseif not target and output:find("^/dev/vd") then
  return fail(output, targetReason)
end

local reader, reason = io.open(input, "rb")
if not reader then
  return fail(nil, reason)
end
local from <close> = reader
if skip > 0 then
  local ok, skipped = from:seek("set", skip * bs)
  if not ok then
    return fail(input, skipped)
  end
end
local writer
if seek > 0 and not target and fs.exists(output) then
  writer, reason = io.open(output, "r+b")
else
  writer, reason = io.open(output, "wb")
end
if not writer then
  return fail(nil, reason)
end
local to <close> = writer
to:setvbuf("no")
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
