--!nonstrict
-- Bar widget for the Clicker plugin.

local C = require("./lib/common.luau")

local hideWhenIdle = noctalia.getConfig("hide_when_idle") == true
local showRate = noctalia.getConfig("show_rate") ~= false

local function tr(key)
  return noctalia.tr(key)
end

local function isActive(runtime)
  return runtime.phase == "running" or runtime.phase == "countdown"
end

local function statusLabel(runtime)
  if runtime.phase == "countdown" then
    return string.format("%.0fs", math.max(1, math.ceil(runtime.countdownRemaining or 0)))
  end
  if runtime.phase == "running" then
    if runtime.mode == "macro" then
      local step = tonumber(runtime.macroStep) or 0
      local total = tonumber(runtime.macroStepCount) or 0
      if total > 0 then
        return string.format("%d/%d", step, total)
      end
      return "M"
    end
    if showRate then
      return C.rateLabel(runtime.effectiveCps or runtime.sessionRate or 0)
    end
  elseif showRate then
    return C.rateLabel(runtime.sessionRate or 0)
  end
  return ""
end

local function tooltip(runtime)
  local rows = {
    { key = tr("panel.mode"), value = runtime.mode == "macro" and ("Macro: " .. tostring(runtime.name or "")) or tr("panel.tab.clicker") },
    { key = tr("panel.rate"), value = C.rateLabel(runtime.effectiveCps or runtime.sessionRate or 0) },
    { key = tr("panel.clicks"), value = tostring(runtime.clicks or 0) .. " · " .. tostring(runtime.keys or 0) .. " keys" },
    { key = tr("panel.helper"), value = runtime.helper == "ready" and tr("panel.helper_ready") or tostring(runtime.helper or "") },
  }
  if runtime.phase == "running" and runtime.mode == "macro" and (runtime.macroRepeat or 0) > 0 then
    table.insert(rows, {
      key = tr("panel.macro.repeat"),
      value = string.format("%d/%d", runtime.macroRepeatIndex or 0, runtime.macroRepeat or 0),
    })
  end
  if runtime.error and runtime.error ~= "" then
    table.insert(rows, { key = tr("panel.error"), value = tostring(runtime.error) })
  end
  return rows
end

local function render(runtime)
  runtime = runtime or { phase = "idle", helper = "starting", sessionRate = 12 }
  local active = isActive(runtime)
  if hideWhenIdle and not active then
    barWidget.setVisible(false)
    return
  end
  barWidget.setVisible(true)
  barWidget.setGlyph("click")
  barWidget.setGlyphColor(active and "primary" or "on_surface_variant")
  barWidget.setText(statusLabel(runtime))
  barWidget.setTooltip(tooltip(runtime))
end

render(noctalia.state.get("runtime"))

noctalia.state.watch("runtime", function(runtime)
  render(runtime)
end)

function onClick()
  local runtime = noctalia.state.get("runtime") or {}
  if isActive(runtime) then
    C.sendCommand("stop")
  else
    C.sendCommand("toggle")
  end
end

function onRightClick()
  noctalia.togglePanel(C.PANEL_ID)
end

function onScroll(axis, steps, _startsGesture)
  if axis ~= "vertical" then
    return
  end
  local runtime = noctalia.state.get("runtime") or {}
  local rate = tonumber(runtime.sessionRate) or 12
  local step = rate >= 20 and 2 or 1
  local nextRate = C.clamp(rate - steps * step, C.RATE_MIN, C.RATE_MAX)
  if nextRate ~= rate then
    C.sendCommand("set_rate", nextRate)
  end
end
