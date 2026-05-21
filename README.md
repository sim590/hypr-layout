# hypr-layout

A CLI tool to launch and arrange tiled window layouts in Hyprland using a simple DSL.

`hypr-layout` lets you describe a window layout as a short expression (splits,
tabs, ratios, commands) and it will spawn the windows and tile them
automatically on your current workspace.

## Features

- **Nested horizontal, vertical and tabbed splits** described in a concise DSL.
- **Per-pane commands:** launch any program (terminal or GUI) in each leaf.
- **Size ratios:** control the proportion of each split.
- **Multiple terminal emulators:** built-in support for Alacritty, Kitty,
  Ghostty, Foot, WezTerm, GNOME Terminal, Konsole, and more.
- Works with both the **[dwindle]** and **[hy3]** layout engines.

## Installation

### From source (cargo)

```bash
cargo install hypr-layout
```

### Arch Linux (AUR)

*Coming soon.*

## Usage

```
hypr-layout [OPTIONS] <LAYOUT>
```

### Options

| Flag                | Description                                     | Default     |
|---------------------|-------------------------------------------------|-------------|
| `--terminal <NAME>` | Terminal emulator to use for leaf nodes         | `alacritty` |
| `--cwd <PATH>`      | Working directory for spawned terminals         | `$HOME`     |
| `--timeout <MS>`    | How long to wait for each window to appear (ms) | `5000`      |

### Layout DSL

A layout is a tree of **split nodes** and **leaf nodes**.

#### Split nodes

A split starts with a direction letter followed by its children in parentheses:

| Prefix   | Direction                 |
|----------|---------------------------|
| `h(...)` | Horizontal (side by side) |
| `v(...)` | Vertical (stacked)        |
| `t(...)` | Tabbed (requires hy3)     |

Children are separated by commas.

#### Leaf nodes

- `{command}`: run *command* inside the configured terminal emulator.
- `{}`: open a plain shell in the terminal emulator.
- `command` (no braces): run *command* directly as a standalone process (for GUI
  apps like `firefox`).

#### Size ratios

Prefix any child with `N%:` to set its share of the parent split:

```
h(70%:{btop}, {nvtop})
```

This gives `btop` 70 % of the horizontal space; `nvtop` gets the rest.
Ratios that are not specified are distributed evenly among the remaining
children. The sum of explicit ratios must not exceed 100 %.

### Examples

Firefox and Vim (running in a terminal) side by side:

```bash
hypr-layout 'h(firefox, {vim})'
```

<p align="center">
  <img src="doc/resources/demo-firefox-vim.gif" alt="Firefox + Vim side by side" width="800" />
  <br />
  <em>Figure: firefox + vim</em>
</p>

Open news feed on the side with a main browser window:

```bash
hypr-layout 'h(20%:qutebrowser --target window https://news.ycombinator.com/, qutebrowser)'
```

<p align="center">
  <img src="doc/resources/demo-news-browser.gif" alt="News feed alongside main browser" width="800" />
  <br />
  <em>Figure: news feed + browser</em>
</p>

Nested splits with tabs. This is ideal for a full workspace layout. For example:

```bash
hypr-layout 't(h(v(30%:{ranger}, {tig -w}), 30%:{}), h({vim}, 40%:{opencode}))'
```

<p align="center">
  <img src="doc/resources/demo-full-workspace-from-ranger.gif" alt="Full workspace layout launched from ranger" width="800" />
  <br />
  <em>Figure: full workspace from ranger</em>
</p>

> [!TIP]
> For example, this can be used in a ranger config (rc.conf) to open all windows straight in the the right current directory (`--cwd '%d'`):
>
>     map <alt>t chain shell -f hypr-layout --cwd '%d' 't(h(v(30%%%%:{ranger .}, {tig -w}), {}), h({vim}, 40%%%%:{opencode}))' ; quit
>
> Notice here you need to escape the `%` signs multiple times because of ranger's string interpolation.

Your music setup! Using Kitty here:

```bash
hypr-layout --terminal kitty 'h(v(30%:{pulsemixer}, {ncmpcpp -s playlist}), v({ncmpcpp -s media_library}, {ncmpcpp -s search_engine}))'
```

<p align="center">
  <img src="doc/resources/demo-ncmpcpp-pulsemixer.gif" alt="ncmpcpp + pulsemixer music setup" width="800" />
  <br />
  <em>Figure: ncmpcpp + pulsemixer</em>
</p>

## Hyprland configuration

`hypr-layout` is designed to be called directly from your Hyprland config.

### Keybind

Bind a key to spawn a layout on demand:

```
bind = $mainMod, t, exec, hypr-layout 'h(30%:{ranger}, {vim})'
```

### Autostart

Launch a default layout when Hyprland starts:

```
exec-once = hypr-layout 't(h(v(30%:{ranger}, {tig -w}), {}), h({vim}, 40%:{opencode}))'
```

## Supported terminal emulators

The following terminals are recognized and receive the correct flags
automatically:

- Alacritty
- Kitty
- Ghostty
- Foot
- WezTerm
- GNOME Terminal
- Konsole
- Xfce4 Terminal
- Tilix
- Sakura
- Terminator
- Rio

Any other terminal falls back to a generic `terminal -e sh -c "cd ... && exec
..."` invocation.

## Layout engines

`hypr-layout` detects the active Hyprland layout engine at runtime:

- **[dwindle]:** uses `layoutmsg preselect` to place new windows.
- **[hy3]:** uses `hy3:makegroup` / `hy3:changegroup` for groups and tabs.

Tabbed splits (`t(...)`) are only available with the [hy3] plugin.

[dwindle]: https://wiki.hyprland.org/Configuring/Layouts/Dwindle-Layout/
[hy3]: https://github.com/outfoxxed/hy3

## Error handling

Errors are printed to stderr and displayed as a Hyprland desktop notification,
so you get feedback even when launching from a keybind or script.

Common error scenarios:

- **TUI app launched without braces:** running `ranger` instead of
  `{ranger}` will fail because the program expects a terminal. The error
  message will suggest wrapping it in braces.
- **Singleton app already running:** apps like `firefox` that reuse an
  existing instance may not open a new window, causing a timeout.
- **Timeout too short:** slow-starting applications may need a longer
  `--timeout` value.

## License

[GPL-3.0](LICENSE)
