-- We wrap these to allow passing through system level errors (e.g. ctrl+c interrupt, power off).
local system = {rawpcall = pcall, rawxpcall = xpcall, rawresume = coroutine.resume, BLOCKED = {}}
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

local BLOCKED = system.BLOCKED

local function relay(co, ok, ...)
  if ok and (...) == BLOCKED and coroutine.isyieldable() and coroutine.status(co) == "suspended" then
    return relay(co, system.rawresume(co, coroutine.yield(...)))
  end
  return ok, ...
end

function coroutine.resume(co, ...)
  return passSystemErrors(relay(co, system.rawresume(co, ...)))
end

local function unwrap(co, ok, ...)
  if ok then
    return ...
  end
  local err = ...
  if coroutine.status(co) == "dead" then
    local closed, closeErr = coroutine.close(co)
    if not closed then
      err = closeErr
    end
  end
  error(err, type(err) == "string" and not term.isSystemError(err) and 2 or 0)
end

function coroutine.wrap(f)
  local co = coroutine.create(f)
  return function(...)
    return unwrap(co, relay(co, system.rawresume(co, ...)))
  end
end
