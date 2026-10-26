if fs.exists((os.getenv("HOME") or "/") .. "/.crontab") then
  require("cron").start(true)
end
