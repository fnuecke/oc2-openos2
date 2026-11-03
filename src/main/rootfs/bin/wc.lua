local shell = require("shell")

local USAGE = [[
Usage: wc [-l] [-w] [-c] [file...]
Prints the number of lines, words and bytes of the files, or of the input.
  -l  lines
  -w  words
  -c  bytes
]]

local args, options = shell.parse(...)
local invalid = shell.check(options, USAGE, "lwc")
if invalid then
  return invalid
end
local columns, wanted = {}, {}
for _, name in ipairs({"l", "w", "c"}) do
  if options[name] or not (options.l or options.w or options.c) then
    columns[#columns + 1], wanted[name] = name, true
  end
end
local stdin = #args == 0
if stdin then
  args[1] = "-"
end

local rows, total, code = {}, {l = 0, w = 0, c = 0}, 0
for _, path in ipairs(args) do
  local file, reason = io.stdin, nil
  if path ~= "-" then
    file, reason = io.open(path)
  end
  if not file then
    io.stderr:write("wc: ", reason, "\n")
    code = 1
  else
    local counts, inWord = {l = 0, w = 0, c = 0}, false
    while true do
      local chunk, readReason = file:read(path == "-" and "L" or 4096)
      if not chunk then
        if readReason then
          io.stderr:write("wc: ", path, ": ", readReason, "\n")
          code = 1
        end
        break
      end
      if wanted.l then
        local i = chunk:find("\n", 1, true)
        while i do
          counts.l = counts.l + 1
          i = chunk:find("\n", i + 1, true)
        end
      end
      if wanted.w then
        local _, words = chunk:gsub("%S+", "")
        counts.w = counts.w + words - (inWord and chunk:find("^%S") and 1 or 0)
        inWord = chunk:sub(-1):find("%S") ~= nil
      end
      counts.c = counts.c + #chunk
    end
    if path ~= "-" then
      file:close()
    end
    for name, count in pairs(counts) do
      total[name] = total[name] + count
    end
    rows[#rows + 1] = {counts, not stdin and path or nil}
  end
end
if #args > 1 then
  rows[#rows + 1] = {total, "total"}
end

local width = 1
if stdin and #columns > 1 then
  width = 7
elseif #columns > 1 or #rows > 1 then
  width = #tostring(math.max(total.c, total.l, total.w))
end
for _, row in ipairs(rows) do
  local line = {}
  for _, name in ipairs(columns) do
    line[#line + 1] = string.format("%" .. width .. "d", row[1][name])
  end
  line[#line + 1] = row[2]
  print(table.concat(line, " "))
end
return code
