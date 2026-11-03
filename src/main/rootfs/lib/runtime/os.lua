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

function os.exit(code)
  error({exit = code == nil or code}, 0)
end

os.sleep = computer.sleep

function os.execute(command)
  if command == nil then
    return true
  end
  local code = require("shell").subshell(command)
  return code == 0 or nil, "exit", code
end
