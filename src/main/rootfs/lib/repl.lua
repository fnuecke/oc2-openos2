local system = require("system")
local term = require("term")

local repl = {}

local function compile(source, env)
  local fn, err = load("return " .. source, "=stdin", "t", env)
  if fn then
    return fn
  end
  return load(source, "=stdin", "t", env)
end

local function step(history, env)
  local source = term.read("lua> ", history)
  if source == nil then
    return false
  end
  local fn, err = compile(source, env)
  while not fn and err:find("<eof>$") do
    local more = term.read(">>   ", history)
    if more == nil then
      return true
    end
    source = source .. "\n" .. more
    fn, err = compile(source, env)
  end
  if not fn then
    print(err)
    return true
  end
  local result = table.pack(system.rawxpcall(fn, function(e)
    if term.isSystemError(e) or type(e) == "table" and e.exit ~= nil then
      return e
    end
    return debug.traceback(e, 2)
  end))
  if not result[1] then
    if term.powerRequest(result[2]) or type(result[2]) == "table" and result[2].exit ~= nil then
      error(result[2], 0)
    end
    print(result[2])
  elseif result.n > 1 then
    print(table.unpack(result, 2, result.n))
  end
  return true
end

function repl.run(env)
  local history = {}
  while true do
    local ok, result = system.rawpcall(step, history, env or _G)
    if not ok then
      if term.powerRequest(result) or type(result) == "table" and result.exit ~= nil then
        error(result, 0)
      end
      print(tostring(result))
    elseif not result then
      return
    end
  end
end

return repl
