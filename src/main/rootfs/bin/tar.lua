local shell = require("shell")

local USAGE = [[
Usage: tar -c|-x|-t [-v] [-f <archive>] [path...]
Creates, extracts or lists archives in the ustar format, without compression.
  -c          create an archive of the paths
  -x          extract the files, or only the paths
  -t          list the files, or only the paths
  -v          print the name of each file
  -f archive  the archive, default the input or the output
]]

local argv = {...}
if argv[1] and argv[1]:find("^[^-]") then
  argv[1] = "-" .. argv[1] -- tar xvf
end
local raw, archive = {}, "-"
local i = 1
while i <= #argv do
  local before, after = argv[i]:match("^%-(%a*)f(.*)$")
  if before then
    if after == "" then
      i = i + 1
      after = argv[i]
      if not after then
        return shell.usage(USAGE)
      end
    end
    archive = after
    raw[#raw + 1] = before ~= "" and "-" .. before or nil
  else
    raw[#raw + 1] = argv[i]
  end
  i = i + 1
end
local args, options = shell.parse(table.unpack(raw))
local invalid = shell.check(options, USAGE, "cxtv")
if invalid then
  return invalid
end
if (options.c and 1 or 0) + (options.x and 1 or 0) + (options.t and 1 or 0) ~= 1 or options.c and #args == 0 then
  return shell.usage(USAGE)
end

local BLOCK = 512
local HEADER = "c100c8c8c8c12c12c8c1c100c6c2c32c32c8c8c155c12"
local code = 0
local log = archive == "-" and options.c and io.stderr or io.stdout

local function fail(...)
  io.stderr:write("tar: ", ...)
  io.stderr:write("\n")
  code = 2
end

local function checksum(block)
  local sum = 8 * 32
  for c in (block:sub(1, 148) .. block:sub(157)):gmatch("[^\0]") do
    sum = sum + c:byte()
  end
  return sum
end

local function octal(value, size)
  return string.format("%0" .. (size - 1) .. "o", value) .. "\0"
end

local function header(name, kind, size, mtime)
  local prefix = ""
  if #name > 100 then
    local cut = name:sub(1, 156):match(".*()/")
    if not cut or #name - cut > 100 then
      return nil
    end
    prefix, name = name:sub(1, cut - 1), name:sub(cut + 1)
  end
  local block = string.pack(HEADER, name, octal(kind == "5" and 493 or 420, 8), octal(0, 8), octal(0, 8),
    octal(size, 12), octal(mtime, 12), "", kind, "", "ustar\0", "00", "", "", "", "", prefix, "")
  return block:sub(1, 148) .. string.format("%06o\0 ", checksum(block)) .. block:sub(157)
end

local function create(output)
  local self = archive ~= "-" and shell.resolve(archive)
  local function add(name, path)
    local stat, reason = fs.lstat(path)
    if not stat then
      return fail(name, ": ", reason)
    elseif path == self then
      return io.stderr:write("tar: ", name, ": file is the archive; not dumped\n")
    elseif stat.kind ~= "file" and stat.kind ~= "directory" then
      return fail(name, ": not a file or directory, skipped")
    end
    local directory = stat.kind == "directory"
    local entry = name:gsub("^[/.]*/", "") .. (directory and not name:find("/$") and "/" or "")
    local block = entry ~= "" and header(entry, directory and "5" or "0", directory and 0 or stat.size, stat.mtime)
    local file, openReason
    if not directory then
      file, openReason = io.open(path, "rb")
    end
    if entry ~= "" and not block then
      return fail(name, ": name too long")
    elseif not directory and not file then
      return fail(name, ": ", (openReason:gsub("^.-: ", "")))
    elseif block then
      output:write(block)
      if options.v then
        log:write(entry, "\n")
      end
    end
    if directory then
      local names = fs.list(path)
      table.sort(names)
      for _, child in ipairs(names) do
        child = child:gsub("/$", "")
        add(name:gsub("/$", "") .. "/" .. child, path:gsub("/$", "") .. "/" .. child)
      end
      return
    end
    local left = stat.size
    while left > 0 do
      local data = file:read(math.min(left, 4096)) or ""
      if data == "" then
        fail(name, ": file shrank, padded with zeros")
        data = string.rep("\0", left)
      end
      output:write(data)
      left = left - #data
    end
    file:close()
    output:write(string.rep("\0", -stat.size % BLOCK))
  end
  for _, arg in ipairs(args) do
    add(arg, shell.resolve(arg))
  end
  output:write(string.rep("\0", 2 * BLOCK))
end

local function selected(name)
  if #args == 0 then
    return true
  end
  for _, arg in ipairs(args) do
    arg = arg:gsub("/$", "")
    if name == arg or name:sub(1, #arg + 1) == arg .. "/" then
      return true
    end
  end
  return false
end

local function makeDirectories(name)
  local current = ""
  for part in name:gmatch("[^/]+") do
    current = current .. part
    if not fs.exists(current) then
      local ok, reason = fs.mkdir(current)
      if not ok then
        return nil, reason
      end
    end
    current = current .. "/"
  end
  return true
end

local function open(name, kind, size, mtime)
  name = name:gsub("^/+", "")
  if not selected((name:gsub("/$", ""))) then
    return
  elseif options.t and options.v then
    log:write(string.format("%s %8d %s %s\n", kind == "5" and "d" or "-", size, os.date("%Y-%m-%d %H:%M", mtime), name))
    return
  elseif options.t or options.v then
    log:write(name, "\n")
  end
  if options.t then
    return
  end
  for part in name:gmatch("[^/]+") do
    if part == ".." then
      return fail(name, ": contains '..', skipped")
    end
  end
  local ok, reason = makeDirectories(kind == "5" and name or name:match("^(.*)/") or "")
  if ok and kind ~= "5" then
    local file, openReason = io.open(name, "wb")
    if file then
      return file
    end
    ok, reason = false, (openReason:gsub("^.-: ", ""))
  end
  if not ok then
    fail(name, ": ", reason)
  end
end

local function extract(input)
  local longName
  while true do
    local block = input:read(BLOCK)
    if not block or not block:find("[^\0]") then
      return
    elseif #block < BLOCK then
      return fail("unexpected end of archive")
    end
    local fields = table.pack(string.unpack(HEADER, block))
    local function text(index)
      return fields[index]:match("^[^\0]*")
    end
    local function number(index)
      return tonumber(fields[index]:match("[0-7]+") or "0", 8)
    end
    if number(7) ~= checksum(block) then
      return fail("this does not look like a tar archive")
    end
    local kind, size = text(8), number(5)
    local name = longName or (fields[10] == "ustar\0" and text(16) ~= "" and text(16) .. "/" or "") .. text(1)
    local output, data = nil, {}
    longName = nil
    if kind == "0" or kind == "" or kind == "5" then
      output = open(name, kind, size, number(6))
    elseif kind ~= "L" and kind ~= "x" and kind ~= "g" then
      fail(name, ": unsupported entry type '", kind, "', skipped")
    end
    local left = size
    while left > 0 do
      local chunk = input:read(math.min(left, 4096))
      if not chunk then
        if output then
          output:close()
        end
        return fail("unexpected end of archive")
      end
      if output then
        output:write(chunk)
      elseif kind == "L" or kind == "x" then
        data[#data + 1] = chunk
      end
      left = left - #chunk
    end
    if output then
      output:close()
    end
    if kind == "L" then
      longName = table.concat(data):match("^[^\0]*")
    elseif kind == "x" then
      longName = ("\n" .. table.concat(data)):match("\n%d+ path=([^\n]*)")
    end
    input:read(-size % BLOCK)
  end
end

local file, reason = archive == "-" and (options.c and io.output() or io.input()), nil
if not file then
  file, reason = io.open(archive, options.c and "wb" or "rb")
end
if not file then
  io.stderr:write("tar: ", reason, "\n")
  return 2
elseif require("term").isTerminal(file) then
  io.stderr:write("tar: refusing to ", options.c and "write the archive to" or "read the archive from",
    " the terminal\n")
  return 2
end
if options.c then
  create(file)
else
  extract(file)
end
if archive ~= "-" then
  file:close()
end
return code
