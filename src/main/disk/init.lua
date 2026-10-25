-- Errors the kernel raises in running code, Ctrl+C, the watchdog and the host's power button, pass
-- through pcall, xpcall and coroutine.resume, so a program cannot keep the system from escalating
-- them. System code that needs to handle them can access raw functions from require("system").
local system = {rawpcall = pcall, rawxpcall = xpcall, rawresume = coroutine.resume}
package.loaded.system = system

local term = require("term")

local function passSystemErrors(ok, ...)
  if not ok and term.isSystemError((...)) then
    error((...), 0)
  end
  return ok, ...
end

function pcall(f, ...)
  return passSystemErrors(system.rawpcall(f, ...))
end

function xpcall(f, handler, ...)
  return passSystemErrors(system.rawxpcall(f, function(err)
    if term.isSystemError(err) then
      return err
    end
    return handler(err)
  end, ...))
end

function coroutine.resume(co, ...)
  return passSystemErrors(system.rawresume(co, ...))
end

local vars = {PATH = "/bin:/mnt/builtin/bin:.", PWD = "/", HOME = "/"}
local kernelGetenv = os.getenv

function os.getenv(name)
  if name == nil then
    return vars
  end
  return vars[name] or kernelGetenv(name)
end

function os.setenv(name, value)
  vars[name] = value ~= nil and tostring(value) or nil
end

-- Ends the running program, see /lib/shell.lua.
function os.exit(code)
  error({exit = code == nil or code}, 0)
end

os.sleep = computer.sleep

-- Runs the command with /lib/shell.lua, with Lua's results: true or nil, "exit", the exit code.
function os.execute(command)
  if command == nil then
    return true
  end
  local code = require("shell").execute(command)
  return code == 0 or nil, "exit", code
end

-- Relative paths start at $PWD. The kernel resolves "." and "..".
local function absolute(path)
  if type(path) == "string" and path:sub(1, 1) ~= "/" then
    return (vars.PWD or "/"):gsub("/$", "") .. "/" .. path
  end
  return path
end

for _, name in ipairs({"list", "stat", "exists", "mkdir", "remove", "info", "format", "label", "openRaw"}) do
  local f = fs[name]
  fs[name] = function(path, ...)
    return f(absolute(path), ...)
  end
end
local rename = fs.rename
function fs.rename(from, to)
  return rename(absolute(from), absolute(to))
end
os.remove, os.rename = fs.remove, fs.rename

local kernelLoadfile = loadfile
function loadfile(path, ...)
  return kernelLoadfile(absolute(path), ...)
end

function dofile(path)
  return assert(loadfile(path))()
end

-- io: stdin, stdout and stderr are the console. io.input and io.output can redirect them.
local fileMethods = getmetatable(io.stdout).__index
function fileMethods:flush()
  return self
end
function fileMethods:setvbuf()
  return true
end

local kernelOpen, kernelLines, kernelType = io.open, io.lines, io.type
local input, output = term.stream, term.stream
io.stdin, io.stdout = nil, nil
setmetatable(io, {__index = function(_, key)
  if key == "stdin" then
    return input
  elseif key == "stdout" then
    return output
  elseif key == "stderr" then
    return term.stream
  end
end})

function io.open(path, mode)
  return kernelOpen(absolute(path), mode)
end

local function openIfPath(file, mode)
  if type(file) == "string" then
    file = assert(io.open(file, mode))
  end
  return file
end

function io.input(file)
  input = file and openIfPath(file, "r") or input
  return input
end

function io.output(file)
  output = file and openIfPath(file, "w") or output
  return output
end

function io.lines(path, ...)
  if path == nil then
    return input:lines(...)
  end
  return kernelLines(absolute(path), ...)
end

function io.read(...)
  return input:read(...)
end

function io.write(...)
  return output:write(...)
end

function io.flush()
  return output:flush()
end

function io.close(file)
  return (file or output):close()
end

function io.type(file)
  if file == term.stream then
    return "file"
  end
  return kernelType(file)
end

function print(...)
  for i = 1, select("#", ...) do
    if i > 1 then
      output:write("\t")
    end
    output:write(tostring((select(i, ...))))
  end
  output:write("\n")
end

print(string.format("%s on OpenOS II, %d KiB free, booted in %.2f s",
  _VERSION, computer.freeMemory() // 1024, computer.uptime()))

local function handleError(err)
  local request = term.powerRequest(err)
  if request then
    if request == "reboot" then
      computer.reboot()
    end
    computer.shutdown()
  end
end

local shell = require("shell")
while true do
  local ok, code = system.rawpcall(shell.execute, "sh")
  if ok and code == 127 then
    ok, code = system.rawpcall(require("repl").run)
  end
  if not ok then
    handleError(code)
  end
end
