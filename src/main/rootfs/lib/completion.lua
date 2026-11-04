-- A command can complete its arguments itself with /lib/completion/<command>.lua, which returns
-- function(args, word) -> candidates, like bash's complete -F.

local medium = require("medium")
local shell = require("shell")
local term = require("term")

local completion = {}

local function startsWith(text, prefix)
  return text:sub(1, #prefix) == prefix
end

function completion.paths(word, directoriesOnly)
  local dir, prefix = word:match("^(.*/)([^/]*)$")
  dir, prefix = dir or "", prefix or word
  local base = shell.resolve(dir == "" and "." or dir)
  local candidates = {}
  local names, kinds = fs.list(base)
  for _, name in ipairs(names or {}) do
    if startsWith(name, prefix) and (prefix ~= "" or name:sub(1, 1) ~= ".") then
      if kinds[name] == "link" and (fs.stat(base .. "/" .. name) or {}).kind == "directory" then
        name = name .. "/"
      end
      if not directoriesOnly or name:sub(-1) == "/" then
        candidates[#candidates + 1] = dir .. name
      end
    end
  end
  return candidates
end

function completion.matches(word, names)
  local candidates = {}
  for _, name in ipairs(names) do
    if startsWith(name, word) then
      candidates[#candidates + 1] = name
    end
  end
  return candidates
end

function completion.operand(args, word)
  local position = 1
  for i = 2, #args - (word ~= "" and 1 or 0) do
    if args[i]:sub(1, 1) ~= "-" then
      position = position + 1
    end
  end
  return position
end

function completion.drives(word, include)
  local devices = {}
  for _, mount in ipairs(fs.mounts(true)) do
    if medium.device(mount) and (not include or include(mount)) then
      devices[#devices + 1] = medium.device(mount)
    end
  end
  return completion.matches(word, devices)
end

function completion.ports(word)
  local ports = {}
  for _, name in ipairs(fs.list("/dev") or {}) do
    if name:find("^ttyS%d+$") then
      ports[#ports + 1] = "/dev/" .. name
    end
  end
  return completion.matches(word, ports)
end

function completion.commands(word, programsOnly)
  if word:find("/") then
    return completion.paths(word)
  end
  local seen, candidates = {}, {}
  local function add(name)
    if startsWith(name, word) and not seen[name] then
      seen[name] = true
      candidates[#candidates + 1] = name
    end
  end
  if not programsOnly then
    for name in pairs(shell.builtins) do
      add(name)
    end
    for name in pairs(shell.aliases) do
      add(name)
    end
  end
  for dir in (os.getenv("PATH") or ""):gmatch("[^:]+") do
    for _, name in ipairs(fs.list(shell.resolve(dir)) or {}) do
      local program = name:match("^(.*)%.luac?$")
      if program then
        add(program)
      end
    end
  end
  return candidates
end

local function aliases(_, word)
  local names = {}
  for name in pairs(shell.aliases) do
    names[#names + 1] = name
  end
  return completion.matches(word, names)
end

local function jobs(_, word)
  return completion.matches(word, require("jobs").ids())
end

local completers = {
  alias = aliases,
  cd = function(_, word)
    return completion.paths(word, true)
  end,
  kill = jobs,
  unalias = aliases,
  wait = jobs,
}

local function completer(command)
  if completers[command] == nil then
    local file = "/lib/completion/" .. command .. ".lua"
    completers[command] = command:find("^[%w_-]+$") and fs.exists(file) and dofile(file) or false
  end
  return completers[command]
end

local function commonPrefix(words)
  local prefix = words[1]
  for i = 2, #words do
    while words[i]:sub(1, #prefix) ~= prefix do
      prefix = prefix:sub(1, -2)
    end
  end
  return prefix
end

local function listCandidates(candidates)
  local names, width = {}, 0
  for i, candidate in ipairs(candidates) do
    names[i] = candidate:match("([^/]*/?)$")
    width = math.max(width, #names[i] + 2)
  end
  table.sort(names)
  local columns = math.max(1, (term.width() - 1) // width)
  local rows = (#names + columns - 1) // columns
  term.write("\n")
  for row = 1, rows do
    local line = {}
    for col = 1, columns do
      local name = names[(col - 1) * rows + row]
      if name then
        line[#line + 1] = name .. string.rep(" ", width - #name)
      end
    end
    term.write((table.concat(line):gsub("%s+$", "")), "\n")
  end
end

local SPECIAL = "[%s'\"\\$#;&|<>]"

local function escape(text)
  return (text:gsub(SPECIAL, "\\%0"))
end

local function wordStart(before)
  local start = #before + 1
  while start > 1 do
    local c = before:sub(start - 1, start - 1)
    local escaped = start > 2 and before:sub(start - 2, start - 2) == "\\"
    if c:find("[%s;&|]") and not escaped then
      break
    end
    start = start - (escaped and 2 or 1)
  end
  return start
end

function completion.candidates(line, cursor)
  local before = line:sub(1, cursor)
  local command = before:match("[^;&|]*$")
  local start = wordStart(before)
  local word = line:sub(start, cursor):gsub("\\(.)", "%1")
  local args = {}
  for arg in command:gmatch("%S+") do
    args[#args + 1] = arg
  end
  if #args == 0 or (#args == 1 and word ~= "") then
    return completion.commands(word), start
  end
  local custom = completer(args[1])
  return custom and custom(args, word) or completion.paths(word), start
end

function completion.complete(line, cursor, again)
  local candidates, start = completion.candidates(line, cursor)
  local word = line:sub(start, cursor):gsub("\\(.)", "%1")
  local replacement = #candidates == 1 and candidates[1]
    or #candidates > 1 and commonPrefix(candidates)
  if replacement and #replacement > #word then
    local text = escape(replacement) .. (#candidates == 1 and replacement:sub(-1) ~= "/" and " " or "")
    return line:sub(1, start - 1) .. text .. line:sub(cursor + 1), start - 1 + #text
  elseif #candidates > 1 and again then
    listCandidates(candidates)
    return
  end
  return line, cursor
end

return completion
