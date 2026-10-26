local system = require("system")
local term = require("term")

local shell = {}

-- $?, kept with the streams, so pipeline stages and threads do not change the shell's.
local function lastCode()
  return system.streams().lastCode or 0
end

local function setLastCode(code)
  system.streams().lastCode = code
  return code
end

function shell.restore(depth, input, output, errors)
  local streams = system.streams()
  streams.files = streams.files or {}
  local files = streams.files
  if depth then
    for i = #files, depth + 1, -1 do
      pcall(files[i].close, files[i]) -- the program may have closed it
      files[i] = nil
    end
    io.input(input)
    io.output(output)
    streams.errors = errors
  end
  return #files, io.input(), io.output(), streams.errors
end

shell.resolve = require("path").resolve

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
-- text for --help, or -h unless it is an allowed option, 1 after printing it to stderr for an unknown
-- option, nil if all is well.
function shell.check(options, usage, allowed)
  if options.help or (options.h and not (allowed or ""):find("h", 1, true)) then
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

function shell.usage(usage)
  io.stderr:write(usage)
  return 1
end

local function isFile(path)
  local stat = fs.stat(path)
  return stat ~= nil and not stat.isDir
end

local function withExtension(path)
  for _, extension in ipairs({".luac", ".lua"}) do
    if isFile(path .. extension) then
      return path .. extension
    end
  end
end

function shell.which(name)
  if name:find("/") then
    local path = shell.resolve(name)
    return withExtension(path) or isFile(path) and path or nil
  end
  for dir in (os.getenv("PATH") or ""):gmatch("[^:]+") do
    local path = shell.resolve(dir .. "/" .. name)
    local program = withExtension(path)
    if program then
      return program
    elseif path:find("%.luac?$") and isFile(path) then
      return path -- not, e.g., oc2's shell scripts in /mnt/builtin/bin
    end
  end
end

local function variable(line, i)
  local name = line:match("^{([%a_][%w_]*)}", i + 1)
  if name then
    return os.getenv(name) or "", i + #name + 3
  end
  if line:sub(i + 1, i + 1) == "?" then
    return tostring(lastCode()), i + 2
  elseif line:sub(i + 1, i + 1) == "!" then
    return shell.lastJob or "", i + 2
  end
  name = line:match("^[%a_][%w_]*", i + 1)
  if not name then
    return "$", i + 1
  end
  return os.getenv(name) or "", i + #name + 1
end

local function lex(line)
  local words, globs, word, pattern, quoted, wildcard = {}, {}, nil, "", false, false
  local function add(s, literal)
    word = (word or "") .. s
    pattern = pattern .. (literal and s:gsub("[*?[\\]", "\\%0") or s)
    wildcard = wildcard or not literal and s:find("[*?[]") ~= nil
  end
  local function finish()
    if word and (word ~= "" or quoted) then
      words[#words + 1] = word
      globs[#words] = wildcard and pattern or nil
    end
    word, pattern, quoted, wildcard = nil, "", false, false
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
      add(line:sub(i + 1, close - 1), true)
      quoted, i = true, close + 1
    elseif c == '"' then
      quoted, i = true, i + 1
      add("", true)
      while true do
        c = line:sub(i, i)
        if c == "" then
          return nil, "Syntax error: Unterminated quoted string"
        elseif c == '"' then
          i = i + 1
          break
        elseif c == "\\" and line:find('^["\\$]', i + 1) then
          add(line:sub(i + 1, i + 1), true)
          i = i + 2
        elseif c == "$" then
          local value
          value, i = variable(line, i)
          add(value, true)
        else
          add(c, true)
          i = i + 1
        end
      end
    elseif c == "\\" then
      add(line:sub(i + 1, i + 1), true)
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
  return words, globs
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
local aliases = {}
shell.builtins, shell.aliases = builtins, aliases

function builtins.cd(dir)
  local path = shell.resolve(dir or os.getenv("HOME") or "/")
  local stat = fs.stat(path)
  if not stat or not stat.isDir then
    io.stderr:write("sh: cd: ", dir or path, stat and ": Not a directory\n" or ": No such file or directory\n")
    return 1
  end
  os.setenv("PWD", path)
end

builtins["true"] = function()
  return 0
end

builtins["false"] = function()
  return 1
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

for _, name in ipairs({"alias", "unalias", "source", ".", "jobs", "wait", "kill"}) do
  builtins[name] = function(...)
    return require("builtins")[name](...)
  end
end

function builtins.unset(...)
  for i = 1, select("#", ...) do
    os.setenv((select(i, ...)), nil)
  end
end

-- Programs that run, by pid: {pid, parent, command, streams}. The streams are the record the
-- program runs with, see system.streams in /init.lua.
shell.processes = {}
local nextPid = 0

function shell.cleanup(streams)
  for i = #(streams.files or {}), 1, -1 do
    pcall(streams.files[i].close, streams.files[i])
  end
  for pid, process in pairs(shell.processes) do
    if process.streams == streams then
      shell.processes[pid] = nil
    end
  end
end

function shell.run(path, ...)
  -- `arg` as lua.c sets it, which oc2's programs in /mnt/builtin/bin use.
  local env = setmetatable({arg = table.pack(...)}, {__index = _G})
  env.arg[0], env.arg.n = path, nil
  local program, reason = loadfile(path, "bt", env)
  if not program then
    io.stderr:write(reason, "\n")
    return 1
  end
  local streams = system.streams()
  local parent = streams.process
  nextPid = nextPid + 1
  local process = {pid = nextPid, parent = parent and parent.pid or 0,
    command = table.concat({path:match("([^/]*)%.luac?$") or path, ...}, " "), streams = streams}
  shell.processes[process.pid] = process
  streams.process = process
  local ok, result = system.rawxpcall(program, handler, ...)
  shell.processes[process.pid], streams.process = nil, parent
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

local function split(line)
  local commands, start, i, quote = {}, 1, 1, nil
  local function cut(op, length)
    commands[#commands + 1] = {text = line:sub(start, i - 1), op = op}
    i = i + length
    start = i
  end
  while i <= #line do
    local c = line:sub(i, i)
    if quote then
      if c == quote then
        quote = nil
      elseif c == "\\" and quote == '"' then
        i = i + 1
      end
      i = i + 1
    elseif c == "'" or c == '"' then
      quote, i = c, i + 1
    elseif c == "\\" then
      i = i + 2
    elseif c == "#" and (i == 1 or line:sub(i - 1, i - 1):find("^%s")) then
      break
    elseif c == ";" or c == "\n" then
      cut(";", 1)
    elseif line:sub(i, i + 1) == "&&" or line:sub(i, i + 1) == "||" then
      cut(line:sub(i, i + 1), 2)
    elseif c == "&" then
      cut("&", 1)
      commands.background = true
    elseif c == "|" then
      cut("|", 1)
    else
      i = i + 1
    end
  end
  if quote then
    return nil, "Syntax error: Unterminated quoted string"
  end
  commands[#commands + 1] = {text = line:sub(start, i - 1), op = ";"}
  return commands
end

local runCommand

function shell.execute(line, expanding)
  local commands, reason = split(line)
  if not commands then
    io.stderr:write("sh: ", reason, "\n")
    return setLastCode(2)
  end
  for index, command in ipairs(commands) do
    local previous = index > 1 and commands[index - 1].op
    if not command.text:find("%S") and (command.op ~= ";" or previous and previous ~= ";" and previous ~= "&") then
      local token = index == #commands and "newline" or command.op
      io.stderr:write("sh: syntax error near unexpected token '", token, "'\n")
      return setLastCode(2)
    end
  end
  if commands.background then
    commands = require("jobs").group(commands)
  end
  local run, pipeline = true, {}
  for _, command in ipairs(commands) do
    pipeline[#pipeline + 1] = command.text
    if command.op == "&" then
      setLastCode(require("jobs").start(command.text))
      run, pipeline = true, {}
    elseif command.op ~= "|" then
      if run and #pipeline > 1 then
        setLastCode(require("pipe").run(pipeline))
      elseif run and command.text:find("%S") then
        local name, rest = command.text:match("^%s*([^%s]+)(.*)$")
        expanding = expanding or {}
        if aliases[name] and not expanding[name] then
          expanding[name] = true
          shell.execute(aliases[name] .. rest, expanding)
          expanding[name] = nil
        else
          runCommand(command.text)
        end
      end
      run, pipeline = command.op == ";" or (command.op == "&&") == (lastCode() == 0), {}
    end
  end
  return lastCode()
end

function runCommand(line)
  local words, globs = lex(line)
  if not words then
    io.stderr:write("sh: ", globs, "\n")
    return setLastCode(2)
  end
  local args, redirects = {}, {}
  for i, word in ipairs(words) do
    if type(word) == "table" then
      local target = words[i + 1]
      if type(target) ~= "string" then
        io.stderr:write("sh: syntax error near unexpected token 'newline'\n")
        return setLastCode(2)
      end
      local targets = globs[i + 1] and require("glob").expand(globs[i + 1]) or {target}
      if #targets > 1 then
        io.stderr:write("sh: ", target, ": ambiguous redirect\n")
        return setLastCode(1)
      end
      redirects[#redirects + 1] = {mode = REDIRECT_MODES[word.op], path = targets[1], errors = word.op:sub(1, 1) == "2"}
    elseif type(words[i - 1]) ~= "table" then
      local matches = globs[i] and require("glob").expand(globs[i]) or {word}
      table.move(matches, 1, #matches, #args + 1, args)
    end
  end
  if #args == 0 then
    return lastCode()
  end

  local depth, oldInput, oldOutput, oldErrors = shell.restore()
  for _, redirect in ipairs(redirects) do
    local file, openReason = io.open(redirect.path, redirect.mode)
    if not file then
      shell.restore(depth, oldInput, oldOutput, oldErrors)
      io.stderr:write("sh: ", openReason, "\n")
      return setLastCode(1)
    end
    local files = system.streams().files
    files[#files + 1] = file
    if redirect.errors then
      system.streams().errors = file
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
      return setLastCode(127)
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
  return setLastCode(result)
end

return shell
