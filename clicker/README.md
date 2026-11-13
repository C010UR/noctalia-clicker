# Clicker

A [Noctalia](https://docs.noctalia.dev) plugin for a precise, configurable
autoclicker and keyboard/mouse macro engine on Wayland.

The UI is a normal Noctalia plugin: typed settings, a bar widget, a panel, and a
control-center shortcut. The actual input loop runs in `clickerd`, a small Rust
helper that speaks to the already-running `ydotoold` virtual device. That keeps
timing accurate and avoids spawning a process per click.

> **Target:** Noctalia 5.4+, plugin API 43. Hyprland is the primary compositor
> for the focus guard; the clicker itself works on other Wayland compositors as
> long as ydotoold is available.

## Requirements

- Noctalia 5.4 or newer.
- `ydotool` installed and `ydotoold` running:

  ```sh
  systemctl --user enable --now ydotool.service
  ```

- `clickerd` installed at `~/.local/bin/clickerd`:

  ```sh
  ./helper/install.sh
  ```

## Install

1. Build the helper:

   ```sh
   ./helper/install.sh
   ```

2. Add this repository as a Noctalia path source:

   ```sh
   noctalia msg plugins source add clicker path ~/Projects/noctalia-clicker
   ```

3. Enable the plugin:

   ```sh
   noctalia msg plugins enable c010ur/clicker
   ```

4. Add the bar widget. In `~/.config/noctalia/config.toml`:

   ```toml
   [widget.clicker]
   type = "c010ur/clicker:widget"
   hide_when_idle = false

   [bar.default]
   end = ["tray", "notifications", "network", "bluetooth", "volume", "brightness", "battery", "keyboard_layout", "control-center", "session", "clicker"]
   ```

   Then reload Noctalia:

   ```sh
   noctalia msg config-reload
   ```

## Hyprland keybinds

```lua
hl.cbind("SUPER + F6", hl.dsp.exec_cmd("noctalia msg plugin c010ur/clicker:service all toggle"), {
  desc = "Toggle Clicker", category = "shell", icon = "ads_click", order = 31,
})
hl.cbind("SUPER + SHIFT + F6", hl.dsp.exec_cmd("noctalia msg panel-toggle c010ur/clicker:panel"), {
  desc = "Clicker Options", category = "shell", icon = "touch_app", order = 32,
})
```

A temporary high-rate profile can be bound without changing settings:

```lua
hl.cbind("F8", hl.dsp.exec_cmd([[noctalia msg plugin c010ur/clicker:service all start '{"cps":30}']]), {
  desc = "Clicker 30 CPS", category = "shell", icon = "ads_click", order = 34,
})
```

Hold-to-click uses two binds, one on press and one on release:

```lua
hl.cbind("F7", hl.dsp.exec_cmd("noctalia msg plugin c010ur/clicker:service all start"), {
  desc = "Hold Clicker", category = "shell", icon = "ads_click", order = 35,
})
hl.cbind("F7", hl.dsp.exec_cmd("noctalia msg plugin c010ur/clicker:service all stop"), {
  desc = "Hold Clicker (release)", category = "shell", icon = "ads_click", order = 35,
  release = true,
})
```

## Settings

Settings → Plugins → Clicker:

| Setting | Meaning |
| --- | --- |
| Rate / unit | Actions per second or minute. |
| Action | Left, right, middle, space, or a custom key/chord. |
| Hold | How long the button or key stays down per press. |
| Jitter | Random timing variation, as a percentage. |
| Start delay | Countdown before clicking starts. |
| Maximum duration | Auto-stop after this many seconds; 0 means unlimited. |
| Target window | Optional class or title substring. |
| Stop on focus loss | Stop when the target window loses focus. |
| OSD / notifications | Feedback on start, stop, errors, and macro completion. |
| Helper binary | Path to `clickerd`. |
| Macro defaults | Default repeat, loop delay, and runtime cap. |

Panel changes are **session overrides**; Noctalia plugins cannot write settings
from a panel. Use **Settings** to change defaults. Scroll on the bar widget to
adjust the rate live.

Key names are physical Linux key names (`space`, `enter`, `f2`,
`ctrl+shift+a`). They follow the active keyboard layout.

## Macros

Macros are stored in Noctalia's plugin data directory as `macros.json`, so they
survive plugin updates and restarts. The panel has a full step editor.

Supported steps:

- `key` — a key or chord, with hold and delay;
- `mouse` — a mouse button, with hold and delay;
- `delay` — a fixed pause;
- `scroll` — wheel up or down.

Example file:

```json
{
  "version": 1,
  "macros": {
    "osu-burst": {
      "name": "osu-burst",
      "steps": [
        { "type": "key", "chord": "space", "hold_ms": 10, "delay_ms": 40 },
        { "type": "mouse", "button": "left", "hold_ms": 8, "delay_ms": 0 },
        { "type": "delay", "ms": 100 },
        { "type": "scroll", "direction": "down", "steps": 1 }
      ],
      "repeats": 5,
      "loop_delay_ms": 250,
      "max_runtime_s": 600
    }
  }
}
```

`repeats = 0` loops until stopped, subject to `max_runtime_s`. The editor
limits macros to 256 steps.

## IPC

```sh
# Clicker
noctalia msg plugin c010ur/clicker:service all toggle
noctalia msg plugin c010ur/clicker:service all start
noctalia msg plugin c010ur/clicker:service all start '{"cps":30,"action":"right"}'
noctalia msg plugin c010ur/clicker:service all stop
noctalia msg plugin c010ur/clicker:service all panic

# Macros
noctalia msg plugin c010ur/clicker:service all macro "osu-burst"
noctalia msg plugin c010ur/clicker:service all macro-loop "farm"
noctalia msg plugin c010ur/clicker:service all macro-once "osu-burst"
noctalia msg plugin c010ur/clicker:service all stop
```

Only one engine runs at a time. Starting a macro stops the clicker and vice
versa.

## Safety

- Every pressed key, modifier, and mouse button is tracked and released on stop.
- The helper exits and releases input if Noctalia disappears (heartbeat plus
  `PDEATHSIG`).
- `panic` releases left, right, and middle as a fallback if the helper is
  unresponsive.
- The focus guard stops clicking when the target window loses focus.
- Macros that loop have a runtime cap (`macro_max_runtime`).

## Troubleshooting

**"clickerd was not found"** — run `./helper/install.sh`, then check the helper
path in settings.

**Clicks do not land in a game** — make sure `ydotoold` is running and that the
game accepts synthetic pointer input. Some anti-cheat systems reject uinput
events; that is outside the plugin's control.

**"target window is not focused"** — the target string is a case-insensitive
substring of the window class or title. Clear it to disable the guard.

**No macro list** — macros are stored under Noctalia's plugin data directory
(`noctalia.pluginDataDir()`). Check the Noctalia log for JSON errors.
