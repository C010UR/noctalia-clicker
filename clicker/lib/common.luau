--!nonstrict
-- Shared helpers for the Clicker plugin entries.

local M = {}

M.PLUGIN_ID = "c010ur/clicker"
M.SERVICE_ID = M.PLUGIN_ID .. ":service"
M.PANEL_ID = M.PLUGIN_ID .. ":panel"
M.WIDGET_ID = M.PLUGIN_ID .. ":widget"

M.RATE_MIN = 1
M.RATE_MAX = 1000
M.HOLD_MIN = 1
M.HOLD_MAX = 100
M.JITTER_MAX = 50
M.STEP_MAX = 256
M.MACRO_NAME_PATTERN = "^[%w_-]+$"

M.MOUSE_BUTTONS = { "left", "right", "middle", "side", "extra", "forward", "back" }
M.ACTION_OPTIONS = { "left", "right", "middle", "space", "custom" }
M.ACTION_LABELS = {
  left = "Left click",
  right = "Right click",
  middle = "Middle click",
  space = "Space",
  custom = "Custom key",
}

local MOUSE_SET = {}
for _, button in ipairs(M.MOUSE_BUTTONS) do
  MOUSE_SET[button] = true
end

function M.clamp(value, low, high)
  if value < low then
    return low
  end
  if value > high then
    return high
  end
  return value
end

function M.isNumber(value)
  return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

function M.deepCopy(value)
  if type(value) ~= "table" then
    return value
  end
  local copy = {}
  for key, item in pairs(value) do
    copy[key] = M.deepCopy(item)
  end
  return copy
end

function M.cpsFrom(rate, unit)
  if unit == "cpm" then
    return (tonumber(rate) or 12) / 60
  end
  return tonumber(rate) or 12
end

function M.runtimeDir()
  local base = noctalia.getenv("XDG_RUNTIME_DIR") or "/tmp"
  return base .. "/noctalia-clicker"
end

function M.configPath()
  return M.runtimeDir() .. "/config.json"
end

function M.statusPath()
  return M.runtimeDir() .. "/status.json"
end

function M.heartbeatPath()
  return M.runtimeDir() .. "/heartbeat"
end

function M.macrosPath()
  local dir = noctalia.pluginDataDir()
  if not dir or dir == "" then
    dir = M.runtimeDir()
  end
  return dir .. "/macros.json"
end

function M.sendCommand(kind, data)
  local seq = tostring(noctalia.nowMs()) .. "-" .. tostring(math.random(1, 1000000))
  noctalia.state.set("command", {
    seq = seq,
    kind = kind,
    data = data,
  })
end

function M.emptyMacroFile()
  return { version = 1, macros = {} }
end

function M.loadMacroFile()
  local raw = noctalia.readFile(M.macrosPath())
  if not raw then
    return M.emptyMacroFile()
  end
  local decoded = noctalia.json.decode(raw)
  if type(decoded) ~= "table" or type(decoded.macros) ~= "table" then
    return M.emptyMacroFile()
  end
  if decoded.version == nil then
    decoded.version = 1
  end
  return decoded
end

function M.saveMacroFile(data)
  local encoded = noctalia.json.encode(data, true)
  if not encoded then
    return false, "could not encode macros"
  end
  local dir = noctalia.pluginDataDir()
  if dir then
    noctalia.mkdirAll(dir)
  end
  local path = M.macrosPath()
  local tmp = path .. ".tmp"
  local ok, err = noctalia.writeFile(tmp, encoded)
  if not ok then
    return false, err or "could not write macros"
  end
  ok, err = noctalia.renameFile(tmp, path)
  if not ok then
    return false, err or "could not replace macros"
  end
  return true
end

function M.sortedMacroNames(macros)
  local names = {}
  for name in pairs(macros or {}) do
    table.insert(names, name)
  end
  table.sort(names, function(a, b)
    return string.lower(a) < string.lower(b)
  end)
  return names
end

function M.validateMacro(macro)
  if type(macro) ~= "table" then
    return false, "macro is not a table"
  end
  local name = macro.name
  if type(name) ~= "string" or name == "" or #name > 64 or not string.match(name, M.MACRO_NAME_PATTERN) then
    return false, "name must be 1-64 letters, digits, underscores, or dashes"
  end
  if type(macro.steps) ~= "table" then
    return false, "steps are missing"
  end
  local count = #macro.steps
  if count < 1 then
    return false, "add at least one step"
  end
  if count > M.STEP_MAX then
    return false, "too many steps (max " .. M.STEP_MAX .. ")"
  end

  for index, step in ipairs(macro.steps) do
    local prefix = "step " .. index .. ": "
    if type(step) ~= "table" then
      return false, prefix .. "not a table"
    end
    local kind = step.type
    if kind == "key" then
      if type(step.chord) ~= "string" or step.chord == "" or string.match(step.chord, "%s") then
        return false, prefix .. "enter a key or chord without spaces"
      end
      if not M.isNumber(step.hold_ms) or step.hold_ms < 0 or step.hold_ms > 2000 then
        return false, prefix .. "hold must be 0-2000 ms"
      end
      if not M.isNumber(step.delay_ms) or step.delay_ms < 0 or step.delay_ms > 60000 then
        return false, prefix .. "delay must be 0-60000 ms"
      end
    elseif kind == "mouse" then
      if not MOUSE_SET[step.button] then
        return false, prefix .. "unknown mouse button"
      end
      if not M.isNumber(step.hold_ms) or step.hold_ms < 0 or step.hold_ms > 2000 then
        return false, prefix .. "hold must be 0-2000 ms"
      end
      if not M.isNumber(step.delay_ms) or step.delay_ms < 0 or step.delay_ms > 60000 then
        return false, prefix .. "delay must be 0-60000 ms"
      end
    elseif kind == "delay" then
      if not M.isNumber(step.ms) or step.ms < 0 or step.ms > 60000 then
        return false, prefix .. "delay must be 0-60000 ms"
      end
    elseif kind == "scroll" then
      if step.direction ~= "up" and step.direction ~= "down" then
        return false, prefix .. "direction must be up or down"
      end
      if not M.isNumber(step.steps) or step.steps < 1 or step.steps > 10 then
        return false, prefix .. "scroll steps must be 1-10"
      end
    else
      return false, prefix .. "unknown step type"
    end
  end

  if macro.repeats ~= nil then
    if not M.isNumber(macro.repeats) or macro.repeats < 0 or macro.repeats > 1000000 or macro.repeats ~= math.floor(macro.repeats) then
      return false, "repeat must be a whole number from 0 to 1000000"
    end
  end
  if macro.loop_delay_ms ~= nil then
    if not M.isNumber(macro.loop_delay_ms) or macro.loop_delay_ms < 0 or macro.loop_delay_ms > 60000 then
      return false, "loop delay must be 0-60000 ms"
    end
  end
  if macro.max_runtime_s ~= nil then
    if not M.isNumber(macro.max_runtime_s) or macro.max_runtime_s < 0 or macro.max_runtime_s > 86400 then
      return false, "max runtime must be 0-86400 seconds"
    end
  end
  return true
end

function M.macroForHelper(macro, playMode, defaults)
  local copy = M.deepCopy(macro)
  copy.steps = copy.steps or {}
  if playMode == "once" then
    copy.repeats = 1
  elseif playMode == "loop" then
    copy.repeats = 0
  else
    copy.repeats = copy.repeats or defaults.macroRepeat or 1
  end
  copy.loop_delay_ms = copy.loop_delay_ms or defaults.macroLoopDelay or 250
  copy.max_runtime_s = copy.max_runtime_s or defaults.macroMaxRuntime or 600
  return copy
end

function M.actionLabel(action, customKey)
  if action == "custom" then
    return customKey or "custom key"
  end
  return M.ACTION_LABELS[action] or action or "Left click"
end

function M.actionForHelper(action, customKey)
  if action == "left" or action == "right" or action == "middle" or action == "side" or action == "extra" or action == "forward" or action == "back" then
    return { type = "mouse", button = action }
  end
  if action == "space" then
    return { type = "key", chord = "space" }
  end
  return { type = "key", chord = customKey or "space" }
end

function M.phaseLabel(phase)
  if phase == "running" then
    return "Running"
  elseif phase == "countdown" then
    return "Starting"
  elseif phase == "error" then
    return "Error"
  end
  return "Idle"
end

function M.rateLabel(cps)
  local value = tonumber(cps) or 0
  if value >= 10 then
    return string.format("%.0f CPS", value)
  end
  return string.format("%.1f CPS", value)
end

return M
