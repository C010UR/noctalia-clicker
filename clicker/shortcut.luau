--!nonstrict
-- Control-center shortcut for the Clicker plugin.
--
-- The tile mirrors the panel tab that is selected: the hotkey bound to it
-- toggles the clicker or the auto-fisher, whichever the panel is showing.

local C = require("./lib/common.luau")

local function tr(key)
  return noctalia.tr(key)
end

local function fishingMode(runtime)
  return runtime.toggleMode == "fisher"
end

local function render(runtime)
  runtime = runtime or { phase = "idle" }
  local fishing = fishingMode(runtime)
  shortcut.setLabel(fishing and tr("panel.tab.fishing") or tr("panel.tab.clicker"))
  shortcut.setIcon(fishing and "phishing" or "click")
  shortcut.setActive(C.isActive(runtime))
end

render(noctalia.state.get("runtime"))

noctalia.state.watch("runtime", function(runtime)
  render(runtime)
end)

function onClick()
  C.sendCommand("toggle")
end
