# Noctalia Clicker

A Noctalia plugin for precise autoclicking and keyboard/mouse macros on
Wayland. It uses the existing `ydotoold` virtual device and a small native
helper instead of spawning a process per click.

## Layout

```
catalog.toml                 # Noctalia source catalog
clicker/                     # plugin directory (id: c010ur/clicker)
  plugin.toml                # manifest + typed settings
  service.luau               # helper lifecycle, safety, IPC
  widget.luau                # bar widget
  panel.luau                 # clicker controls + macro editor
  shortcut.luau              # control-center tile
  lib/common.luau            # shared helpers
  translations/en.json       # settings and UI strings
  helper/                    # clickerd (Rust)
  README.md                  # plugin documentation
```

## Build

```sh
cd clicker/helper
cargo build --release
./install.sh
```

## Use

Add the repository as a Noctalia path source, enable `c010ur/clicker`, then add
the widget and keybinds. See [clicker/README.md](clicker/README.md) for the full
configuration, IPC, macro format, and safety notes.

## License

MIT
