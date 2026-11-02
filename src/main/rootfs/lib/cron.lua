-- CRON_MINUTE_SECONDS sets the length of a minute, for tests.

local shell = require("shell")

local cron = {}

local RANGES = {{0, 59}, {0, 23}, {1, 31}, {1, 12}, {0, 7}}

function cron.path()
  return shell.resolve((os.getenv("HOME") or "/") .. "/.crontab")
end

local function parseField(field, low, high)
  local values = {}
  for part in field:gmatch("[^,]+") do
    local range, step = part:match("^([^/]+)/?(%d*)$")
    local first, last
    if range == "*" then
      first, last = low, high
    else
      first, last = range and range:match("^(%d+)-?(%d*)$")
      first, last = tonumber(first), tonumber(last ~= "" and last or step ~= "" and high or first)
    end
    step = tonumber(step ~= "" and step or 1)
    if not first or not last or not step or step < 1 or first < low or last > high or first > last then
      return nil
    end
    for value = first, last, step do
      values[value] = true
    end
  end
  return values
end

function cron.parse(text)
  local jobs, bad, number = {}, {}, 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    number = number + 1
    if line:find("^%s*@reboot%s") then
      jobs[#jobs + 1] = {reboot = true, command = line:match("^%s*@reboot%s+(.-)%s*$")}
    elseif line:find("%S") and not line:find("^%s*#") then
      local fields = {line:match("^%s*(%S+)%s+(%S+)%s+(%S+)%s+(%S+)%s+(%S+)%s+(.-)%s*$")}
      -- Like common cron, a day or weekday that starts with * does not count as restricted.
      local job = #fields == 6 and {command = fields[6], days = fields[3]:sub(1, 1) ~= "*" and fields[5]:sub(1, 1) ~= "*"}
      for i, range in ipairs(RANGES) do
        if job then
          job[i] = parseField(fields[i], range[1], range[2])
          job = job[i] and job
        end
      end
      if job then
        job[5][7], job[5][0] = job[5][7] or job[5][0], job[5][0] or job[5][7]
        jobs[#jobs + 1] = job
      else
        bad[#bad + 1] = number
      end
    end
  end
  return jobs, bad
end

function cron.due(job, time)
  local t = os.date("*t", time)
  local day, weekday = job[3][t.day], job[5][t.wday - 1]
  return job[1][t.min] and job[2][t.hour] and job[4][t.month]
    and (job.days and (day or weekday) or (not job.days and day and weekday))
end

local function run(command)
  local job = require("thread").create(function()
    local streams = require("system").streams()
    streams.input, streams.output = io.open("/dev/null"), io.open("/dev/null", "w")
    streams.errors, streams.files = streams.output, {streams.input, streams.output}
    shell.execute(command)
  end)
  job.name, job.parent = command, 0
end

local reportedCrontab

local function load()
  local file = io.open(cron.path())
  if not file then
    return {}
  end
  local text = file:read("a") or ""
  file:close()
  local jobs, bad = cron.parse(text)
  if #bad > 0 and text ~= reportedCrontab then
    reportedCrontab = text
    io.stderr:write("cron: ", cron.path(), ": bad entry in line ", table.concat(bad, ", "), "\n")
  end
  return jobs
end

local function loop(boot)
  for _, job in ipairs(boot and load() or {}) do
    if job.reboot then
      run(job.command)
    end
  end
  while true do
    local length = tonumber(os.getenv("CRON_MINUTE_SECONDS")) or 60
    local minute = os.time() // length
    while os.time() // length == minute do
      computer.pullSignal(length - os.time() % length)
    end
    local time = (os.time() // length) * 60
    for _, job in ipairs(load()) do
      if not job.reboot and cron.due(job, time) then
        run(job.command)
      end
    end
  end
end

function cron.start(boot)
  if not cron.thread or cron.thread:status() == "dead" then
    cron.thread = require("thread").create(loop, boot)
    cron.thread.name, cron.thread.parent = "cron", 0
  end
  return cron.thread
end

return cron
