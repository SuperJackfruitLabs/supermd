# Capturing App Store screenshots

Driving SuperMD for store screenshots, and the pitfalls that make it slower
than it looks. The App Store accepts 1280x800, 1440x900, 2560x1600 or
2880x1800 for macOS.

## Approach

The window opens at a fixed origin (100,60) sized 1200x800 points, so
`screencapture -R 100,60,1200,800` captures exactly the window and nothing
else on the desktop. On a Retina display that yields 2400x1600, which pads to
2560x1600 with `sips --padToHeightWidth 1600 2560 --padColor <hex>`.

Use the theme's own background as the pad colour and the matting is
invisible — the result reads as one continuous canvas rather than a crop.

## Run the app against an isolated HOME

```sh
HOME=<isolated> open -n -a <bundle> --args <workspace>
```

The isolated home needs `.supermd/settings.toml` with flux disabled and the
themes pinned, plus `.supermd/plugins/` populated from `dist/plugins` — a
bare dev-run home seeds nothing, and plugin fences such as `chart` or the
inline calc will render as plain text. This also leaves the real
`~/.supermd` untouched.

## Pitfalls

**The app follows system appearance.** `theme.rs` resolves the light or dark
slot from it, and a slot only accepts a theme whose `is_dark` matches — so
naming a light theme in `dark_theme` silently falls back to the first dark
theme instead. To shoot light screenshots, set the system to Light.

**Synthesized keystrokes do not reach the app.** `osascript` /
System Events `keystroke` is silently dropped — verified by sending only
Cmd-P and capturing: no overlay appeared. `set frontmost` does work, because
it goes by PID. Wrapping the binary in a proper `.app` with a bundle
identifier does not help either. Any state needing a keystroke has to be set
up by hand.

**Raise the window before capturing.** `screencapture -R` composites whatever
is visually on top of that region, so without raising the app you capture
whatever window happens to be there — a terminal, most likely.

**Park the mouse outside the window.** Hover highlights a table row, which
reads as a stray selection on a store page.

**A freshly opened file puts the cursor at offset 0**, inside the H1 — so
hybrid WYSIWYG reveals its `#` and the title looks like unrendered Markdown.
Click into body text first. (Deliberately useful for one shot: a heading
showing its marker while the rest of the document renders is the clearest
single picture of what the app does.)

**`flowchart TD` is too tall** for a 16:10 frame and clips; `graph LR`
renders wide and short, and fits.

## A workspace worth shooting

Give the screenshots real content rather than lorem ipsum: a few
cross-linked notes so the graph has a shape, a table, a `graph LR` mermaid
diagram, a fenced code block, a plugin fence, checkboxes and tags. Make it a
git repo with one commit plus an uncommitted edit, so Show Changes has
genuine word-level diffs instead of an empty state.
