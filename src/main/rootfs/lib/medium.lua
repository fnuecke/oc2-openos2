local medium = {}

local KINDS = {
  blank = "blank",
  unsupported = "a file system OpenOS II cannot use",
  unknown = "unknown data",
}

-- The drive of a mount, e.g. /dev/vdb, or nil if it is not on a drive.
function medium.device(mount)
  if KINDS[mount.medium] or mount.medium == "ext2" then
    return "/dev/vd" .. string.char(string.byte("a") + mount.index)
  end
end

function medium.isUsable(mount)
  return not KINDS[mount.medium]
end

function medium.find(path)
  for _, mount in ipairs(fs.mounts(true)) do
    if medium.device(mount) == path then
      return mount
    end
  end
  return nil, select(2, fs.label(path)) or "Medium changed"
end

local function contents(mount)
  if mount.medium == "ext2" then
    return string.format("ext2%s, %d KiB used", mount.label and " \"" .. mount.label .. "\"" or "",
      (mount.size - mount.free) // 1024)
  end
  return KINDS[mount.medium]
end

function medium.confirm(mount)
  local device = medium.device(mount)
  print(string.format("%s: %d KiB, %s, %s", device, mount.size // 1024, contents(mount),
    mount.unmounted and "not mounted" or "mounted at " .. mount.path))
  io.write("all data on ", device, " will be lost, continue? [y/N] ")
  local answer = io.read()
  return answer ~= nil and answer:match("^[yY]") ~= nil
end

function medium.mountPoints()
  local points = {}
  for _, mount in ipairs(fs.mounts()) do
    points[mount.path] = true
  end
  return points
end

return medium
