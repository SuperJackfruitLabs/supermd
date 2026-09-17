# Themes

**⌘ T** opens the theme picker. SuperMD follows your system's light/dark setting, with a theme for each: pick your light theme and your dark theme once, and the app switches with your OS.

Eight themes ship built in: **Jackfruit Light** and **Jackfruit Dark** (the defaults), **Paper**, **Graphite**, **Nord**, **Gruvbox Dark**, and **Solarized** in both light and dark. Diagrams, code highlighting, and the whole interface follow the active theme.

## Flux: themes that follow the sun

Like f.lux, but for your editor's own palette: as evening falls, SuperMD can fade to your dark theme and gently warm every color toward candle-light — then fade back at dawn. Enable it from the command palette (**⌘ ⇧ P** → *Flux: Enable Adaptive Theme*) or in `~/.supermd/settings.toml`:

```toml
[flux]
enabled = true
latitude = 51.51        # optional — your rough coordinates
longitude = -0.13
auto_dark = true        # crossfade to the dark theme at night
warm_shift = true       # drift colors warmer after sunset
night_kelvin = 3400     # how warm the night gets (6500 = no shift)
transition_minutes = 40 # fade length around sunrise and sunset
```

With coordinates set, sunrise and sunset are computed **offline** with the NOAA solar equations — no location permission, no network, nothing leaves your machine. Without them, a fixed 7:00–19:00 day window applies (as it does under a polar sun). Both halves are independent: keep `auto_dark` and drop `warm_shift` for a hard theme schedule, or the reverse to stay on your system theme but lose the blue light at night.

## Custom themes

A theme is a single TOML file dropped into your themes folder:

- macOS / Linux: `~/.supermd/themes/`
- Windows: `%USERPROFILE%\.supermd\themes\`

The Mac App Store build is sandboxed, so its settings and themes live inside the app's container instead: `~/Library/Containers/com.superjackfruit.supermd/Data/.supermd/themes/`. You do not have to type that — **Reveal Settings Folder** in the Help menu opens it in Finder.

The shape:

```toml
name = "My Theme"
appearance = "dark"   # or "light" — decides which system mode it offers in

[colors]
bg = "#272c36"          # the desk the page rests on
page_bg = "#2e3440"     # the document itself — where the reading happens
fg = "#d8dee9"          # body text
fg_strong = "#eceff4"   # headings, emphasis
fg_muted = "#8492ad"    # secondary text
accent = "#bf616a"      # highlights, links in chrome
link = "#88c0d0"        # links in documents
code_bg = "#3b4252"     # code block background
code_fg = "#d8dee9"     # code text
border = "#434c5e"
border_subtle = "#434c5e8c"  # hairlines — table rules and the like
shadow = "#10141c57"    # the page's drop shadow: tint + strength
floating_bg = "#313744" # the finder, palette, menus and dialogs
panel_bg = "#22262f"    # table headers, the knowledge panel, banners
hover_bg = "#343a48"
selected_bg = "#3c4454"
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

### The page and the desk

The document sits on its own surface. `page_bg` is that surface — the one a reader actually looks at — and `bg` is the desk behind and beneath it, which the sidebar, tab strip, outline and status bar share. If your theme predates this split, or you simply leave the four newer keys out, they are derived from the colors you did give: `page_bg` steps one notch away from `bg`, `border_subtle` is `border` at reduced alpha, `shadow` is picked to suit your `appearance`, and `floating_bg` is worked out as described below. Your theme keeps working; setting them by hand is how you take control of the relationship.

Two things are worth checking by eye once you set `page_bg` yourself, because both are drawn *on the page* rather than on the desk:

- **`code_bg`** — a fence tuned against the old single background can vanish against a brighter page.
- **`panel_bg` and `hover_bg`** — a table's header row is `panel_bg` and a hovered table row is `hover_bg`, both painted on the page. If either matches `page_bg`, that feedback disappears. `hover_bg` also highlights an inactive tab, and the active tab carries `page_bg`, so a collision there makes every hovered tab look active.

### Things that float

`floating_bg` is the surface of everything that floats above the page: the finder, the command palette, workspace search, the theme picker, menus, link previews, the selection toolbar, and dialogs. The rule it follows is simple — **a surface that floats above the page is never darker than the page**. A shadow can only darken what is beside a surface, so an overlay darker than the page under it reads as a hole with a recess wall rather than as a card lifted off it.

In a dark theme that means a step lighter than `page_bg`. Left out, it is derived halfway between `page_bg` and `hover_bg`, which keeps it a visible step up while a hovered or selected row painted on it still stands out. If you set it yourself, keep it below `hover_bg` for the same reason. In a light theme the page is usually at or near white, with no brighter step to take, so the derived `floating_bg` is `page_bg` itself and the shadow does the separating. It should be opaque: an overlay is drawn over whatever is beneath it.

And one that catches nearly every theme by surprise: **`fg_muted` is painted on all six backgrounds** — the desk, the page, `panel_bg`, `floating_bg`, `hover_bg` and `selected_bg`. The sidebar's chevrons and folder icons are muted on whatever the row is painted with, and the finder's directory hints, the palette's plugin names, the search results' line numbers and the `[[` completion popup's path hints are all muted text on a *selected* row. `selected_bg` is the far end of that ramp, so it is the one that decides: a `fg_muted` picked to look right on the page will usually be a step too faint there.

### Colors with alpha

A color is `#rrggbb`, or `#rrggbbaa` when it needs to be translucent — the last pair of digits is the alpha, `00` transparent through `ff` opaque.

`shadow` really wants the second form. It is a tint *and* a strength: SuperMD scales that alpha across the shadow's layers, so a six-digit `shadow = "#000000"` means fully opaque black and would paint a hard band around the page instead of a falloff. Write `#00000057` — black at about a third — or pick a near-black in your theme's own hue so the shadow deepens the desk rather than draining it. Anything stronger than 50% alpha is capped at load, so a mistake here dims rather than disfigures.

Restart SuperMD and your theme appears in the picker alongside the built-ins. A theme file that doesn't parse is skipped with a message in the terminal — it never breaks the app.
