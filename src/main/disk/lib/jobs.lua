local shell = require("shell")
local system = require("system")
local thread = require("thread")

local jobs = {}

local list = {}

local function status(job)
  if job.thread:status() ~= "dead" then
    return "Running"
  elseif job.killed then
    return "Terminated"
  end
  return job.code == 0 and "Done" or "Exit " .. tostring(job.code or 1)
end

local function line(job)
  return string.format("[%d]+  %-11s %s", job.number, status(job), job.text)
end

function jobs.group(commands)
  local grouped, parts = {}, {}
  for _, command in ipairs(commands) do
    parts[#parts + 1] = command.text
    if command.op == "&" or command.op == ";" then
      if command.op == "&" then
        grouped[#grouped + 1] = {text = table.concat(parts), op = "&"}
      else
        for i = 1, #parts, 2 do
          grouped[#grouped + 1] = {text = parts[i], op = parts[i + 1] or ";"}
        end
      end
      parts = {}
    else
      parts[#parts + 1] = command.op
    end
  end
  return grouped
end

function jobs.start(text)
  text = text:match("^%s*(.-)%s*$")
  local job = {number = (list[#list] and list[#list].number or 0) + 1, text = text}
  local parent = system.streams().process
  job.thread = thread.create(function()
    local streams = system.streams()
    streams.process, streams.input = parent, io.open("/dev/null")
    streams.files = {streams.input}
    job.code = shell.execute(text)
  end)
  job.thread.name = text
  list[#list + 1] = job
  shell.lastJob = "t" .. job.thread.id
  print("[" .. job.number .. "] " .. shell.lastJob)
  return 0
end

function jobs.report(everything)
  for i = 1, #list do
    if everything or list[i].thread:status() == "dead" then
      print(line(list[i]))
    end
  end
  for i = #list, 1, -1 do
    if list[i].thread:status() == "dead" then
      table.remove(list, i)
    end
  end
end

local function find(id)
  for _, job in ipairs(list) do
    if id == "%" .. job.number or id == "t" .. job.thread.id then
      return job
    end
  end
  for _, t in ipairs(thread.list()) do
    if id == "t" .. t.id then
      return {thread = t}
    end
  end
end

function jobs.jobs()
  jobs.report(true)
end

function jobs.wait(...)
  local waiting = {...}
  if #waiting == 0 then
    for _, job in ipairs(list) do
      waiting[#waiting + 1] = "%" .. job.number
    end
  end
  local code = 0
  for _, id in ipairs(waiting) do
    local job = find(id)
    if not job then
      io.stderr:write("sh: wait: ", id, ": no such job\n")
      code = 127
    else
      job.thread:join()
      code = job.killed and 143 or job.code or 0
    end
  end
  return code
end

function jobs.kill(...)
  if select("#", ...) == 0 then
    io.stderr:write("sh: kill: usage: kill %job | tID...\n")
    return 2
  end
  local code = 0
  for _, id in ipairs({...}) do
    local job = find(id)
    if not job then
      io.stderr:write("sh: kill: ", id, ": arguments must be job or thread IDs\n")
      code = 1
    else
      job.killed = true
      job.thread:kill()
    end
  end
  return code
end

return jobs
