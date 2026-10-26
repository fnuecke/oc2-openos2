local shell = require("shell")
local system = require("system")
local term = require("term")

local USAGE = [[
Usage: sh [script]
Runs the commands in the script, one per line, or reads commands from the
terminal.
]]

local script = ...
if script == "-h" or script == "--help" then
  io.write(USAGE)
  return 0
end
if script then
  local code = 0
  for line in io.lines(script) do
    code = shell.execute(line)
  end
  return code
end

local PROMPT_ESCAPES = {["$"] = "#", e = "\27", n = "\n", ["\\"] = "\\"}
local function prompt()
  local ps1 = os.getenv("PS1") or "\\w \\$ "
  return (ps1:gsub("\\(.)", function(c)
    return c == "w" and os.getenv("PWD") or PROMPT_ESCAPES[c] or "\\" .. c
  end):gsub("%$([%a_][%w_]*)", function(name)
    return os.getenv(name) or ""
  end))
end

local home = os.getenv("HOME") or "/"
local shrc = shell.resolve(home .. "/.shrc")
if fs.exists(shrc) then
  local ok, err = system.rawpcall(shell.execute, "source '" .. shrc:gsub("'", "'\\''") .. "'")
  if not ok and term.powerRequest(err) then
    error(err, 0)
  elseif not ok and not (type(err) == "table" and err.exit ~= nil) and term.systemError(err) ~= "interrupt" then
    io.stderr:write(tostring(err), "\n")
  end
end

local function complete(...)
  return require("completion").complete(...)
end

local history = {}
local depth, input, output, errors = shell.restore()
while true do
  local ok, err = system.rawpcall(function()
    shell.restore(depth, input, output, errors)
    local pwd = os.getenv("PWD")
    if not pwd or not fs.exists(pwd) then
      os.setenv("PWD", "/") -- e.g. the medium was removed
    end
    if term.column() > 0 then
      term.write("\n")
    end
    if package.loaded.jobs then
      package.loaded.jobs.report()
    end
    local line = term.read(prompt(), history, complete)
    if line == nil then
      os.exit(0)
    end
    shell.execute(line)
  end)
  if not ok then
    if type(err) == "table" and err.exit ~= nil then
      return err.exit
    elseif term.powerRequest(err) then
      error(err, 0) -- up to /init.lua
    elseif term.systemError(err) ~= "interrupt" then
      io.stderr:write(tostring(err), "\n")
    end
  end
end
