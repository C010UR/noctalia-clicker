--!nonstrict
-- Bar widget for the Clicker plugin.

local C = require("./lib/common.luau")

local hideWhenIdle = noctalia.getConfig("hide_when_idle") == true
local showRate = noctalia.getConfig("show_rate") ~= false

local function tr(key)
  return noctalia.tr(key)
end

-- The panel tab decides which action the widget and the hotkey toggle.
local function targetMode(runtime)
  return runtime.toggleMode == "fisher" and "fisher" or "clicker"
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
    if runtime.mode == "fisher" then
      if showRate then
        return tostring(runtime.fisherBites or 0)
      end
      return "F"
    end
    if showRate then
      return C.rateLabel(runtime.effectiveCps or runtime.sessionRate or 0)
    end
  elseif showRate then
    if targetMode(runtime) == "fisher" then
      return tostring(runtime.fisherBites or 0)
    end
    return C.rateLabel(runtime.sessionRate or 0)
  end
  return ""
end

local function tooltip(runtime)
  local modeLabel = tr("panel.tab.clicker")
  if runtime.phase == "running" and runtime.mode == "macro" then
    modeLabel = "Macro: " .. tostring(runtime.name or "")
  elseif targetMode(runtime) == "fisher" then
    modeLabel = tr("panel.tab.fishing")
  end

  local rows = {
    { key = tr("panel.mode"), value = modeLabel },
  }
  local active = C.isActive(runtime)
  local fisherStats = active and runtime.mode == "fisher"
  if not active then
    fisherStats = targetMode(runtime) == "fisher"
  end
  if fisherStats then
    table.insert(rows, { key = tr("panel.fishing.caught_short"), value = tostring(runtime.fisherBites or 0) })
    table.insert(rows, {
      key = tr("panel.fishing.casts_short"),
      value = string.format("%d · %d %s", runtime.fisherCasts or 0, runtime.fisherMisses or 0, tr("panel.fishing.misses_short")),
    })
    table.insert(rows, { key = tr("panel.fishing.match_short"), value = string.format("%.2f", tonumber(runtime.matchScore) or 0) })
  else
    table.insert(rows, { key = tr("panel.rate"), value = C.rateLabel(runtime.effectiveCps or runtime.sessionRate or 0) })
    table.insert(rows, { key = tr("panel.clicks"), value = tostring(runtime.clicks or 0) .. " · " .. tostring(runtime.keys or 0) .. " keys" })
  end
  table.insert(rows, { key = tr("panel.helper"), value = runtime.helper == "ready" and tr("panel.helper_ready") or tostring(runtime.helper or "") })
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
  local active = C.isActive(runtime)
  if hideWhenIdle and not active then
    barWidget.setVisible(false)
    return
  end
  local fishing = targetMode(runtime) == "fisher"
  barWidget.setVisible(true)
  barWidget.setGlyph(fishing and "phishing" or "click")
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
  if C.isActive(runtime) then
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
