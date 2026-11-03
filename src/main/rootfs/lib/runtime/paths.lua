local paths = {}

function paths.absolute(path)
  if type(path) == "string" and path:sub(1, 1) ~= "/" then
    return (os.getenv("PWD") or "/"):gsub("/$", "") .. "/" .. path
  end
  return path
end

function paths.isDevice(path)
  return type(path) == "string" and path:find("/dev", 1, true)
    and (" stdin stdout stderr tty console dev . .. "):find(" " .. path:match("([^/]*)/*$") .. " ", 1, true)
end

local absolute, isDevice = paths.absolute, paths.isDevice

for _, name in ipairs({"list", "stat", "lstat", "readlink", "realpath", "exists", "mkdir", "remove", "info",
                       "format", "label", "mount", "unmount"}) do
  local f = fs[name]
  fs[name] = function(path, ...)
    return f(absolute(path), ...)
  end
end
local rename, link = fs.rename, fs.link
function fs.rename(from, to)
  return rename(absolute(from), absolute(to))
end
function fs.link(from, to)
  return link(absolute(from), absolute(to))
end
local symlink = fs.symlink
function fs.symlink(target, path)
  return symlink(target, absolute(path))
end
os.remove, os.rename = fs.remove, fs.rename

for _, name in ipairs({"list", "stat", "lstat", "exists"}) do
  local f = fs[name]
  fs[name] = function(path)
    if isDevice(absolute(path)) then
      return require("streams")[name](f, absolute(path))
    end
    return f(path)
  end
end

local kernelLoadfile = loadfile
function loadfile(path, ...)
  return kernelLoadfile(absolute(path), ...)
end

function dofile(path)
  return assert(loadfile(path))()
end

package.searchers[2] = function(name)
  local file, reason = package.searchpath(name, (package.path:gsub("[^;]+", absolute)))
  if not file then
    return reason
  end
  local chunk, err = loadfile(file)
  if not chunk then
    error(("error loading module '%s' from file '%s':\n\t%s"):format(name, file, err), 0)
  end
  return chunk, file
end

return paths
