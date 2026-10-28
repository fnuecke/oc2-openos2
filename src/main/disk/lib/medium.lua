local medium = {}

local KINDS = {
  blank = "blank",
  unsupported = "a file system OpenOS II cannot use",
  unknown = "unknown data",
}

function medium.device(index)
  return "/dev/vd" .. string.char(string.byte("a") + index)
end

function medium.find(path, reading)
  if path == "/" and not reading then
    return nil, "Is the boot medium"
  end
  local device = path:match("^/dev/vd%l$")
  for _, mount in ipairs(fs.mounts(true)) do
    if mount.index and (mount.path == path and not mount.unmounted or device and medium.device(mount.index) == path) then
      if mount.path == "/" and not reading then
        return nil, "Is the boot medium"
      end
      return mount
    end
  end
  if device then
    return nil, select(2, fs.info(path))
  end
  return nil, "Not the mount point of a drive"
end

local function contents(mount)
  if mount.medium == "ext2" then
    return string.format("ext2%s, %d KiB used", mount.label and " \"" .. mount.label .. "\"" or "",
      (mount.size - mount.free) // 1024)
  end
  return KINDS[mount.medium]
end

function medium.check(mount, force)
  if mount.medium == "ext2" and not force then
    return nil, string.format("Holds an ext2 file system with %d KiB used, use -f", (mount.size - mount.free) // 1024)
  elseif mount.medium ~= "blank" and not force then
    return nil, "Holds " .. KINDS[mount.medium] .. ", use -f"
  elseif mount.readOnly and mount.medium ~= "ext2" then
    return nil, "Read-only file system"
  end
  return true
end

function medium.confirm(mount)
  local device = medium.device(mount.index)
  print(string.format("%s: %d KiB, %s, %s", device, mount.size // 1024, contents(mount),
    mount.unmounted and "not mounted" or "mounted at " .. mount.path))
  io.write("all data on ", device, " will be lost, continue? [y/N] ")
  local answer = io.read()
  return answer ~= nil and answer:match("^[yY]") ~= nil
end

function medium.mountPoints()
  local points = {["/dev"] = true, ["/sys"] = true}
  for _, mount in ipairs(fs.mounts()) do
    points[mount.path] = true
  end
  return points
end

return medium
