-- For programs that overwrite whole media, format and dd: finds the drive mounted at a path,
-- describes it, and asks before its data is lost.

local medium = {}

local KINDS = {
  blank = "blank",
  unsupported = "a file system OpenOS II cannot use",
  unknown = "unknown data",
}

-- The fs.mounts() entry of the drive whose mount point path is, or nil and why not.
function medium.find(path, reading)
  if path == "/" and not reading then
    return nil, "Is the boot medium"
  end
  for _, mount in ipairs(fs.mounts()) do
    if mount.path == path and mount.index then
      return mount
    end
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

-- Returns true if the drive may be overwritten, else nil and why not.
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

-- Describes the drive and asks. Returns whether to go on.
function medium.confirm(mount)
  print(string.format("%s: drive %d, %d KiB, %s", mount.path, mount.index, mount.size // 1024, contents(mount)))
  io.write("all data on ", mount.path, " will be lost, continue? [y/N] ")
  local answer = io.read()
  return answer ~= nil and answer:match("^[yY]") ~= nil
end

return medium
