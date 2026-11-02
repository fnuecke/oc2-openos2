local shell = require("shell")

local USAGE = [[
Usage: ln [-s] <target> <link>
Creates a hard link to target, in link if that is a directory.
  -s  make a symbolic link instead
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "s")
if invalid then
  return invalid
end
if #args ~= 2 then
  return shell.usage(USAGE)
end
local target, link = args[1], shell.resolve(args[2])
local stat = fs.stat(link)
if stat and stat.kind == "directory" then
  link = link .. "/" .. (require("path").name(target) or "")
end
if options.s then
  local ok, reason = fs.symlink(target, link)
  if not ok then
    io.stderr:write("ln: failed to create symbolic link '", args[2], "': ", reason, "\n")
    return 1
  end
else
  local ok, reason = fs.link(shell.resolve(target), link)
  if not ok then
    io.stderr:write("ln: failed to create hard link '", args[2], "' => '", target, "': ", reason, "\n")
    return 1
  end
end
return 0
