local shell = require("shell")

local USAGE = [[
Usage: crontab [file | -l | -e | -r]
Manages the crontab, $HOME/.crontab.
  file  install file as the crontab
  -l    print the crontab
  -e    edit the crontab
  -r    remove the crontab
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "ler")
if invalid then
  return invalid
end
local cron = require("cron")
local path = cron.path()

local function check(text)
  local _, bad = cron.parse(text)
  if #bad > 0 then
    io.stderr:write("crontab: line ", bad[1], ": bad entry\n")
    return false
  end
  return true
end

local function install(text)
  local out = assert(io.open(path, "w"))
  out:write(text)
  out:close()
  cron.start()
end

local function read(file)
  local handle = io.open(file)
  if not handle then
    return nil
  end
  local text = handle:read("a")
  handle:close()
  return text
end

if options.l then
  local file = io.open(path)
  if not file then
    io.stderr:write("no crontab for root\n")
    return 1
  end
  io.write(file:read("a"))
  file:close()
elseif options.r then
  fs.remove(path)
elseif options.e then
  local temp = path .. ".edit"
  local out = assert(io.open(temp, "w"))
  out:write(read(path) or "")
  out:close()
  local code = shell.execute("edit '" .. temp .. "'")
  local text = read(temp)
  fs.remove(temp)
  if code ~= 0 or not text then
    return code ~= 0 and code or 1
  end
  if not check(text) then
    io.stderr:write("crontab: not installed\n")
    return 1
  end
  install(text)
elseif #args == 1 then
  local file, reason = io.open(shell.resolve(args[1]))
  if not file then
    io.stderr:write("crontab: ", reason, "\n")
    return 1
  end
  local text = file:read("a")
  file:close()
  if not check(text) then
    return 1
  end
  install(text)
else
  return shell.usage(USAGE)
end
