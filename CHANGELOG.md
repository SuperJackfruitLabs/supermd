# Changelog

Everything notable that changes in SuperMD, newest first. The shape is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[semantic versioning](https://semver.org/spec/v2.0.0.html).

Release notes are written here first. The GitHub release body is
generated from commit subjects, which for 0.0.16 produced a single line
linking a pull request and had to be rewritten by hand afterwards.

## [Unreleased]

### Added

- A visual language of surfaces: the document is a page resting on the
  ground, with page, border and shadow tokens derived for any theme that
  does not define them, and a surface of its own for everything that
  floats.
- Appearance setting — Light, Dark or System — at the top of the ⌘T
  picker. An explicit choice outranks flux, so Light stays light at
  midnight rather than flipping.
- Twenty more themes, converted from base16 rather than hand-written,
  each with a page of its own; the theme picker filters as you type.

### Changed

- The sidebar, outline and status bar read as one ground; the active tab
  is the page's own edge; the page is inset left, right and bottom.
- Overlays share one shadow vocabulary, and tables lose their interior
  vertical rules.
- Every theme token is measured against the surface it is actually
  painted on, so muted and body ink stay legible on all five.

### Fixed

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

[Unreleased]: https://github.com/SuperJackfruitLabs/supermd/compare/v0.0.16...HEAD
[0.0.16]: https://github.com/SuperJackfruitLabs/supermd/releases/tag/v0.0.16
