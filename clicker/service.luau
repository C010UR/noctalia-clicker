--!nonstrict
-- Clicker service: owns helper lifecycle, settings, safety, and IPC.

local C = require("./lib/common.luau")

local runtime = {
  phase = "idle",
  mode = "clicker",
  name = "",
  helper = "starting",
  helperVersion = "",
  error = "",
  lastReason = "",
  clicks = 0,
  keys = 0,
  effectiveCps = 0,
  effectiveHold = 0,
  macroStep = 0,
  macroStepCount = 0,
  macroRepeat = 0,
  macroRepeatIndex = 0,
  startedAt = 0,
  stopAt = 0,
  countdownEndsAt = 0,
  countdownRemaining = 0,
  sessionRate = 12,
  sessionAction = "left",
  sessionCustom = "space",
  sessionHold = 8,
  sessionJitter = 0,
  sessionDelay = 3,
  sessionDuration = 0,
  sessionTarget = "",
  sessionStopOnFocus = true,
  overrides = false,
}

local cfg = {}
local session = {}
local helper = { proc = false, pid = nil, path = nil, lastSeen = 0 }
local focus = { active = false, target = "", checking = false, lastCheck = 0 }
local pendingStart = nil
local countdown = nil
local deferredStart = nil
local lastStatusRead = 0
local lastHeartbeat = 0
local lastPublish = ""
local lastCommandSeq = nil

local function tr(key)
  return noctalia.tr(key)
end

local function log(message)
  noctalia.log("[clicker] " .. tostring(message))
end

local function notify(title, body)
  if cfg.notify then
    noctalia.notify(title, body or "")
  end
end

local function notifyError(title, body)
  if cfg.notify then
    noctalia.notifyError(title, body or "")
  end
end

local function osd(text, inactive)
  if cfg.showOsd then
    noctalia.osd.show("click", tostring(text), { inactive = inactive == true })
  end
end

local function readConfig()
  cfg.rate = tonumber(noctalia.getConfig("rate")) or 12
  cfg.rateUnit = noctalia.getConfig("rate_unit") or "cps"
  cfg.action = noctalia.getConfig("action") or "left"
  cfg.customKey = noctalia.getConfig("custom_key") or "space"
  cfg.holdMs = tonumber(noctalia.getConfig("hold_ms")) or 8
  cfg.jitterPct = tonumber(noctalia.getConfig("jitter_pct")) or 0
  cfg.startDelay = tonumber(noctalia.getConfig("start_delay")) or 0
  cfg.maxDuration = tonumber(noctalia.getConfig("max_duration")) or 0
  cfg.targetApp = noctalia.getConfig("target_app") or ""
  cfg.stopOnFocus = noctalia.getConfig("stop_on_focus_loss") ~= false
  cfg.showOsd = noctalia.getConfig("show_osd") ~= false
  cfg.notify = noctalia.getConfig("notify") ~= false
  cfg.helperPath = noctalia.getConfig("helper_path") or "~/.local/bin/clickerd"
  cfg.macroRepeat = tonumber(noctalia.getConfig("macro_default_repeat")) or 1
  cfg.macroLoopDelay = tonumber(noctalia.getConfig("macro_default_loop_delay")) or 250
  cfg.macroMaxRuntime = tonumber(noctalia.getConfig("macro_max_runtime")) or 600
end

local function effectiveSession()
  local rate = session.rate or C.cpsFrom(cfg.rate, cfg.rateUnit)
  local action = session.action or cfg.action
  local custom = session.customKey or cfg.customKey
  local hold = session.hold or cfg.holdMs
  local jitter = session.jitter or cfg.jitterPct
  local delay = session.delay
  if delay == nil then
    delay = cfg.startDelay
  end
  local duration = session.duration
  if duration == nil then
    duration = cfg.maxDuration
  end
  local target = session.target
  if target == nil then
    target = cfg.targetApp
  end
  local stopOnFocus = session.stopOnFocus
  if stopOnFocus == nil then
    stopOnFocus = cfg.stopOnFocus
  end
  return {
    rate = C.clamp(tonumber(rate) or 12, C.RATE_MIN, C.RATE_MAX),
    action = action,
    customKey = custom,
    hold = C.clamp(tonumber(hold) or 8, 0, 2000),
    jitter = C.clamp(tonumber(jitter) or 0, 0, C.JITTER_MAX),
    delay = C.clamp(tonumber(delay) or 0, 0, 10),
    duration = C.clamp(tonumber(duration) or 0, 0, 86400),
    target = tostring(target or ""),
    stopOnFocus = stopOnFocus == true,
  }
end

local function syncSessionToRuntime(e)
  runtime.sessionRate = e.rate
  runtime.sessionAction = e.action
  runtime.sessionCustom = e.customKey
  runtime.sessionHold = e.hold
  runtime.sessionJitter = e.jitter
  runtime.sessionDelay = e.delay
  runtime.sessionDuration = e.duration
  runtime.sessionTarget = e.target
  runtime.sessionStopOnFocus = e.stopOnFocus
  runtime.overrides = next(session) ~= nil
end

local function publish(force)
  local encoded = noctalia.json.encode(runtime)
  if not force and encoded == lastPublish then
    return
  end
  lastPublish = encoded
  noctalia.state.set("runtime", runtime)
end

local function ensureRuntimeDir()
  noctalia.mkdirAll(C.runtimeDir())
end

local function writeTextAtomic(path, text)
  local tmp = path .. ".tmp"
  local ok, err = noctalia.writeFile(tmp, text)
  if not ok then
    return false, err or "write failed"
  end
  ok, err = noctalia.renameFile(tmp, path)
  if not ok then
    return false, err or "rename failed"
  end
  return true
end

local function writeHelperConfig(config)
  ensureRuntimeDir()
  local encoded = noctalia.json.encode(config)
  if not encoded then
    return false, "could not encode config"
  end
  return writeTextAtomic(C.configPath(), encoded)
end

local function resolveHelper()
  local path = cfg.helperPath
  if type(path) ~= "string" or path == "" then
    path = "~/.local/bin/clickerd"
  end
  if string.sub(path, 1, 1) == "~" then
    path = noctalia.expandPath(path)
  end
  if path ~= "clickerd" and noctalia.fileExists(path) then
    return path
  end
  if noctalia.commandExists("clickerd") then
    return "clickerd"
  end
  if noctalia.fileExists(path) then
    return path
  end
  return nil
end

local function sendSignal(signal)
  local pid = helper.pid
  if not pid then
    return false
  end
  noctalia.runAsync({ "kill", "-" .. signal, tostring(pid) })
  return true
end

local stop
local startNow
local requestStart
local checkFocus
local startFocusWatch
local stopFocusWatch

local function releaseFallback()
  if helper.path then
    noctalia.runAsync({ helper.path, "--release-all" })
  else
    noctalia.runAsync({ "ydotool", "click", "0x80", "0x81", "0x82" })
  end
end

local function handleStopReason(reason)
  if reason == "completed" then
    notify(tr("panel.macro.finished"))
    osd(tr("panel.macro.finished"), true)
  elseif reason == "duration" then
    notify(tr("notification.duration"))
    osd(tr("notification.duration"), true)
  elseif reason == "focus" then
    notify(tr("notification.focus_lost"))
    osd(tr("notification.focus_lost"), true)
  elseif reason == "heartbeat" or reason == "error" then
    local detail = runtime.error ~= "" and runtime.error or tr("notification.helper_error")
    notifyError(tr("notification.helper_error"), detail)
    osd(tr("notification.helper_error"), true)
  elseif reason == "panic" then
    osd(tr("notification.stopped"), true)
  elseif reason == "stopped" or reason == "toggled" or reason == "ipc" or reason == "restart" then
    osd(tr("notification.stopped"), true)
  end
end

stop = function(reason)
  local wasActive = runtime.phase ~= "idle"
  pendingStart = nil
  countdown = nil
  deferredStart = nil
  stopFocusWatch()
  if helper.proc then
    sendSignal("USR2")
  end
  runtime.phase = "idle"
  runtime.stopAt = 0
  runtime.countdownEndsAt = 0
  runtime.countdownRemaining = 0
  runtime.macroStep = 0
  runtime.lastReason = reason or "stopped"
  publish(true)
  if wasActive then
    if reason == "panic" then
      releaseFallback()
    end
    handleStopReason(reason or "stopped")
  end
end

local function spawnHelper(quiet)
  if helper.proc then
    return
  end
  local path = resolveHelper()
  if not path then
    helper.path = nil
    helper.pid = nil
    runtime.helper = "missing"
    runtime.error = tr("notification.helper_missing")
    publish(true)
    if not quiet then
      notifyError(tr("panel.title"), tr("notification.helper_missing"))
    end
    return
  end

  ensureRuntimeDir()
  noctalia.removeFile(C.statusPath())
  noctalia.writeFile(C.heartbeatPath(), tostring(noctalia.nowMs()))
  local function shellQuote(value)
    return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
  end
  local command = table.concat({
    "exec",
    shellQuote(path),
    "--daemon",
    "--config",
    shellQuote(C.configPath()),
    "--status",
    shellQuote(C.statusPath()),
    "--heartbeat",
    shellQuote(C.heartbeatPath()),
  }, " ")
  local accepted = noctalia.runStream(command, function(_line) end)

  if not accepted then
    runtime.helper = "error"
    runtime.error = "could not launch clickerd"
    publish(true)
    if not quiet then
      notifyError(tr("panel.title"), runtime.error)
    end
    return
  end

  helper.proc = true
  helper.path = path
  helper.lastSeen = noctalia.nowMs()
  runtime.helper = "starting"
  runtime.error = ""
  publish(true)
end

local function normalize(value)
  return string.lower(tostring(value or ""))
end

local function matchesWindow(class, title)
  local target = focus.target
  if target == "" then
    return true
  end
  return string.find(normalize(class), target, 1, true) ~= nil
    or string.find(normalize(title), target, 1, true) ~= nil
end

checkFocus = function(callback)
  local target = focus.target
  if target == "" then
    callback(true)
    return
  end
  if focus.checking then
    return
  end
  focus.checking = true
  noctalia.runAsync({ "hyprctl", "-j", "activewindow" }, function(result)
    focus.checking = false
    if not result or result.exitCode ~= 0 then
      callback(false)
      return
    end
    local data = noctalia.json.decode(result.stdout or "")
    if type(data) ~= "table" then
      callback(false)
      return
    end
    callback(matchesWindow(data.class or data.initialClass, data.title))
  end)
end

stopFocusWatch = function()
  focus.active = false
  focus.checking = false
end

startFocusWatch = function()
  if not cfg.stopOnFocus or focus.target == "" then
    focus.active = false
    return
  end
  focus.active = true
  focus.lastCheck = 0
end

local function setRunningState(mode, name, e)
  runtime.phase = "running"
  runtime.mode = mode
  runtime.name = name or ""
  runtime.error = ""
  runtime.lastReason = ""
  runtime.startedAt = noctalia.nowMs()
  runtime.countdownEndsAt = 0
  runtime.countdownRemaining = 0
  runtime.macroStep = 0
  runtime.macroStepCount = 0
  runtime.macroRepeat = 0
  runtime.macroRepeatIndex = 0
  runtime.stopAt = 0
  if e.duration > 0 then
    runtime.stopAt = runtime.startedAt + e.duration * 1000
  end
  syncSessionToRuntime(e)
  focus.target = normalize(e.target)
  startFocusWatch()
end

startNow = function(kind, macro, playMode)
  local e = effectiveSession()
  if helper.proc and runtime.helper ~= "ready" then
    pendingStart = { kind = kind, macro = macro, playMode = playMode, at = noctalia.nowMs() }
    return
  end
  if not helper.proc then
    spawnHelper(false)
    pendingStart = { kind = kind, macro = macro, playMode = playMode, at = noctalia.nowMs() }
    return
  end

  local config
  if kind == "macro" then
    local def = C.macroForHelper(macro, playMode, cfg)
    config = {
      mode = "macro",
      cps = e.rate,
      hold_ms = e.hold,
      jitter_pct = e.jitter,
      max_runtime_s = def.max_runtime_s,
      action = C.actionForHelper(e.action, e.customKey),
      macro = def,
    }
    runtime.macroStepCount = #def.steps
  else
    config = {
      mode = "repeat",
      cps = e.rate,
      hold_ms = e.hold,
      jitter_pct = e.jitter,
      max_runtime_s = e.duration,
      action = C.actionForHelper(e.action, e.customKey),
    }
  end

  local ok, err = writeHelperConfig(config)
  if not ok then
    runtime.phase = "error"
    runtime.error = err or "could not write config"
    publish(true)
    notifyError(tr("panel.title"), runtime.error)
    return
  end

  setRunningState(kind, macro and macro.name or "", e)
  publish(true)
  sendSignal("USR1")

  if kind == "macro" then
    osd(tr("notification.macro_started") .. ": " .. tostring(macro.name), false)
    notify(tr("notification.macro_started"), macro.name)
  else
    osd(string.format("%s · %s", C.rateLabel(e.rate), C.actionLabel(e.action, e.customKey)), false)
    notify(tr("notification.started"), C.rateLabel(e.rate))
  end
end

requestStart = function(kind, macro, playMode)
  local wasActive = runtime.phase == "running" or runtime.phase == "countdown"
  local e = effectiveSession()
  syncSessionToRuntime(e)
  focus.target = normalize(e.target)

  local function begin(ok)
    if not ok then
      runtime.phase = "idle"
      runtime.error = "target window is not focused"
      publish(true)
      notifyError(tr("panel.title"), runtime.error)
      return
    end
    if kind == "macro" or e.delay <= 0 then
      startNow(kind, macro, playMode)
      return
    end
    countdown = { kind = kind, macro = macro, playMode = playMode, e = e }
    runtime.phase = "countdown"
    runtime.mode = kind
    runtime.name = macro and macro.name or ""
    runtime.countdownEndsAt = noctalia.nowMs() + e.delay * 1000
    runtime.countdownRemaining = e.delay
    syncSessionToRuntime(e)
    publish(true)
    osd(string.format(tr("panel.countdown"), tostring(e.delay)), false)
  end

  if wasActive then
    stop("restart")
    deferredStart = { at = noctalia.nowMs() + 350, kind = kind, macro = macro, playMode = playMode }
    return
  end

  if e.target ~= "" then
    checkFocus(begin)
  else
    begin(true)
  end
end

local function applyOverrides(data)
  if type(data) ~= "table" then
    return
  end
  local rate = data.cps or data.rate
  if rate ~= nil then
    session.rate = C.clamp(tonumber(rate) or 12, C.RATE_MIN, C.RATE_MAX)
  end
  if data.action ~= nil then
    session.action = tostring(data.action)
  end
  local custom = data.custom_key or data.customKey
  if custom ~= nil then
    session.customKey = tostring(custom)
  end
  local hold = data.hold_ms or data.hold
  if hold ~= nil then
    session.hold = C.clamp(tonumber(hold) or 8, 0, 2000)
  end
  local jitter = data.jitter_pct or data.jitter
  if jitter ~= nil then
    session.jitter = C.clamp(tonumber(jitter) or 0, 0, C.JITTER_MAX)
  end
  local delay = data.start_delay or data.delay
  if delay ~= nil then
    session.delay = C.clamp(tonumber(delay) or 0, 0, 10)
  end
  local duration = data.max_duration or data.duration
  if duration ~= nil then
    session.duration = C.clamp(tonumber(duration) or 0, 0, 86400)
  end
  local target = data.target_app or data.target
  if target ~= nil then
    session.target = tostring(target)
  end
  local stopOnFocus = data.stop_on_focus_loss
  if stopOnFocus == nil then
    stopOnFocus = data.stopOnFocus
  end
  if stopOnFocus ~= nil then
    session.stopOnFocus = stopOnFocus == true or stopOnFocus == "true"
  end
  syncSessionToRuntime(effectiveSession())
end

local function pushLiveConfig()
  if runtime.phase ~= "running" or runtime.mode ~= "clicker" then
    return
  end
  local e = effectiveSession()
  syncSessionToRuntime(e)
  focus.target = normalize(e.target)
  local config = {
    mode = "repeat",
    cps = e.rate,
    hold_ms = e.hold,
    jitter_pct = e.jitter,
    max_runtime_s = e.duration,
    action = C.actionForHelper(e.action, e.customKey),
  }
  local ok, err = writeHelperConfig(config)
  if not ok then
    runtime.error = err or "could not update config"
    publish(true)
    return
  end
  sendSignal("HUP")
  if e.duration > 0 then
    runtime.stopAt = runtime.startedAt + e.duration * 1000
  else
    runtime.stopAt = 0
  end
  stopFocusWatch()
  startFocusWatch()
  publish(true)
end

local function toggle()
  if runtime.phase == "running" or runtime.phase == "countdown" then
    stop("toggled")
  else
    requestStart("clicker")
  end
end

local function playMacro(name, playMode)
  local data = C.loadMacroFile()
  local macro = data.macros[name]
  if not macro then
    notifyError(tr("notification.macro_error"), tr("panel.macro.unknown"))
    return
  end
  local ok, err = C.validateMacro(macro)
  if not ok then
    notifyError(tr("notification.macro_error"), err)
    return
  end
  requestStart("macro", macro, playMode)
end

local function testClick()
  local e = effectiveSession()
  if e.action == "space" or e.action == "custom" then
    notify(tr("panel.test"), "Test is only available for mouse actions")
    return
  end
  if helper.path then
    noctalia.runAsync({ helper.path, "--click-once", e.action })
  else
    local codes = { left = "0xC0", right = "0xC1", middle = "0xC2" }
    noctalia.runAsync({ "ydotool", "click", codes[e.action] or "0xC0" })
  end
end

local function parsePayload(payload)
  if payload == nil then
    return nil
  end
  local text = tostring(payload)
  if text == "" then
    return nil
  end
  if string.sub(text, 1, 1) == "{" then
    local decoded, err = noctalia.json.decode(text)
    if not decoded then
      notifyError(tr("panel.title"), err or "invalid payload")
      return nil
    end
    return decoded
  end
  return { name = text }
end

local function handleCommand(command)
  if type(command) ~= "table" then
    return
  end
  local kind = command.kind
  local data = command.data
  if kind == "toggle" then
    applyOverrides(data)
    toggle()
  elseif kind == "start" then
    applyOverrides(data)
    if runtime.phase == "running" and runtime.mode == "clicker" then
      pushLiveConfig()
    else
      requestStart("clicker")
    end
  elseif kind == "stop" then
    stop("ipc")
  elseif kind == "panic" then
    stop("panic")
  elseif kind == "set" then
    applyOverrides(data)
    pushLiveConfig()
    publish(true)
  elseif kind == "set_rate" then
    if data ~= nil then
      session.rate = C.clamp(tonumber(data) or 12, C.RATE_MIN, C.RATE_MAX)
      syncSessionToRuntime(effectiveSession())
      pushLiveConfig()
      publish(true)
    end
  elseif kind == "reset" then
    session = {}
    syncSessionToRuntime(effectiveSession())
    pushLiveConfig()
    publish(true)
  elseif kind == "test" then
    testClick()
  elseif kind == "macro" then
    local name = type(data) == "table" and data.name or data
    if name then
      playMacro(tostring(name), type(data) == "table" and data.mode or "defined")
    end
  elseif kind == "macro-loop" then
    local name = type(data) == "table" and data.name or data
    if name then
      playMacro(tostring(name), "loop")
    end
  elseif kind == "macro-once" then
    local name = type(data) == "table" and data.name or data
    if name then
      playMacro(tostring(name), "once")
    end
  end
end

local function readStatus(now)
  if now - lastStatusRead < 200 then
    return
  end
  lastStatusRead = now
  local raw = noctalia.readFile(C.statusPath())
  if not raw then
    if helper.proc and runtime.helper == "starting" then
      publish(false)
    end
    return
  end
  local status = noctalia.json.decode(raw)
  if type(status) ~= "table" then
    return
  end

  helper.pid = tonumber(status.pid) or helper.pid
  helper.lastSeen = now
  runtime.helperVersion = tostring(status.version or "")
  runtime.helper = status.ready and "ready" or "starting"
  runtime.effectiveCps = tonumber(status.effective_cps) or runtime.effectiveCps
  runtime.effectiveHold = tonumber(status.effective_hold_ms) or runtime.effectiveHold
  runtime.clicks = tonumber(status.clicks) or 0
  runtime.keys = tonumber(status.keys) or 0
  runtime.macroStep = tonumber(status.macro_step) or 0
  runtime.macroStepCount = tonumber(status.macro_step_count) or runtime.macroStepCount
  runtime.macroRepeat = tonumber(status.macro_repeat) or 0
  runtime.macroRepeatIndex = tonumber(status.macro_repeat_index) or 0
  if status.error and status.error ~= "" then
    runtime.error = tostring(status.error)
  end

  if runtime.phase == "running" then
    if status.running == false then
      if now - (runtime.startedAt or 0) > 450 then
        local reason = tostring(status.reason or "stopped")
        runtime.phase = "idle"
        runtime.stopAt = 0
        runtime.macroStep = 0
        runtime.lastReason = reason
        stopFocusWatch()
        handleStopReason(reason)
        publish(true)
      end
    elseif status.updated_at_ms and now - tonumber(status.updated_at_ms) > 3000 then
      runtime.error = "helper stopped responding"
      stop("heartbeat")
      releaseFallback()
    end
  elseif status.running == true and runtime.phase == "idle" and runtime.lastReason == "" then
    runtime.phase = "running"
  end

  publish(false)
end

function onIpc(event, payload)
  local data = parsePayload(payload)
  if event == "toggle" then
    applyOverrides(data)
    toggle()
  elseif event == "start" then
    applyOverrides(data)
    if runtime.phase == "running" and runtime.mode == "clicker" then
      pushLiveConfig()
    else
      requestStart("clicker")
    end
  elseif event == "stop" or event == "macro-stop" then
    stop("ipc")
  elseif event == "panic" then
    stop("panic")
  elseif event == "macro" then
    if data and data.name then
      playMacro(data.name, data.mode or "defined")
    else
      notifyError(tr("notification.macro_error"), "macro name is required")
    end
  elseif event == "macro-loop" then
    if data and data.name then
      playMacro(data.name, "loop")
    else
      notifyError(tr("notification.macro_error"), "macro name is required")
    end
  elseif event == "macro-once" then
    if data and data.name then
      playMacro(data.name, "once")
    else
      notifyError(tr("notification.macro_error"), "macro name is required")
    end
  end
end

function onConfigChanged()
  readConfig()
  if runtime.phase == "running" and runtime.mode == "clicker" then
    pushLiveConfig()
  elseif runtime.phase == "countdown" and countdown then
    countdown.e = effectiveSession()
    runtime.countdownEndsAt = noctalia.nowMs() + countdown.e.delay * 1000
    runtime.countdownRemaining = countdown.e.delay
  end
  publish(true)
end

function onSuspend()
  stop("suspend")
end

function onResume()
  spawnHelper(true)
end

function onExit(_signal, _reason)
  stopFocusWatch()
  noctalia.removeFile(C.heartbeatPath())
  if helper.proc then
    sendSignal("TERM")
  end
  if helper.path then
    noctalia.runAsync({ helper.path, "--release-all" })
  end
end

function update()
  local now = noctalia.nowMs()

  if now - lastHeartbeat >= 1000 then
    lastHeartbeat = now
    ensureRuntimeDir()
    noctalia.writeFile(C.heartbeatPath(), tostring(now))
  end

  readStatus(now)

  if runtime.phase == "idle" and helper.proc and helper.lastSeen > 0 and now - helper.lastSeen > 3000 then
    helper.proc = false
    helper.pid = nil
    helper.path = nil
    runtime.helper = "stopped"
    publish(true)
  end

  if pendingStart and runtime.helper == "ready" then
    local item = pendingStart
    pendingStart = nil
    if now - item.at < 3000 then
      startNow(item.kind, item.macro, item.playMode)
    else
      runtime.phase = "error"
      runtime.error = "helper did not become ready"
      publish(true)
    end
  end

  if deferredStart and now >= deferredStart.at then
    local item = deferredStart
    deferredStart = nil
    local e = effectiveSession()
    if e.target ~= "" then
      checkFocus(function(ok)
        if ok then
          if item.kind == "macro" or e.delay <= 0 then
            startNow(item.kind, item.macro, item.playMode)
          else
            requestStart(item.kind, item.macro, item.playMode)
          end
        else
          runtime.error = "target window is not focused"
          publish(true)
        end
      end)
    else
      if item.kind == "macro" or e.delay <= 0 then
        startNow(item.kind, item.macro, item.playMode)
      else
        requestStart(item.kind, item.macro, item.playMode)
      end
    end
  end

  if runtime.phase == "countdown" and countdown then
    runtime.countdownRemaining = math.max(0, (runtime.countdownEndsAt - now) / 1000)
    if now >= runtime.countdownEndsAt then
      local item = countdown
      countdown = nil
      local e = effectiveSession()
      local function begin(ok)
        if ok then
          startNow(item.kind, item.macro, item.playMode)
        else
          runtime.phase = "idle"
          runtime.error = "target window is not focused"
          publish(true)
          notifyError(tr("panel.title"), runtime.error)
        end
      end
      if e.target ~= "" then
        checkFocus(begin)
      else
        begin(true)
      end
    else
      publish(false)
    end
  end

  if runtime.phase == "running" and runtime.stopAt > 0 and now >= runtime.stopAt then
    stop("duration")
  end

  if runtime.phase == "running" and focus.active and not focus.checking and now - focus.lastCheck >= 500 then
    focus.lastCheck = now
    checkFocus(function(ok)
      if not ok and runtime.phase == "running" then
        stop("focus")
      end
    end)
  end
end

readConfig()
noctalia.setUpdateInterval(250)
ensureRuntimeDir()
noctalia.writeFile(C.heartbeatPath(), tostring(noctalia.nowMs()))
spawnHelper(true)
syncSessionToRuntime(effectiveSession())
publish(true)

local existing = noctalia.state.get("command")
if type(existing) == "table" then
  lastCommandSeq = existing.seq
end

noctalia.state.watch("command", function(command)
  if type(command) ~= "table" or command.seq == lastCommandSeq then
    return
  end
  lastCommandSeq = command.seq
  handleCommand(command)
end)
