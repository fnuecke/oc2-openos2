-- Command line parsing and program execution. shell.parse is based on OpenOS's shell.

local system = require("system")
local term = require("term")

local shell = {}

local lastCode = 0
local redirected = {} -- files opened for the redirects of running commands, oldest first

-- Closes redirect files opened after `depth` of them were open, e.g. left open by Ctrl+C, and
-- restores io.input, io.output and io.stderr. Returns the depth and the streams to pass in later.
function shell.restore(depth, input, output, errors)
  if depth then
    for i = #redirected, depth + 1, -1 do
      pcall(redirected[i].close, redirected[i]) -- the program may have closed it
      redirected[i] = nil
    end
    io.input(input)
    io.output(output)
    rawset(io, "stderr", errors) -- nil is the console, see /init.lua
  end
  return #redirected, io.input(), io.output(), rawget(io, "stderr")
end

-- Returns the absolute, normalized form of path, relative to $PWD.
function shell.resolve(path)
  if path:sub(1, 1) ~= "/" then
    path = (os.getenv("PWD") or "/") .. "/" .. path
  end
  local parts = {}
  for part in path:gmatch("[^/]+") do
    if part == ".." then
      parts[#parts] = nil
    elseif part ~= "." then
      parts[#parts + 1] = part
    end
  end
  return "/" .. table.concat(parts, "/")
end

-- Splits program arguments into positional arguments and options. "-ab" sets options a and b,
-- "--name" sets name, "--name=value" sets name to value, "--" ends option parsing.
function shell.parse(...)
  local args, options = {}, {}
  local doneWithOptions = false
  for i = 1, select("#", ...) do
    local arg = select(i, ...)
    if doneWithOptions or type(arg) ~= "string" or arg == "-" or arg:sub(1, 1) ~= "-" then
      args[#args + 1] = arg
    elseif arg == "--" then
      doneWithOptions = true
    elseif arg:sub(1, 2) == "--" then
      local key, value = arg:match("^%-%-(.-)=(.*)$")
      if key then
        options[key] = value
      else
        options[arg:sub(3)] = true
      end
    else
      for j = 2, #arg do
        options[arg:sub(j, j)] = true
      end
    end
  end
  return args, options
end

-- For programs: checks the options against the allowed letters. Returns 0 after printing the usage
-- text for -h or --help, 1 after printing it to stderr for an unknown option, nil if all is well.
function shell.check(options, usage, allowed)
  if options.h or options.help then
    io.write(usage)
    return 0
  end
  for name in pairs(options) do
    if #name ~= 1 or not (allowed or ""):find(name, 1, true) then
      io.stderr:write("unknown option: ", #name == 1 and "-" or "--", name, "\n", usage)
      return 1
    end
  end
end

-- For programs: prints the usage text to stderr and returns 1, the exit code for wrong arguments.
function shell.usage(usage)
  io.stderr:write(usage)
  return 1
end

local function isFile(path)
  local stat = fs.stat(path)
  return stat ~= nil and not stat.isDir
end

-- Returns the path of the program for a command name: names containing "/" are paths, others are
-- looked up in $PATH, where only *.lua files count. The ".lua" extension is optional.
function shell.which(name)
  if name:find("/") then
    local path = shell.resolve(name)
    return isFile(path .. ".lua") and path .. ".lua" or isFile(path) and path or nil
  end
  for dir in (os.getenv("PATH") or ""):gmatch("[^:]+") do
    local path = shell.resolve(dir .. "/" .. name)
    if isFile(path .. ".lua") then
      return path .. ".lua"
    elseif path:find("%.lua$") and isFile(path) then
      return path -- not, e.g., oc2's shell scripts in /mnt/builtin/bin
    end
  end
end

-- Lexer: words separated by whitespace, '...' literal, "..." with $NAME, ${NAME} and \" \\ \$,
-- backslash escapes outside quotes, # starts a comment, and the redirections <, > and >>.
local function variable(line, i)
  local name = line:match("^{([%a_][%w_]*)}", i + 1)
  if name then
    return os.getenv(name) or "", i + #name + 3
  end
  if line:sub(i + 1, i + 1) == "?" then
    return tostring(lastCode), i + 2
  end
  name = line:match("^[%a_][%w_]*", i + 1)
  if not name then
    return "$", i + 1
  end
  return os.getenv(name) or "", i + #name + 1
end

local function lex(line)
  local words, word, quoted = {}, nil, false
  local function add(s)
    word = (word or "") .. s
  end
  local function finish()
    if word and (word ~= "" or quoted) then
      words[#words + 1] = word
    end
    word, quoted = nil, false
  end
  local i = 1
  while i <= #line do
    local c = line:sub(i, i)
    if c:find("^%s") then
      finish()
      i = i + 1
    elseif c == "#" and word == nil then
      break
    elseif c == "'" then
      local close = line:find("'", i + 1, true)
      if not close then
        return nil, "Syntax error: Unterminated quoted string"
      end
      add(line:sub(i + 1, close - 1))
      quoted, i = true, close + 1
    elseif c == '"' then
      quoted, i = true, i + 1
      add("")
      while true do
        c = line:sub(i, i)
        if c == "" then
          return nil, "Syntax error: Unterminated quoted string"
        elseif c == '"' then
          i = i + 1
          break
        elseif c == "\\" and line:find('^["\\$]', i + 1) then
          add(line:sub(i + 1, i + 1))
          i = i + 2
        elseif c == "$" then
          local value
          value, i = variable(line, i)
          add(value)
        else
          add(c)
          i = i + 1
        end
      end
    elseif c == "\\" then
      add(line:sub(i + 1, i + 1))
      quoted, i = true, i + 2
    elseif c == "$" then
      local value
      value, i = variable(line, i)
      add(value)
    elseif c == "<" or c == ">" or (c == "2" and word == nil and line:sub(i + 1, i + 1) == ">") then
      finish()
      local op = line:match("^2?>>", i) or line:match("^2?[<>]", i)
      words[#words + 1] = {op = op}
      i = i + #op
    else
      add(c)
      i = i + 1
    end
  end
  finish()
  return words
end

local function toCode(result)
  if result == nil or result == true then
    return 0
  elseif result == false then
    return 1
  end
  return math.tointeger(result) or 1
end

local function isExit(err)
  return type(err) == "table" and err.exit ~= nil
end

local function handler(err)
  if isExit(err) or term.isSystemError(err) then
    return err
  end
  local trace = debug.traceback(tostring(err), 2)
  return (trace:gsub("\n[^\n]*%[C%]: in function 'xpcall'.*$", ""))
end

local builtins = {}

function builtins.cd(dir)
  local path = shell.resolve(dir or os.getenv("HOME") or "/")
  local stat = fs.stat(path)
  if not stat or not stat.isDir then
    io.stderr:write("sh: cd: ", dir or path, stat and ": Not a directory\n" or ": No such file or directory\n")
    return 1
  end
  os.setenv("PWD", path)
end

function builtins.pwd()
  print(os.getenv("PWD"))
end

function builtins.exit(code)
  os.exit(tonumber(code) or 0)
end

function builtins.set(...)
  if select("#", ...) == 0 then
    local names = {}
    for name in pairs(os.getenv()) do
      names[#names + 1] = name
    end
    table.sort(names)
    for _, name in ipairs(names) do
      print(name .. "=" .. os.getenv(name))
    end
  end
  for i = 1, select("#", ...) do
    local name, value = (select(i, ...)):match("^([%a_][%w_]*)=(.*)$")
    if not name then
      io.stderr:write("sh: set: ", (select(i, ...)), ": not NAME=value\n")
      return 1
    end
    os.setenv(name, value)
  end
end

function builtins.unset(...)
  for i = 1, select("#", ...) do
    os.setenv((select(i, ...)), nil)
  end
end

-- Runs a program in its own global environment. Returns its exit code: the returned number, 0 for
-- nil or true, 1 for false or an error, 130 if interrupted.
function shell.run(path, ...)
  -- `arg` as lua.c sets it, which oc2's programs in /mnt/builtin/bin use.
  local env = setmetatable({arg = table.pack(...)}, {__index = _G})
  env.arg[0], env.arg.n = path, nil
  local program, reason = loadfile(path, "bt", env)
  if not program then
    io.stderr:write(reason, "\n")
    return 1
  end
  local ok, result = system.rawxpcall(program, handler, ...)
  if ok then
    return toCode(result)
  elseif isExit(result) then
    return toCode(result.exit)
  elseif term.powerRequest(result) then
    error(result, 0) -- up to /init.lua
  end
  io.stderr:write(result, "\n")
  return term.systemError(result) == "interrupt" and 130 or 1
end

local REDIRECT_MODES = {["<"] = "r", [">"] = "w", [">>"] = "a", ["2>"] = "w", ["2>>"] = "a"}

-- Runs a command line. Returns the exit code, also available as $?.
function shell.execute(line)
  local words, reason = lex(line)
  if not words then
    io.stderr:write("sh: ", reason, "\n")
    lastCode = 2
    return lastCode
  end
  local args, redirects = {}, {}
  for i, word in ipairs(words) do
    if type(word) == "table" then
      local target = words[i + 1]
      if type(target) ~= "string" then
        io.stderr:write("sh: syntax error near unexpected token 'newline'\n")
        lastCode = 2
        return lastCode
      end
      redirects[#redirects + 1] = {mode = REDIRECT_MODES[word.op], path = target, errors = word.op:sub(1, 1) == "2"}
    elseif type(words[i - 1]) ~= "table" then
      args[#args + 1] = word
    end
  end
  if #args == 0 then
    return lastCode
  end

  local depth, oldInput, oldOutput, oldErrors = shell.restore()
  for _, redirect in ipairs(redirects) do
    local file, openReason = io.open(redirect.path, redirect.mode)
    if not file then
      shell.restore(depth, oldInput, oldOutput, oldErrors)
      io.stderr:write("sh: ", openReason, "\n")
      lastCode = 1
      return lastCode
    end
    redirected[#redirected + 1] = file
    if redirect.errors then
      rawset(io, "stderr", file)
    elseif redirect.mode == "r" then
      io.input(file)
    else
      io.output(file)
    end
  end

  local builtin, path = builtins[args[1]], nil
  if not builtin then
    path = shell.which(args[1])
    if not path then
      io.stderr:write("sh: ", args[1], ": command not found\n")
      shell.restore(depth, oldInput, oldOutput, oldErrors)
      lastCode = 127
      return lastCode
    end
  end

  local ok, result
  if builtin then
    ok, result = system.rawpcall(builtin, table.unpack(args, 2))
    result = ok and toCode(result) or result
  else
    ok, result = system.rawpcall(shell.run, path, table.unpack(args, 2))
  end
  shell.restore(depth, oldInput, oldOutput, oldErrors)
  if not ok then
    error(result, 0)
  end
  lastCode = result
  return lastCode
end

return shell
