local shell = require("shell")

local builtins = {}
local aliases = shell.aliases

function builtins.alias(...)
  if select("#", ...) == 0 then
    local names = {}
    for name in pairs(aliases) do
      names[#names + 1] = name
    end
    table.sort(names)
    for _, name in ipairs(names) do
      print("alias " .. name .. "='" .. aliases[name] .. "'")
    end
  end
  local code = 0
  for i = 1, select("#", ...) do
    local arg = select(i, ...)
    local name, value = arg:match("^([^=]+)=(.*)$")
    if name then
      aliases[name] = value
    elseif aliases[arg] then
      print("alias " .. arg .. "='" .. aliases[arg] .. "'")
    else
      io.stderr:write("sh: alias: ", arg, ": not found\n")
      code = 1
    end
  end
  return code
end

function builtins.unalias(...)
  local code = 0
  for i = 1, select("#", ...) do
    local name = select(i, ...)
    if aliases[name] then
      aliases[name] = nil
    else
      io.stderr:write("sh: unalias: ", name, ": not found\n")
      code = 1
    end
  end
  return code
end

function builtins.source(file)
  if not file then
    io.stderr:write("sh: source: filename argument required\n")
    return 2
  end
  local path = shell.resolve(file)
  local stat = fs.stat(path)
  if not stat or stat.isDir then
    io.stderr:write("sh: source: ", file, stat and ": Is a directory\n" or ": No such file or directory\n")
    return 1
  end
  local code = 0
  for line in io.lines(path) do
    code = shell.execute(line)
    if code == 130 then
      break -- Ctrl+C ends the script, like in bash
    end
  end
  return code
end
builtins["."] = builtins.source

for _, name in ipairs({"jobs", "wait", "kill"}) do
  builtins[name] = function(...)
    return require("jobs")[name](...)
  end
end

return builtins
