# Changelog

Everything notable that changes in SuperMD, newest first. The shape is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[semantic versioning](https://semver.org/spec/v2.0.0.html).

Release notes are written here first. The GitHub release body is
generated from commit subjects, which for 0.0.16 produced a single line
linking a pull request and had to be rewritten by hand afterwards.

## [0.0.17] - 2026-09-21

### Added

- A workspace graph you can read and stay in. Hovering a node names it
  and its neighbours at any zoom; resting on one shows a card with its
  title, an excerpt and its link counts. Clicking peeks into a panel
  beside the graph instead of replacing it, and walking a backlink from
  there moves the peek without leaving the view.
- A visual language of surfaces: the document is a page resting on the
  ground, with page, border and shadow tokens derived for any theme that
  does not define them, and a surface of its own for everything that
  floats.
- Appearance setting — Light, Dark or System — at the top of the ⌘T
  picker. An explicit choice outranks flux, so Light stays light at
  midnight rather than flipping, and confirming a theme moves the
  setting to the appearance you just previewed.
- Twenty more themes, converted from base16 rather than hand-written,
  each with a page of its own; the theme picker filters as you type.

### Changed

- The graph board paints as a single canvas rather than an element per
  node: at a few thousand notes a frame went from 6.3ms to 70µs, and the
  view from 11,308 elements to one. Unlinked notes now dim rather than
  crowding the structure they surround.
- The sidebar, outline and status bar read as one ground; the active tab
  is the page's own edge; the page is inset left, right and bottom.
- Overlays share one shadow vocabulary, and tables lose their interior
  vertical rules.
- Every theme token is measured against the surface it is actually
  painted on: body ink on the six it is written on, muted ink on those
  six plus the code fence, and diff ink on its own wash.

### Fixed

- Opening a note from the graph no longer throws the graph away. The
  layout, pan, zoom and filter survive, so reopening it lands exactly
  where you left rather than re-simulating from scratch.
- The reading view renders images, renders HTML blocks as literal text
  instead of dropping them, keeps HTML comments invisible, and toggles
  checkboxes.
- Frontmatter is metadata rather than a giant heading — to the knowledge
  index as well as to the editor.
- Thematic breaks draw as a rule in the editor, and a changed one keeps
  its diff colour.
- Right-clicking a rendered table opens the table menu; delete-row leaves
  the caret where it belongs and stops refusing silently.
- The app survives losing its last window.
- The sidebar shows ignored files and the watcher wakes for them, while
  hidden scratch files (`.DS_Store`, editor swap files) stop waking it
  for a repaint of what is already on screen.
- Enter in a numbered list no longer parses the whole document, and the
  watcher's hardlink check walks the workspace once per event batch
  instead of once per changed file.

## [0.0.16] - 2026-09-13

### Added

- Multiple windows (⌘⇧N, and Open Folder in New Window). Each window is
  its own workspace: its own tabs, knowledge index, graph, and plugin
  sandbox root, so two vaults open at once never show each other's notes.

### Changed

- Plugin authors: `workspace-read` is now per window, and is no longer
  available to `render_inline` (whose results are cached across every
  window) — inline calls from a plugin declaring it fail with a named
  error instead of silently seeing an empty `/workspace`.

[Unreleased]: https://github.com/SuperJackfruitLabs/supermd/compare/v0.0.17...HEAD
[0.0.17]: https://github.com/SuperJackfruitLabs/supermd/releases/tag/v0.0.17
[0.0.16]: https://github.com/SuperJackfruitLabs/supermd/releases/tag/v0.0.16
