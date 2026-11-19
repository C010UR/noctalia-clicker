--!nonstrict
-- Clicker panel: clicker controls, the macro editor, and the fishing tab.

local C = require("./lib/common.luau")

local open = false
local tab = "clicker"
local runtime = noctalia.state.get("runtime") or {}
local draft = {}
local macrosData = C.emptyMacroFile()
local editing = nil
local originalName = nil
local selectedStep = nil
local editGeneration = 0
local openGeneration = 0
local message = ""
local messageIsError = false

local render
local loadMacros
local saveEditing
local editMacro
local deleteMacro
local playMacro
local renderClicker
local renderFishing
local renderMacros

local function tr(key)
  return noctalia.tr(key)
end

local function syncDraft(r)
  r = r or {}
  draft = {
    rate = tonumber(r.sessionRate) or C.DEFAULTS.rate,
    action = r.sessionAction or "left",
    customKey = r.sessionCustom or "space",
    hold = tonumber(r.sessionHold) or C.DEFAULTS.hold,
    jitter = tonumber(r.sessionJitter) or C.DEFAULTS.jitter,
    delay = tonumber(r.sessionDelay) or C.DEFAULTS.delay,
    duration = tonumber(r.sessionDuration) or C.DEFAULTS.duration,
    target = r.sessionTarget or "",
    stopOnFocus = r.sessionStopOnFocus ~= false,
    sensitivity = tonumber(r.sessionSensitivity) or C.FISHER_DEFAULTS.sensitivity,
    biteTimeout = tonumber(r.sessionBiteTimeout) or C.FISHER_DEFAULTS.biteTimeout,
    reelDelay = tonumber(r.sessionReelDelay) or C.FISHER_DEFAULTS.reelDelay,
    postReel = tonumber(r.sessionPostReel) or C.FISHER_DEFAULTS.postReel,
  }
end

local function sendSet()
  C.sendCommand("set", {
    rate = draft.rate,
    action = draft.action,
    custom_key = draft.customKey,
    hold_ms = draft.hold,
    jitter_pct = draft.jitter,
    start_delay = draft.delay,
    max_duration = draft.duration,
    target_app = draft.target,
    stop_on_focus_loss = draft.stopOnFocus,
  })
end

local function sendFisherSet()
  C.sendCommand("set_fisher", {
    sensitivity = draft.sensitivity,
    bite_timeout_s = draft.biteTimeout,
    reel_delay_ms = draft.reelDelay,
    post_reel_ms = draft.postReel,
  })
end

local function setMessage(text, isError)
  message = text or ""
  messageIsError = isError == true
  render()
end

-- The shortcut and widget toggle whatever this tab selects. Switching to a
-- different action stops the one that is running, in the service.
local function selectTab(next)
  tab = next
  message = ""
  if next == "clicker" then
    C.sendCommand("select_mode", { mode = "clicker" })
  elseif next == "fishing" then
    C.sendCommand("select_mode", { mode = "fisher" })
  end
  render()
end

local function actionIndex(action)
  for index, value in ipairs(C.ACTION_OPTIONS) do
    if value == action then
      return index - 1
    end
  end
  return 0
end

local function phaseColor(r)
  if r.phase == "running" then
    return "primary"
  elseif r.phase == "countdown" then
    return "secondary"
  elseif r.phase == "error" then
    return "error"
  end
  return "on_surface_variant"
end

local function section(title, children)
  local rows = {
    ui.label({ text = title, fontWeight = "bold", color = "on_surface" }),
  }
  for _, child in ipairs(children) do
    table.insert(rows, child)
  end
  return ui.column({ gap = 8, fill = "surface_container/0.35", radius = 8, padding = 12 }, rows)
end

local function sliderRow(label, valueText, slider)
  return ui.column({ gap = 4 }, {
    ui.row({ align = "center", justify = "space_between" }, {
      ui.label({ text = label, color = "on_surface_variant" }),
      ui.label({ text = valueText, color = "primary" }),
    }),
    slider,
  })
end

local function labeledRow(label, input)
  return ui.row({ align = "center", gap = 8 }, {
    ui.label({ text = label, flexGrow = 1 }),
    input,
  })
end

-- Every clicker slider edits the draft on drag and commits on release.
local function draftSlider(field, minimum, maximum, step, value)
  local function assign(next)
    draft[field] = tonumber(next) or draft[field]
  end
  return ui.slider({
    min = minimum,
    max = maximum,
    step = step,
    value = C.clamp(value, minimum, maximum),
    onChange = function(next)
      assign(next)
      render()
    end,
    onDragEnd = function(next)
      assign(next)
      sendSet()
    end,
  })
end

-- Same behaviour for the fisher tab, committing through the fisher command.
local function fisherSlider(field, minimum, maximum, step, value)
  local function assign(next)
    draft[field] = tonumber(next) or draft[field]
  end
  return ui.slider({
    min = minimum,
    max = maximum,
    step = step,
    value = C.clamp(value, minimum, maximum),
    onChange = function(next)
      assign(next)
      render()
    end,
    onDragEnd = function(next)
      assign(next)
      sendFisherSet()
    end,
  })
end

-- A text meter, because the panel API only offers labels and sliders.
local function meterRow(label, value, detail)
  local amount = C.clamp(tonumber(value) or 0, 0, 1)
  local filled = math.floor(amount * 10 + 0.5)
  local bar = string.rep("█", filled) .. string.rep("·", 10 - filled)
  return ui.row({ align = "center", justify = "space_between" }, {
    ui.label({ text = label, color = "on_surface_variant" }),
    ui.label({
      text = string.format("%s %s", bar, detail or string.format("%3.0f%%", amount * 100)),
      color = amount > 0.55 and "primary" or "on_surface_variant",
    }),
  })
end

render = function()
  if not open then
    return
  end
  local r = runtime or {}

  local header = ui.row({ align = "center", gap = 8 }, {
    ui.label({ text = tr("panel.title"), fontSize = 18, fontWeight = "bold", flexGrow = 1 }),
    ui.label({
      text = tr(
        r.phase == "countdown" and "panel.status.starting"
          or ("panel.status." .. tostring(r.phase or "idle"))
      ),
      color = phaseColor(r),
    }),
    ui.button({ glyph = "close", variant = "ghost", onClick = function()
      panel.close()
    end }),
  })

  local tabs = ui.row({ gap = 8 }, {
    ui.button({
      text = tr("panel.tab.clicker"),
      variant = tab == "clicker" and "primary" or "ghost",
      onClick = function()
        selectTab("clicker")
      end,
    }),
    ui.button({
      text = tr("panel.tab.macros"),
      variant = tab == "macros" and "primary" or "ghost",
      onClick = function()
        selectTab("macros")
      end,
    }),
    ui.button({
      text = tr("panel.tab.fishing"),
      variant = tab == "fishing" and "primary" or "ghost",
      onClick = function()
        selectTab("fishing")
      end,
    }),
  })

  local content
  if tab == "macros" then
    content = renderMacros()
  elseif tab == "fishing" then
    content = renderFishing(r)
  else
    content = renderClicker(r)
  end
  panel.render(ui.column({ gap = 12, padding = 16, flexGrow = 1 }, {
    header,
    tabs,
    content,
  }))
end

renderClicker = function(r)
  local active = C.isActive(r)
  local rateMax = math.max(200, math.min(C.RATE_MAX, math.ceil((draft.rate or C.DEFAULTS.rate) + 20)))
  local durationMax = math.max(300, math.min(3600, math.ceil((draft.duration or C.DEFAULTS.duration) + 60)))

  local statusText
  if r.phase == "countdown" then
    statusText = string.format(
      tr("panel.countdown"),
      tostring(math.max(1, math.ceil(r.countdownRemaining or 0)))
    )
  elseif r.phase == "running" and r.mode == "macro" then
    statusText = "Macro: " .. tostring(r.name or "")
  else
    statusText = tr("panel.status.idle")
  end

  local status = ui.column({ gap = 4 }, {
    ui.label({
      text = active and C.rateLabel(r.effectiveCps or draft.rate) or C.rateLabel(draft.rate),
      fontSize = 26,
      fontWeight = "bold",
      color = active and "primary" or "on_surface",
    }),
    ui.label({ text = statusText, color = "on_surface_variant" }),
  })

  local controls = ui.row({ gap = 8 }, {
    ui.button({
      text = active and tr("panel.stop") or tr("panel.start"),
      variant = active and "destructive" or "primary",
      flexGrow = 1,
      onClick = function()
        C.sendCommand(active and "stop" or "start")
      end,
    }),
    ui.button({
      text = tr("panel.test"),
      variant = "secondary",
      onClick = function()
        C.sendCommand("test")
      end,
    }),
    ui.button({
      text = tr("panel.settings"),
      variant = "ghost",
      onClick = function()
        noctalia.openSettings()
      end,
    }),
  })

  local rateSection = section(tr("panel.rate"), {
    sliderRow(
      C.rateLabel(draft.rate),
      "",
      draftSlider("rate", 1, rateMax, 1, draft.rate)
    ),
  })

  local actionOptions = {}
  for _, value in ipairs(C.ACTION_OPTIONS) do
    table.insert(actionOptions, tr("settings.action." .. value))
  end
  local actionSection = section(tr("panel.action"), {
    ui.select({
      options = actionOptions,
      selectedIndex = actionIndex(draft.action),
      onChange = function(index)
        local value = C.ACTION_OPTIONS[(tonumber(index) or 0) + 1]
        if value then
          draft.action = value
          sendSet()
          render()
        end
      end,
    }),
    ui.input({
      key = "custom-key",
      value = draft.customKey,
      placeholder = "space / ctrl+shift+a",
      visible = draft.action == "custom",
      onChange = function(value)
        draft.customKey = value
      end,
      onSubmit = function(value)
        draft.customKey = value
        sendSet()
      end,
    }),
  })

  local timingSection = section(tr("panel.hold"), {
    sliderRow(
      string.format("%s · %d ms", tr("panel.hold"), draft.hold),
      "",
      draftSlider("hold", 1, 100, 1, draft.hold)
    ),
    sliderRow(
      string.format("%s · %.0f%%", tr("panel.jitter"), draft.jitter),
      "",
      draftSlider("jitter", 0, C.JITTER_MAX, 1, draft.jitter)
    ),
    sliderRow(
      string.format("%s · %.1f s", tr("panel.start_delay"), draft.delay),
      "",
      draftSlider("delay", 0, 10, 0.5, draft.delay)
    ),
    sliderRow(
      string.format(
        "%s · %s",
        tr("panel.duration"),
        draft.duration <= 0 and tr("panel.unlimited") or string.format("%.0f s", draft.duration)
      ),
      "",
      draftSlider("duration", 0, durationMax, 5, draft.duration)
    ),
  })

  local safetySection = section(tr("panel.target"), {
    ui.input({
      key = "target-app-" .. tostring(openGeneration),
      value = draft.target,
      placeholder = "osu! / minecraft",
      submitOnEnter = true,
      onChange = function(value)
        draft.target = value
      end,
      onSubmit = function(value)
        draft.target = value
        sendSet()
      end,
    }),
    ui.row({ align = "center", justify = "space_between" }, {
      ui.label({ text = tr("panel.focus_guard"), color = "on_surface_variant" }),
      ui.toggle({
        checked = draft.stopOnFocus,
        onChange = function(value)
          draft.stopOnFocus = value == true or value == "true"
          sendSet()
          render()
        end,
      }),
    }),
  })

  local footerRows = {
    ui.label({ text = tr("panel.session_note"), color = "on_surface_variant", fontSize = 11, maxLines = 2 }),
    ui.row({ gap = 12 }, {
      ui.label({ text = string.format("%s: %d", tr("panel.clicks"), r.clicks or 0), color = "on_surface_variant" }),
      ui.label({ text = string.format("%s: %d", tr("panel.keys"), r.keys or 0), color = "on_surface_variant" }),
      ui.label({
        text = string.format(
          "%s: %s",
          tr("panel.helper"),
          r.helper == "ready" and tr("panel.helper_ready") or tostring(r.helper or "")
        ),
        color = r.helper == "missing" and "error" or "on_surface_variant",
      }),
    }),
  }
  if r.error and r.error ~= "" then
    table.insert(footerRows, ui.label({ text = r.error, color = "error", maxLines = 3 }))
  end

  return ui.scroll({ flexGrow = 1 }, {
    ui.column({ gap = 12 }, {
      status,
      controls,
      rateSection,
      actionSection,
      timingSection,
      safetySection,
      ui.column({ gap = 6 }, footerRows),
    }),
  })
end

renderFishing = function(r)
  local active = C.isActive(r) and r.mode == "fisher"
  local catches = tonumber(r.fisherBites) or 0
  local casts = tonumber(r.fisherCasts) or 0
  local misses = tonumber(r.fisherMisses) or 0

  local stateKey = "panel.fishing.state." .. tostring(r.fisherState or "idle")
  local stateText = tr(stateKey)
  if stateText == stateKey then
    stateText = tostring(r.fisherState or "")
  end

  local status = ui.column({ gap = 4 }, {
    ui.label({
      text = string.format(tr("panel.fishing.caught"), catches),
      fontSize = 22,
      fontWeight = "bold",
      color = active and "primary" or "on_surface",
    }),
    ui.label({ text = stateText, color = "on_surface_variant" }),
    ui.label({
      text = string.format(tr("panel.fishing.counters"), casts, misses),
      color = "on_surface_variant",
      fontSize = 11,
    }),
  })

  local controls = ui.row({ gap = 8 }, {
    ui.button({
      text = active and tr("panel.stop") or tr("panel.start"),
      variant = active and "destructive" or "primary",
      flexGrow = 1,
      onClick = function()
        C.sendCommand(active and "fish-stop" or "fish")
      end,
    }),
    ui.button({
      text = tr("panel.settings"),
      variant = "ghost",
      onClick = function()
        noctalia.openSettings()
      end,
    }),
  })

  local detection = section(tr("panel.fishing.detection"), {
    meterRow(tr("panel.fishing.level"), r.audioLevel or 0),
    meterRow(
      tr("panel.fishing.match"),
      r.matchScore or 0,
      string.format("%.2f", tonumber(r.matchScore) or 0)
    ),
    sliderRow(
      string.format("%s · %.2f", tr("panel.fishing.sensitivity"), draft.sensitivity),
      "",
      fisherSlider("sensitivity", 0.1, 0.99, 0.01, draft.sensitivity)
    ),
    sliderRow(
      string.format("%s · %.0f s", tr("panel.fishing.bite_timeout"), draft.biteTimeout),
      "",
      fisherSlider("biteTimeout", 3, 120, 1, draft.biteTimeout)
    ),
    sliderRow(
      string.format("%s · %.0f ms", tr("panel.fishing.reel_delay"), draft.reelDelay),
      "",
      fisherSlider("reelDelay", 0, 200, 1, draft.reelDelay)
    ),
    sliderRow(
      string.format("%s · %.0f ms", tr("panel.fishing.cast_delay"), draft.postReel),
      "",
      fisherSlider("postReel", 100, 2500, 50, draft.postReel)
    ),
  })

  local setupRows = {
    ui.label({
      text = (r.templateSource and r.templateSource ~= "" and r.templateSource)
        or tr("panel.fishing.template_auto"),
      color = "on_surface_variant",
      fontSize = 11,
      maxLines = 2,
    }),
  }
  if r.error and r.error ~= "" then
    table.insert(setupRows, ui.label({ text = r.error, color = "error", maxLines = 3 }))
  end

  return ui.scroll({ flexGrow = 1 }, {
    ui.column({ gap = 12 }, {
      status,
      controls,
      detection,
      section(tr("panel.fishing.template"), setupRows),
      section(tr("panel.fishing.tips"), {
        ui.label({ text = tr("panel.fishing.tip_rod"), color = "on_surface_variant", fontSize = 11, maxLines = 3 }),
        ui.label({ text = tr("panel.fishing.tip_audio"), color = "on_surface_variant", fontSize = 11, maxLines = 3 }),
        ui.label({ text = tr("panel.session_note"), color = "on_surface_variant", fontSize = 11, maxLines = 2 }),
      }),
    }),
  })
end

local function stepSummary(step)
  if step.type == "key" then
    return string.format(
      "Key %s · hold %d ms · delay %d ms",
      step.chord,
      step.hold_ms or 0,
      step.delay_ms or 0
    )
  elseif step.type == "mouse" then
    return string.format(
      "Mouse %s · hold %d ms · delay %d ms",
      step.button or "left",
      step.hold_ms or 0,
      step.delay_ms or 0
    )
  elseif step.type == "delay" then
    return string.format("Delay %d ms", step.ms or 0)
  elseif step.type == "scroll" then
    return string.format("Scroll %s × %d", step.direction or "down", step.steps or 1)
  end
  return "Unknown step"
end

local function numberInput(field, value, minimum, maximum, onChange)
  return ui.input({
    key = field .. "-" .. tostring(selectedStep or 0) .. "-" .. tostring(editGeneration),
    value = tostring(value or 0),
    width = 110,
    onChange = function(text)
      local parsed = tonumber(text)
      if parsed and parsed >= minimum and parsed <= maximum then
        onChange(parsed)
      end
    end,
  })
end

local function addStep(step)
  if not editing then
    return
  end
  table.insert(editing.steps, step)
  selectedStep = #editing.steps
  render()
end

local function renderStepEditor()
  if not editing or not selectedStep or not editing.steps[selectedStep] then
    return ui.label({ text = tr("panel.macro.select_step"), color = "on_surface_variant" })
  end

  local step = editing.steps[selectedStep]
  local rows = {
    ui.label({ text = string.format("%s %d", tr("panel.macro.step"), selectedStep), fontWeight = "bold" }),
  }

  if step.type == "key" then
    table.insert(rows, labeledRow(tr("panel.macro.chord"), ui.input({
      key = "chord-" .. tostring(selectedStep) .. "-" .. tostring(editGeneration),
      value = step.chord,
      width = 180,
      onChange = function(value)
        step.chord = value
      end,
    })))
    table.insert(rows, labeledRow(tr("panel.macro.hold"), numberInput("hold", step.hold_ms, 0, 2000, function(value)
      step.hold_ms = value
    end)))
    table.insert(rows, labeledRow(tr("panel.macro.delay"), numberInput("delay", step.delay_ms, 0, 60000, function(value)
      step.delay_ms = value
    end)))
  elseif step.type == "mouse" then
    local options = {}
    local index = 0
    for position, button in ipairs(C.MOUSE_BUTTONS) do
      table.insert(options, button)
      if button == step.button then
        index = position - 1
      end
    end
    table.insert(rows, ui.select({
      options = options,
      selectedIndex = index,
      onChange = function(value)
        step.button = C.MOUSE_BUTTONS[(tonumber(value) or 0) + 1] or "left"
        render()
      end,
    }))
    table.insert(rows, labeledRow(tr("panel.macro.hold"), numberInput("hold", step.hold_ms, 0, 2000, function(value)
      step.hold_ms = value
    end)))
    table.insert(rows, labeledRow(tr("panel.macro.delay"), numberInput("delay", step.delay_ms, 0, 60000, function(value)
      step.delay_ms = value
    end)))
  elseif step.type == "delay" then
    table.insert(rows, labeledRow(tr("panel.macro.delay"), numberInput("ms", step.ms, 0, 60000, function(value)
      step.ms = value
    end)))
  elseif step.type == "scroll" then
    table.insert(rows, ui.select({
      options = { "Down", "Up" },
      selectedIndex = step.direction == "up" and 1 or 0,
      onChange = function(value)
        step.direction = (tonumber(value) or 0) == 1 and "up" or "down"
        render()
      end,
    }))
    table.insert(rows, labeledRow(tr("panel.macro.amount"), numberInput("amount", step.steps, 1, 10, function(value)
      step.steps = math.floor(value)
    end)))
  end

  return ui.column({ gap = 8, fill = "surface_container/0.35", radius = 8, padding = 12 }, rows)
end

renderMacros = function()
  local rows = {}
  if message ~= "" then
    table.insert(rows, ui.label({ text = message, color = messageIsError and "error" or "primary", maxLines = 3 }))
  end

  if editing then
    table.insert(rows, ui.row({ align = "center", gap = 8 }, {
      ui.label({ text = tr("panel.macro.edit"), fontSize = 16, fontWeight = "bold", flexGrow = 1 }),
      ui.label({ text = tr("panel.macro.steps") .. " " .. tostring(#editing.steps), color = "on_surface_variant" }),
    }))
    table.insert(rows, labeledRow(tr("panel.macro.name"), ui.input({
      key = "macro-name-" .. tostring(editGeneration),
      value = editing.name,
      width = 220,
      onChange = function(value)
        editing.name = value
      end,
    })))
    table.insert(rows, sliderRow(
      string.format(
        "%s · %s",
        tr("panel.macro.repeats"),
        (editing.repeats or 0) == 0 and tr("panel.unlimited") or tostring(editing.repeats)
      ),
      "",
      ui.slider({
        min = 0,
        max = 50,
        step = 1,
        value = C.clamp(editing.repeats or 1, 0, 50),
        onChange = function(value)
          editing.repeats = math.floor(tonumber(value) or 1)
          render()
        end,
      })
    ))
    table.insert(rows, labeledRow(tr("panel.macro.loop_delay") .. " (ms)", numberInput(
      "loop-delay",
      editing.loop_delay_ms or 250,
      0,
      60000,
      function(value)
        editing.loop_delay_ms = value
      end
    )))
    table.insert(rows, labeledRow(tr("panel.macro.max_runtime") .. " (s)", numberInput(
      "max-runtime",
      editing.max_runtime_s or 600,
      0,
      86400,
      function(value)
        editing.max_runtime_s = value
      end
    )))

    table.insert(rows, ui.separator({}))
    if #editing.steps == 0 then
      table.insert(rows, ui.label({ text = tr("panel.macro.no_steps"), color = "on_surface_variant" }))
    end
    for index, step in ipairs(editing.steps) do
      local selected = index == selectedStep
      table.insert(rows, ui.row({ align = "center", gap = 6 }, {
        ui.button({
          text = tostring(index) .. ". " .. stepSummary(step),
          variant = selected and "primary" or "ghost",
          contentAlign = "start",
          flexGrow = 1,
          onClick = function()
            selectedStep = index
            render()
          end,
        }),
        ui.button({ glyph = "arrow-up", variant = "ghost", enabled = index > 1, onClick = function()
          editing.steps[index], editing.steps[index - 1] = editing.steps[index - 1], editing.steps[index]
          selectedStep = index - 1
          render()
        end }),
        ui.button({ glyph = "arrow-down", variant = "ghost", enabled = index < #editing.steps, onClick = function()
          editing.steps[index], editing.steps[index + 1] = editing.steps[index + 1], editing.steps[index]
          selectedStep = index + 1
          render()
        end }),
        ui.button({ glyph = "trash", variant = "ghost", onClick = function()
          table.remove(editing.steps, index)
          selectedStep = nil
          render()
        end }),
      }))
    end

    table.insert(rows, ui.row({ gap = 8 }, {
      ui.button({ text = tr("panel.macro.add_key"), variant = "secondary", flexGrow = 1, onClick = function()
        addStep({ type = "key", chord = "space", hold_ms = 10, delay_ms = 40 })
      end }),
      ui.button({ text = tr("panel.macro.add_mouse"), variant = "secondary", flexGrow = 1, onClick = function()
        addStep({ type = "mouse", button = "left", hold_ms = 8, delay_ms = 0 })
      end }),
    }))
    table.insert(rows, ui.row({ gap = 8 }, {
      ui.button({ text = tr("panel.macro.add_delay"), variant = "secondary", flexGrow = 1, onClick = function()
        addStep({ type = "delay", ms = 100 })
      end }),
      ui.button({ text = tr("panel.macro.add_scroll"), variant = "secondary", flexGrow = 1, onClick = function()
        addStep({ type = "scroll", direction = "down", steps = 1 })
      end }),
    }))

    table.insert(rows, renderStepEditor())

    table.insert(rows, ui.row({ gap = 8 }, {
      ui.button({ text = tr("panel.macro.save"), variant = "primary", flexGrow = 1, onClick = function()
        saveEditing(false)
      end }),
      ui.button({ text = tr("panel.macro.test"), variant = "secondary", onClick = function()
        if saveEditing(true) then
          playMacro(editing.name, "once")
        end
      end }),
      ui.button({ text = tr("panel.macro.cancel"), variant = "ghost", onClick = function()
        editing = nil
        originalName = nil
        selectedStep = nil
        message = ""
        render()
      end }),
    }))
  else
    table.insert(rows, ui.row({ align = "center", gap = 8 }, {
      ui.label({ text = tr("panel.tab.macros"), fontSize = 16, fontWeight = "bold", flexGrow = 1 }),
      ui.button({ text = tr("panel.macro.new"), variant = "primary", onClick = function()
        local base = "macro"
        local name = base
        local index = 2
        while macrosData.macros[name] do
          name = base .. "-" .. tostring(index)
          index = index + 1
        end
        editing = {
          name = name,
          steps = { { type = "mouse", button = "left", hold_ms = 8, delay_ms = 0 } },
          repeats = 1,
          loop_delay_ms = 250,
          max_runtime_s = 600,
        }
        editGeneration = editGeneration + 1
        originalName = nil
        selectedStep = 1
        message = ""
        render()
      end }),
    }))

    local names = C.sortedMacroNames(macrosData.macros)
    if #names == 0 then
      table.insert(rows, ui.label({ text = tr("panel.macro.no_macros"), color = "on_surface_variant" }))
    else
      for _, name in ipairs(names) do
        local macro = macrosData.macros[name]
        local repeats = (macro.repeats or 0) == 0 and "loop" or (tostring(macro.repeats or 1) .. "x")
        table.insert(rows, ui.column({ gap = 6, fill = "surface_container/0.35", radius = 8, padding = 10 }, {
          ui.row({ align = "center", gap = 8 }, {
            ui.label({ text = name, fontWeight = "bold", flexGrow = 1 }),
            ui.label({
              text = string.format("%d steps · %s", #(macro.steps or {}), repeats),
              color = "on_surface_variant",
            }),
          }),
          ui.row({ gap = 6 }, {
            ui.button({ text = tr("panel.macro.play"), variant = "primary", flexGrow = 1, onClick = function()
              playMacro(name, "defined")
            end }),
            ui.button({ text = tr("panel.macro.loop"), variant = "secondary", onClick = function()
              playMacro(name, "loop")
            end }),
            ui.button({ text = tr("panel.macro.edit"), variant = "ghost", onClick = function()
              editMacro(name)
            end }),
            ui.button({ glyph = "trash", variant = "ghost", onClick = function()
              deleteMacro(name)
            end }),
          }),
        }))
      end
    end
  end

  return ui.scroll({ flexGrow = 1 }, {
    ui.column({ gap = 10 }, rows),
  })
end

loadMacros = function()
  macrosData = C.loadMacroFile()
end

saveEditing = function(silent)
  if not editing then
    return false
  end
  local ok, err = C.validateMacro(editing)
  if not ok then
    setMessage(err, true)
    return false
  end
  if not originalName or originalName ~= editing.name then
    if macrosData.macros[editing.name] then
      setMessage(tr("panel.macro.name_exists"), true)
      return false
    end
  end

  local clean = {
    name = editing.name,
    steps = C.deepCopy(editing.steps),
    repeats = editing.repeats or 1,
    loop_delay_ms = editing.loop_delay_ms or 250,
    max_runtime_s = editing.max_runtime_s or 600,
  }
  if originalName and originalName ~= clean.name then
    macrosData.macros[originalName] = nil
  end
  macrosData.macros[clean.name] = clean
  local saved, saveErr = C.saveMacroFile(macrosData)
  if not saved then
    setMessage(saveErr or "could not save macro", true)
    return false
  end
  originalName = clean.name
  editing = C.deepCopy(clean)
  loadMacros()
  if not silent then
    setMessage(tr("panel.macro.saved"), false)
  end
  return true
end

editMacro = function(name)
  local macro = macrosData.macros[name]
  if not macro then
    setMessage(tr("panel.macro.unknown"), true)
    return
  end
  editing = C.deepCopy(macro)
  editGeneration = editGeneration + 1
  originalName = name
  selectedStep = #(editing.steps or {}) > 0 and 1 or nil
  message = ""
  render()
end

deleteMacro = function(name)
  macrosData.macros[name] = nil
  local ok, err = C.saveMacroFile(macrosData)
  if not ok then
    setMessage(err or "could not delete macro", true)
    return
  end
  if editing and editing.name == name then
    editing = nil
    originalName = nil
    selectedStep = nil
  end
  loadMacros()
  setMessage("", false)
end

playMacro = function(name, mode)
  C.sendCommand("macro", { name = name, mode = mode })
end

function onOpen(context)
  open = true
  openGeneration = openGeneration + 1
  runtime = noctalia.state.get("runtime") or {}
  if context == "macros" then
    tab = "macros"
  elseif context == "fishing" then
    tab = "fishing"
    C.sendCommand("select_mode", { mode = "fisher" })
  elseif context == "clicker" then
    tab = "clicker"
    C.sendCommand("select_mode", { mode = "clicker" })
  elseif tab ~= "macros" then
    tab = runtime.toggleMode == "fisher" and "fishing" or "clicker"
  end
  syncDraft(runtime)
  loadMacros()
  editing = nil
  originalName = nil
  selectedStep = nil
  message = ""
  render()
end

function onClose()
  open = false
end

function onConfigChanged()
  runtime = noctalia.state.get("runtime") or {}
  syncDraft(runtime)
  render()
end

noctalia.state.watch("runtime", function(value)
  runtime = value or {}
  if open then
    render()
  end
end)
