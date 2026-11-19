# Clicker

A [Noctalia](https://docs.noctalia.dev) plugin for a precise, configurable
autoclicker, a keyboard/mouse macro engine, and a Terraria Fisher on
Wayland.

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

- For the Terraria Fisher only: PipeWire/PulseAudio with `pactl` and `parec`, and
  Terraria installed through Steam (the helper reads the game's own bite sound).

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

Bind the clicker so you are not holding **Super** when the clicks start:
Hyprland turns Super + right-click into `SUPER + mouse:273` (resize window), so
the game never receives those clicks. The example uses Ctrl, which has no mouse
bind, but any modifier still held during clicking is delivered to the focused
app, so release it after toggling.

`toggle` follows the panel's selected tab, so one bind covers both actions. With
**Clicker** selected it starts and stops the autoclicker; switch the panel to
**Terraria Fisher** and the same bind starts and stops the Terraria Fisher. Changing tabs
stops whatever was running, so the hotkey always toggles the mode you are
looking at. The **Macros** tab does not change the toggle target.

```lua
hl.cbind("CTRL + F6", hl.dsp.exec_cmd("noctalia msg plugin c010ur/clicker:service all toggle"), {
  desc = "Toggle Clicker / Terraria Fisher", category = "shell", icon = "ads_click", order = 31,
})
hl.cbind("CTRL + SHIFT + F6", hl.dsp.exec_cmd("noctalia msg panel-toggle c010ur/clicker:panel"), {
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
| Hold | How long the button or key stays down per press. At least half the interval (up to 20 ms) is used automatically so per-frame games still see each press. |
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

Games that read the mouse once per frame (XNA/FNA titles such as Terraria) need
each press to span a frame. The helper keeps the button down for at least half
the click interval, capped at 20 ms. At 60 FPS, rates up to about 25 CPS leave
both the press and the release visible to the game; exact divisors of the frame
rate (30, 20, 15 CPS) can beat against the frame clock at very short holds.

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

## Terraria Fisher

The Terraria Fisher casts, listens for the game's bite sound, reels in, and repeats.

It never reads the screen. When a fish bites, the bobber plays the game's
`Content/Sounds/Splash_5.xnb` sample, with a randomized pitch of about ±5
semitones. The helper reads that waveform from your own installation, captures
the default speaker's monitor with `parec`, and compares the last 120 ms against
pitch-shifted copies of it as a band-energy fingerprint. That survives the
resampling PipeWire applies before the monitor, and it is independent of
resolution, zoom, biome, lighting, and mods.

### Setup

1. Stand at water, select a fishing rod, and make sure you have bait.
2. Open the panel, switch to **Terraria Fisher**, and press **Start**. A three-second
   countdown gives you time to refocus the game. Selecting the tab also points
   the hotkey and the bar widget at the Terraria Fisher, so one bind can toggle either
   action.
3. Keep Terraria focused. Losing focus stops the run.

The panel shows the live input level, the current match score, casts, misses,
and catches. The bar widget shows the catch count while the Terraria Fisher is selected
or running, and clicking it toggles the selected action.

### Cycle

| Step | Default | Setting |
| --- | --- | --- |
| Cast (left click) | 40 ms press | `fisher_press_ms` |
| Ignore audio after casting | 1200 ms | `fisher_settle_ms` |
| Wait for a bite | up to 30 s | `fisher_bite_timeout` |
| Wait after a bite before reeling | 33 ms | `fisher_reel_delay_ms` |
| Reel in on a bite | 40 ms press | `fisher_press_ms` |
| Pause after a catch | 600 ms | `fisher_post_reel_ms` |
| Reel in an empty line, then recast | 350 ms | `fisher_recover_ms` |
| Minimum gap between bites | 1200 ms | `fisher_min_interval` |

`fisher_max_casts` and `fisher_max_runtime` stop the run automatically, and the
target window defaults to Terraria's Steam class, `steam_app_105600`.

### Detection

The match score is a cosine similarity between the bite sound's band-energy
shape and the last 120 ms of audio. Real bites score around 0.75–0.85; unrelated
sounds stay below ~0.65. The default `fisher_sensitivity` of 0.70 sits between
them: raise it if you get false reels, lower it if bites are missed.
`fisher_pitch_variants` matches ±5 semitones of pitch shift, which the game
applies randomly to every splash.

`fisher_reel_delay_ms` waits one or two frames after the match before clicking.
The splash can play a frame before Terraria marks the bobber as ready; the
default 33 ms (two frames at 60 FPS) prevents reeling too early and losing the
catch. Raise it if catches are still missed, or lower it toward 17 ms for the
fastest response.

To check the audio path without sending any input:

```sh
clickerd --probe-audio 15 --config "$XDG_RUNTIME_DIR/noctalia-clicker/config.json"
```

It prints a level meter and the live match score. Cast a line and watch the score
jump on a bite. A profile with `"mode": "fisher"` can also be used directly.

## IPC

```sh
# Clicker / Terraria Fisher (follows the panel's selected tab)
noctalia msg plugin c010ur/clicker:service all toggle
noctalia msg plugin c010ur/clicker:service all select_mode fisher
noctalia msg plugin c010ur/clicker:service all start
noctalia msg plugin c010ur/clicker:service all start '{"cps":30,"action":"right"}'
noctalia msg plugin c010ur/clicker:service all stop
noctalia msg plugin c010ur/clicker:service all panic

# Macros
noctalia msg plugin c010ur/clicker:service all macro "osu-burst"
noctalia msg plugin c010ur/clicker:service all macro-loop "farm"
noctalia msg plugin c010ur/clicker:service all macro-once "osu-burst"
noctalia msg plugin c010ur/clicker:service all stop

# Terraria Fisher
noctalia msg plugin c010ur/clicker:service all fish
noctalia msg plugin c010ur/clicker:service all fish '{"sensitivity":0.6,"bite_timeout":20}'
noctalia msg plugin c010ur/clicker:service all fish-stop
```

`toggle` starts and stops whichever action the panel is showing (`select_mode`
changes that target from outside the UI). Selecting a different mode stops the
running one first.

Only one engine runs at a time. Starting a macro stops the clicker and vice
versa.

## Safety

- Every pressed key, modifier, and mouse button is tracked and released on stop.
- The helper exits and releases input if Noctalia disappears (heartbeat plus
  `PDEATHSIG`).
- `panic` releases left, right, and middle as a fallback if the helper is
  unresponsive.
- The focus guard stops clicking when the target window loses focus, and the
  Terraria Fisher refuses to start outside its target window.
- The Terraria Fisher only listens to audio; it never captures the screen.
- Macros that loop have a runtime cap (`macro_max_runtime`).

## Troubleshooting

**"clickerd was not found"** — run `./helper/install.sh`, then check the helper
path in settings.

**Clicks do not land in a game** — make sure `ydotoold` is running and that the
game accepts synthetic pointer input. Some anti-cheat systems reject uinput
events; that is outside the plugin's control.

**"target window is not focused"** — the target string is a case-insensitive
substring of the window class or title. Clear it to disable the guard.

**The Terraria Fisher never reels in** — open the panel's Terraria Fisher tab and check the level
meter. If it stays empty, the capture device is wrong: set `fisher_audio_device`
to your speaker's monitor (see `pactl list short sources | grep monitor`) or make
sure the game is not muted. If the level moves but the match score never rises,
the bite sound file is missing: point `fisher_template` at
`.../Terraria/Content/Sounds/Splash_5.xnb`. `clickerd --probe-audio`
shows the same meters from a terminal.

**No macro list** — macros are stored under Noctalia's plugin data directory
(`noctalia.pluginDataDir()`). Check the Noctalia log for JSON errors.

## Development

The helper is a small Rust binary split by responsibility:

| Module | Responsibility |
| --- | --- |
| `config` | Parse and validate the JSON written by the plugin. |
| `program` | Turn validated config into prepared actions and macro steps. |
| `timing` | Repeat rate, hold floor, and jitter. |
| `engine` | Signals, lifecycle, status, and the run loops. |
| `input` | The ydotool datagram client. |

Run the checks from `helper/`:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The Lua side can be syntax-checked with `luac -p`; `noctalia plugins lint`
cross-checks settings keys against the code.
