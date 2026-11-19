--!nonstrict
-- Clicker service: owns helper lifecycle, settings, safety, and IPC.

local C = require("./lib/common.luau")

local DEFAULT_HELPER = "~/.local/bin/clickerd"
local STATUS_POLL_MS = 200
local HELPER_STALE_MS = 3000
local HELPER_START_TIMEOUT_MS = 3000
local RESTART_DELAY_MS = 350
local FOCUS_POLL_MS = 500
local HEARTBEAT_MS = 1000

local STOP_NOTICES = {
  duration = "notification.duration",
  focus = "notification.focus_lost",
}

local runtime = {
  phase = "idle",
  mode = "clicker",
  toggleMode = "clicker",
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
  fisherState = "idle",
  fisherCasts = 0,
  fisherBites = 0,
  fisherMisses = 0,
  audioLevel = 0,
  matchScore = 0,
  templateSource = "",
  startedAt = 0,
  stopAt = 0,
  countdownEndsAt = 0,
  countdownRemaining = 0,
  sessionRate = C.DEFAULTS.rate,
  sessionAction = "left",
  sessionCustom = "space",
  sessionHold = C.DEFAULTS.hold,
  sessionJitter = C.DEFAULTS.jitter,
  sessionDelay = C.DEFAULTS.delay,
  sessionDuration = C.DEFAULTS.duration,
  sessionTarget = "",
  sessionStopOnFocus = true,
  sessionSensitivity = C.FISHER_DEFAULTS.sensitivity,
  sessionBiteTimeout = C.FISHER_DEFAULTS.biteTimeout,
  sessionReelDelay = C.FISHER_DEFAULTS.reelDelay,
  sessionPostReel = C.FISHER_DEFAULTS.postReel,
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
local lastHelperError = ""

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
  cfg.rate = tonumber(noctalia.getConfig("rate")) or C.DEFAULTS.rate
  cfg.rateUnit = noctalia.getConfig("rate_unit") or "cps"
  cfg.action = noctalia.getConfig("action") or "left"
  cfg.customKey = noctalia.getConfig("custom_key") or "space"
  cfg.holdMs = tonumber(noctalia.getConfig("hold_ms")) or C.DEFAULTS.hold
  cfg.jitterPct = tonumber(noctalia.getConfig("jitter_pct")) or C.DEFAULTS.jitter
  cfg.startDelay = tonumber(noctalia.getConfig("start_delay")) or C.DEFAULTS.delay
  cfg.maxDuration = tonumber(noctalia.getConfig("max_duration")) or C.DEFAULTS.duration
  cfg.targetApp = noctalia.getConfig("target_app") or ""
  cfg.stopOnFocus = noctalia.getConfig("stop_on_focus_loss") ~= false
  cfg.showOsd = noctalia.getConfig("show_osd") ~= false
  cfg.notify = noctalia.getConfig("notify") ~= false
  cfg.helperPath = noctalia.getConfig("helper_path") or DEFAULT_HELPER
  cfg.macroRepeat = tonumber(noctalia.getConfig("macro_default_repeat")) or 1
  cfg.macroLoopDelay = tonumber(noctalia.getConfig("macro_default_loop_delay")) or 250
  cfg.macroMaxRuntime = tonumber(noctalia.getConfig("macro_max_runtime")) or 600

  cfg.fisherTargetApp = noctalia.getConfig("fisher_target_app") or C.FISHER_DEFAULTS.targetApp
  cfg.fisherCastButton = noctalia.getConfig("fisher_cast_button") or "left"
  cfg.fisherPressMs = tonumber(noctalia.getConfig("fisher_press_ms")) or C.FISHER_DEFAULTS.press
  cfg.fisherSettleMs = tonumber(noctalia.getConfig("fisher_settle_ms")) or C.FISHER_DEFAULTS.settle
  cfg.fisherBiteTimeout = tonumber(noctalia.getConfig("fisher_bite_timeout")) or C.FISHER_DEFAULTS.biteTimeout
  cfg.fisherReelDelay = tonumber(noctalia.getConfig("fisher_reel_delay_ms")) or C.FISHER_DEFAULTS.reelDelay
  cfg.fisherPostReel = tonumber(noctalia.getConfig("fisher_post_reel_ms")) or C.FISHER_DEFAULTS.postReel
  cfg.fisherRecover = tonumber(noctalia.getConfig("fisher_recover_ms")) or C.FISHER_DEFAULTS.recover
  cfg.fisherMaxCasts = tonumber(noctalia.getConfig("fisher_max_casts")) or C.FISHER_DEFAULTS.maxCasts
  cfg.fisherMaxRuntime = tonumber(noctalia.getConfig("fisher_max_runtime")) or C.FISHER_DEFAULTS.maxRuntime
  cfg.fisherSensitivity = tonumber(noctalia.getConfig("fisher_sensitivity")) or C.FISHER_DEFAULTS.sensitivity
  cfg.fisherMinInterval = tonumber(noctalia.getConfig("fisher_min_interval")) or C.FISHER_DEFAULTS.minInterval
  cfg.fisherAudioDevice = noctalia.getConfig("fisher_audio_device") or ""
  cfg.fisherTemplate = noctalia.getConfig("fisher_template") or ""
  cfg.fisherPitchVariants = noctalia.getConfig("fisher_pitch_variants") ~= false
end

-- Session values override settings; settings override built-in defaults.
local function effectiveSession(mode)
  local fisherMode = mode == "fisher"
  local defaultTarget = fisherMode and cfg.fisherTargetApp or cfg.targetApp
  local rate = C.firstDefined(session.rate, C.cpsFrom(cfg.rate, cfg.rateUnit))
  local action = C.firstDefined(session.action, cfg.action) or "left"
  local customKey = C.firstDefined(session.customKey, cfg.customKey) or "space"
  local hold = C.firstDefined(session.hold, cfg.holdMs)
  local jitter = C.firstDefined(session.jitter, cfg.jitterPct)
  local delay = C.firstDefined(session.delay, cfg.startDelay)
  local duration = C.firstDefined(session.duration, cfg.maxDuration)
  local target = C.firstDefined(session.target, defaultTarget)
  local stopOnFocus = C.firstDefined(session.stopOnFocus, cfg.stopOnFocus)

  return {
    rate = C.clamp(tonumber(rate) or C.DEFAULTS.rate, C.RATE_MIN, C.RATE_MAX),
    action = action,
    customKey = customKey,
    hold = C.clamp(tonumber(hold) or C.DEFAULTS.hold, 0, 2000),
    jitter = C.clamp(tonumber(jitter) or C.DEFAULTS.jitter, 0, C.JITTER_MAX),
    delay = C.clamp(tonumber(delay) or C.DEFAULTS.delay, 0, 10),
    duration = C.clamp(tonumber(duration) or C.DEFAULTS.duration, 0, 86400),
    target = tostring(target or ""),
    stopOnFocus = stopOnFocus == true,
  }
end

-- Fisher overrides are kept beside the clicker ones so one panel tab does not
-- disturb the other's session.
local function effectiveFisher()
  local sensitivity = C.firstDefined(session.sensitivity, cfg.fisherSensitivity)
  local timeout = C.firstDefined(session.fisherTimeout, cfg.fisherBiteTimeout)
  local reelDelay = C.firstDefined(session.reelDelay, cfg.fisherReelDelay)
  return {
    sensitivity = C.clamp(tonumber(sensitivity) or C.FISHER_DEFAULTS.sensitivity, 0.1, 0.99),
    biteTimeout = C.clamp(tonumber(timeout) or C.FISHER_DEFAULTS.biteTimeout, 3, 300),
    reelDelay = C.clamp(tonumber(reelDelay) or C.FISHER_DEFAULTS.reelDelay, 0, 200),
    postReel = C.clamp(tonumber(C.firstDefined(session.postReel, cfg.fisherPostReel)) or C.FISHER_DEFAULTS.postReel, 100, 5000),
    runtimeLimit = tonumber(cfg.fisherMaxRuntime) or 0,
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
  local fisher = effectiveFisher()
  runtime.sessionSensitivity = fisher.sensitivity
  runtime.sessionBiteTimeout = fisher.biteTimeout
  runtime.sessionReelDelay = fisher.reelDelay
  runtime.sessionPostReel = fisher.postReel
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

-- The helper config is built in one place so startup, macro playback and live
-- updates always send the same shape.
local function helperConfig(e)
  return {
    mode = "repeat",
    cps = e.rate,
    hold_ms = e.hold,
    jitter_pct = e.jitter,
    max_runtime_s = e.duration,
    action = C.actionForHelper(e.action, e.customKey),
  }
end

local function macroConfig(e, def)
  local config = helperConfig(e)
  config.mode = "macro"
  config.max_runtime_s = def.max_runtime_s
  config.macro = def
  return config
end

local function fisherConfig()
  local fisher = effectiveFisher()
  local limit = math.max(0, tonumber(fisher.runtimeLimit) or 0)
  return {
    mode = "fisher",
    max_runtime_s = limit,
    action = { type = "mouse", button = cfg.fisherCastButton },
    fisher = {
      cast_button = cfg.fisherCastButton,
      press_ms = cfg.fisherPressMs,
      settle_ms = cfg.fisherSettleMs,
      bite_timeout_s = fisher.biteTimeout,
      reel_delay_ms = fisher.reelDelay,
      post_reel_ms = fisher.postReel,
      recover_ms = cfg.fisherRecover,
      max_casts = cfg.fisherMaxCasts,
      sensitivity = fisher.sensitivity,
      min_interval_ms = cfg.fisherMinInterval,
      audio_device = cfg.fisherAudioDevice,
      template = cfg.fisherTemplate,
      pitch_variants = cfg.fisherPitchVariants,
    },
  }
end

local function resolveHelper()
  local path = cfg.helperPath
  if type(path) ~= "string" or path == "" then
    path = DEFAULT_HELPER
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

local function releaseFallback()
  if helper.path then
    noctalia.runAsync({ helper.path, "--release-all" })
  else
    noctalia.runAsync({ "ydotool", "click", "0x80", "0x81", "0x82" })
  end
end

local stop
local startNow
local requestStart
local checkFocus
local startFocusWatch
local stopFocusWatch

local function failFocus()
  runtime.phase = "idle"
  runtime.error = "target window is not focused"
  publish(true)
  notifyError(tr("panel.title"), runtime.error)
end

local function handleStopReason(reason)
  if reason == "completed" then
    local key = runtime.mode == "fisher" and "notification.fishing_limit" or "panel.macro.finished"
    notify(tr(key))
    osd(tr(key), true)
    return
  end

  local notice = STOP_NOTICES[reason]
  if notice then
    notify(tr(notice))
    osd(tr(notice), true)
  elseif reason == "heartbeat" then
    -- The helper exits when the plugin stops updating its heartbeat, which is
    -- what happens across a suspend. That is a safety stop, not a failure.
    if runtime.error ~= "" then
      notifyError(tr("notification.helper_error"), runtime.error)
      osd(tr("notification.helper_error"), true)
    else
      osd(tr("notification.stopped"), true)
    end
  elseif reason == "error" then
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
  log(string.format("stop: %s (phase %s, mode %s)", tostring(reason), tostring(runtime.phase), tostring(runtime.mode)))
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
  runtime.fisherState = "idle"
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
  -- The helper loads its config on startup, so seed it with the current
  -- settings before launching. Without this the first launch reports a
  -- missing config.json even though nothing is running yet.
  writeHelperConfig(helperConfig(effectiveSession()))

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
  if mode == "clicker" or mode == "fisher" then
    runtime.toggleMode = mode
  end
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
  local e = effectiveSession(kind == "fisher" and "fisher" or "clicker")
  if kind == "fisher" then
    e.duration = math.max(0, tonumber(cfg.fisherMaxRuntime) or 0)
  end
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
    config = macroConfig(e, def)
    runtime.macroStepCount = #def.steps
  elseif kind == "fisher" then
    config = fisherConfig()
  else
    config = helperConfig(e)
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
  log(string.format("run started: %s, target '%s', duration %.0f", kind, tostring(e.target), e.duration))
  publish(true)
  sendSignal("USR1")

  if kind == "fisher" then
    osd(tr("notification.fishing_started"), false)
    notify(tr("notification.fishing_started"), "")
  elseif kind == "macro" then
    osd(tr("notification.macro_started") .. ": " .. tostring(macro.name), false)
    notify(tr("notification.macro_started"), macro.name)
  else
    osd(string.format("%s · %s", C.rateLabel(e.rate), C.actionLabel(e.action, e.customKey)), false)
    notify(tr("notification.started"), C.rateLabel(e.rate))
  end
end

requestStart = function(kind, macro, playMode)
  local wasActive = C.isActive(runtime)
  local e = effectiveSession(kind == "fisher" and "fisher" or "clicker")
  syncSessionToRuntime(e)
  focus.target = normalize(e.target)

  local function begin(ok)
    if not ok then
      log("focus check failed for " .. tostring(focus.target))
      failFocus()
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
    log("restarting because a start was requested while active")
    stop("restart")
    deferredStart = {
      at = noctalia.nowMs() + RESTART_DELAY_MS,
      kind = kind,
      macro = macro,
      playMode = playMode,
    }
    return
  end

  checkFocus(begin)
end

local function applyOverrides(data)
  if type(data) ~= "table" then
    return
  end

  local rate = C.firstDefined(data.cps, data.rate)
  if rate ~= nil then
    session.rate = C.clamp(tonumber(rate) or C.DEFAULTS.rate, C.RATE_MIN, C.RATE_MAX)
  end
  if data.action ~= nil then
    session.action = tostring(data.action)
  end

  local custom = C.firstDefined(data.custom_key, data.customKey)
  if custom ~= nil then
    session.customKey = tostring(custom)
  end

  local hold = C.firstDefined(data.hold_ms, data.hold)
  if hold ~= nil then
    session.hold = C.clamp(tonumber(hold) or C.DEFAULTS.hold, 0, 2000)
  end

  local jitter = C.firstDefined(data.jitter_pct, data.jitter)
  if jitter ~= nil then
    session.jitter = C.clamp(tonumber(jitter) or C.DEFAULTS.jitter, 0, C.JITTER_MAX)
  end

  local delay = C.firstDefined(data.start_delay, data.delay)
  if delay ~= nil then
    session.delay = C.clamp(tonumber(delay) or C.DEFAULTS.delay, 0, 10)
  end

  local duration = C.firstDefined(data.max_duration, data.duration)
  if duration ~= nil then
    session.duration = C.clamp(tonumber(duration) or C.DEFAULTS.duration, 0, 86400)
  end

  local target = C.firstDefined(data.target_app, data.target)
  if target ~= nil then
    session.target = tostring(target)
  end

  local stopOnFocus = C.firstDefined(data.stop_on_focus_loss, data.stopOnFocus)
  if stopOnFocus ~= nil then
    session.stopOnFocus = stopOnFocus == true or stopOnFocus == "true"
  end

  syncSessionToRuntime(effectiveSession(runtime.mode == "fisher" and "fisher" or "clicker"))
end

local function applyFisherOverrides(data)
  if type(data) ~= "table" then
    return
  end

  local sensitivity = C.firstDefined(data.sensitivity, data.fisher_sensitivity)
  if sensitivity ~= nil then
    session.sensitivity = C.clamp(tonumber(sensitivity) or C.FISHER_DEFAULTS.sensitivity, 0.1, 0.99)
  end

  local timeout = C.firstDefined(data.bite_timeout_s, data.bite_timeout, data.timeout)
  if timeout ~= nil then
    session.fisherTimeout = C.clamp(tonumber(timeout) or C.FISHER_DEFAULTS.biteTimeout, 3, 300)
  end

  local reelDelay = C.firstDefined(data.reel_delay_ms, data.reel_delay)
  if reelDelay ~= nil then
    session.reelDelay = C.clamp(tonumber(reelDelay) or C.FISHER_DEFAULTS.reelDelay, 0, 200)
  end

  local postReel = C.firstDefined(data.post_reel_ms, data.post_reel)
  if postReel ~= nil then
    session.postReel = C.clamp(tonumber(postReel) or C.FISHER_DEFAULTS.postReel, 100, 5000)
  end

  syncSessionToRuntime(effectiveSession(runtime.mode == "fisher" and "fisher" or "clicker"))
end

local function pushLiveConfig()
  if runtime.phase ~= "running" then
    return
  end

  if runtime.mode == "fisher" then
    local ok, err = writeHelperConfig(fisherConfig())
    if not ok then
      runtime.error = err or "could not update config"
      publish(true)
      return
    end
    sendSignal("HUP")
    publish(true)
    return
  end

  if runtime.mode ~= "clicker" then
    return
  end

  local e = effectiveSession()
  syncSessionToRuntime(e)
  focus.target = normalize(e.target)

  local ok, err = writeHelperConfig(helperConfig(e))
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

-- The panel tab decides what the shortcut and the widget toggle. Switching to
-- a tab for a different action stops whatever is running, so the hotkey never
-- starts a mode the user is not looking at.
local function selectMode(mode)
  if mode ~= "clicker" and mode ~= "fisher" then
    return
  end
  runtime.toggleMode = mode
end

local function toggle()
  if C.isActive(runtime) then
    stop("toggled")
    return
  end
  local mode = runtime.toggleMode == "fisher" and "fisher" or "clicker"
  log("toggle: starting " .. mode)
  requestStart(mode)
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

local function playMacroCommand(kind, data)
  local name = type(data) == "table" and data.name or data
  if not name then
    notifyError(tr("notification.macro_error"), "macro name is required")
    return
  end

  local playMode = type(data) == "table" and data.mode or "defined"
  if kind == "macro-loop" then
    playMode = "loop"
  elseif kind == "macro-once" then
    playMode = "once"
  end
  playMacro(tostring(name), playMode)
end

-- One dispatcher serves both IPC events and panel/widget commands.
local function dispatch(kind, data)
  if kind == "toggle" then
    applyOverrides(data)
    toggle()
  elseif kind == "select_mode" then
    local data_mode = type(data) == "table" and (data.mode or data.name) or data
    selectMode(data_mode)
    log(string.format(
      "mode selected: %s (phase %s, last %s)",
      tostring(runtime.toggleMode),
      tostring(runtime.phase),
      tostring(runtime.mode)
    ))
    if C.isActive(runtime) and runtime.mode ~= runtime.toggleMode then
      stop("switch")
    else
      publish(true)
    end
  elseif kind == "start" then
    applyOverrides(data)
    if runtime.phase == "running" and runtime.mode == "clicker" then
      pushLiveConfig()
    else
      requestStart("clicker")
    end
  elseif kind == "stop" or kind == "macro-stop" then
    stop("ipc")
  elseif kind == "fish" then
    applyOverrides(data)
    applyFisherOverrides(data)
    if runtime.phase == "running" and runtime.mode == "fisher" then
      pushLiveConfig()
    else
      requestStart("fisher")
    end
  elseif kind == "fish-stop" then
    stop("ipc")
  elseif kind == "set_fisher" then
    applyFisherOverrides(data)
    pushLiveConfig()
    publish(true)
  elseif kind == "panic" then
    stop("panic")
  elseif kind == "set" then
    applyOverrides(data)
    pushLiveConfig()
    publish(true)
  elseif kind == "set_rate" then
    if data ~= nil then
      session.rate = C.clamp(tonumber(data) or C.DEFAULTS.rate, C.RATE_MIN, C.RATE_MAX)
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
  elseif kind == "macro" or kind == "macro-loop" or kind == "macro-once" then
    playMacroCommand(kind, data)
  end
end

local function syncHelperError(status)
  local statusError = tostring(status.error or "")
  if statusError ~= "" then
    lastHelperError = statusError
    runtime.error = statusError
  elseif lastHelperError ~= "" then
    -- The helper recovered; drop the error it reported instead of leaving it
    -- pinned in the widget and panel.
    if runtime.error == lastHelperError then
      runtime.error = ""
    end
    lastHelperError = ""
  end
end

local function applyStatus(status)
  helper.pid = tonumber(status.pid) or helper.pid
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
  runtime.fisherState = tostring(status.fisher_state or "idle")
  runtime.fisherCasts = tonumber(status.fisher_casts) or 0
  runtime.fisherBites = tonumber(status.fisher_bites) or 0
  runtime.fisherMisses = tonumber(status.fisher_misses) or 0
  runtime.audioLevel = tonumber(status.audio_level) or 0
  runtime.matchScore = tonumber(status.match_score) or 0
  runtime.templateSource = tostring(status.template_source or "")
  syncHelperError(status)
end

local function readStatus(now)
  if now - lastStatusRead < STATUS_POLL_MS then
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

  -- After a suspend the helper can exit while the last status document still
  -- says "ready". Treat an old document as evidence that the process is gone,
  -- so the next start spawns a fresh helper instead of signalling a dead pid.
  -- While running, the stale branch below reports a wedged helper instead.
  local updated = tonumber(status.updated_at_ms) or 0
  if runtime.phase ~= "running" and (updated <= 0 or now - updated > HELPER_STALE_MS) then
    if helper.proc or helper.pid or runtime.helper ~= "stopped" then
      log("helper status is stale; marking it stopped")
      helper.proc = false
      helper.pid = nil
      helper.path = nil
      runtime.helper = "stopped"
      publish(true)
    end
    return
  end

  helper.lastSeen = now
  applyStatus(status)

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
    elseif status.updated_at_ms and now - tonumber(status.updated_at_ms) > HELPER_STALE_MS then
      runtime.error = "helper stopped responding"
      stop("heartbeat")
      releaseFallback()
    end
  elseif status.running == true and runtime.phase == "idle" and runtime.lastReason == "" then
    runtime.phase = "running"
  end

  publish(false)
end

local function writeHeartbeat(now)
  if now - lastHeartbeat < HEARTBEAT_MS then
    return
  end
  lastHeartbeat = now
  ensureRuntimeDir()
  noctalia.writeFile(C.heartbeatPath(), tostring(now))
end

local function reapHelper(now)
  if runtime.phase ~= "idle" or not helper.proc or helper.lastSeen <= 0 then
    return
  end
  if now - helper.lastSeen <= HELPER_STALE_MS then
    return
  end
  helper.proc = false
  helper.pid = nil
  helper.path = nil
  runtime.helper = "stopped"
  publish(true)
end

local function handlePendingStart(now)
  if not pendingStart or runtime.helper ~= "ready" then
    return
  end
  local item = pendingStart
  pendingStart = nil
  if now - item.at < HELPER_START_TIMEOUT_MS then
    startNow(item.kind, item.macro, item.playMode)
  else
    runtime.phase = "error"
    runtime.error = "helper did not become ready"
    publish(true)
  end
end

local function handleDeferredStart(now)
  if not deferredStart or now < deferredStart.at then
    return
  end
  local item = deferredStart
  deferredStart = nil
  local e = effectiveSession(item.kind == "fisher" and "fisher" or "clicker")
  focus.target = normalize(e.target)

  checkFocus(function(ok)
    if not ok then
      runtime.error = "target window is not focused"
      publish(true)
      return
    end
    if item.kind == "macro" or e.delay <= 0 then
      startNow(item.kind, item.macro, item.playMode)
    else
      requestStart(item.kind, item.macro, item.playMode)
    end
  end)
end

local function handleCountdown(now)
  if runtime.phase ~= "countdown" or not countdown then
    return
  end

  runtime.countdownRemaining = math.max(0, (runtime.countdownEndsAt - now) / 1000)
  if now < runtime.countdownEndsAt then
    publish(false)
    return
  end

  local item = countdown
  countdown = nil
  local e = effectiveSession(item.kind == "fisher" and "fisher" or "clicker")
  focus.target = normalize(e.target)
  checkFocus(function(ok)
    if not ok then
      failFocus()
      return
    end
    startNow(item.kind, item.macro, item.playMode)
  end)
end

function onIpc(event, payload)
  dispatch(event, parsePayload(payload))
end

function onConfigChanged()
  readConfig()
  if runtime.phase == "running" and (runtime.mode == "clicker" or runtime.mode == "fisher") then
    pushLiveConfig()
  elseif runtime.phase == "countdown" and countdown then
    countdown.e = effectiveSession(countdown.kind == "fisher" and "fisher" or "clicker")
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

  writeHeartbeat(now)
  readStatus(now)
  reapHelper(now)
  handlePendingStart(now)
  handleDeferredStart(now)
  handleCountdown(now)

  if runtime.phase == "running" and runtime.stopAt > 0 and now >= runtime.stopAt then
    stop("duration")
  end

  if runtime.phase == "running" and focus.active and not focus.checking and now - focus.lastCheck >= FOCUS_POLL_MS then
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
  dispatch(command.kind, command.data)
end)
