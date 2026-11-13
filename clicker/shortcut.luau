--!nonstrict
-- Control-center shortcut for the Clicker plugin.

local C = require("./lib/common.luau")

local function render(runtime)
  runtime = runtime or { phase = "idle" }
  local active = runtime.phase == "running" or runtime.phase == "countdown"
  shortcut.setLabel("Clicker")
  shortcut.setIcon("click")
  shortcut.setActive(active)
end

render(noctalia.state.get("runtime"))

noctalia.state.watch("runtime", function(runtime)
  render(runtime)
end)

function onClick()
  C.sendCommand("toggle")
end
