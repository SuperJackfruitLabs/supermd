# Themes

**⌘ T** opens the theme picker. SuperMD follows your system's light/dark setting, with a theme for each: pick your light theme and your dark theme once, and the app switches with your OS.

Eight themes ship built in: **Jackfruit Light** and **Jackfruit Dark** (the defaults), **Paper**, **Graphite**, **Nord**, **Gruvbox Dark**, and **Solarized** in both light and dark. Diagrams, code highlighting, and the whole interface follow the active theme.

## Custom themes

A theme is a single TOML file dropped into your themes folder:

- macOS / Linux: `~/.supermd/themes/`
- Windows: `%USERPROFILE%\.supermd\themes\`

The shape:

```toml
name = "My Theme"
appearance = "dark"   # or "light" — decides which system mode it offers in

[colors]
bg = "#2e3440"          # page background
fg = "#d8dee9"          # body text
fg_strong = "#eceff4"   # headings, emphasis
fg_muted = "#616e88"    # secondary text
accent = "#bf616a"      # highlights, links in chrome
link = "#88c0d0"        # links in documents
code_bg = "#3b4252"     # code block background
code_fg = "#d8dee9"     # code text
border = "#434c5e"
panel_bg = "#292e39"    # sidebar, overlays
hover_bg = "#3b4252"
selected_bg = "#434c5e"
find_match_bg = "#665c22"
find_active_bg = "#8a7d33"

[syntax]
keyword = "#81a1c1"
function = "#88c0d0"
type = "#8fbcbb"
string = "#a3be8c"
comment = "#616e88"
constant = "#b48ead"
```

Optionally, `diff_added_bg`, `diff_added_fg`, `diff_deleted_bg`, and `diff_deleted_fg` under `[colors]` tune the git diff view; sensible defaults are used otherwise.

Restart SuperMD and your theme appears in the picker alongside the built-ins. A theme file that doesn't parse is skipped with a message in the terminal — it never breaks the app.
