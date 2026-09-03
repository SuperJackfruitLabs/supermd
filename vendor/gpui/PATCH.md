# Why gpui is vendored

App Store review rejected SuperMD 0.0.14 under Guideline 2.1 for referencing
`_CGSSetWindowBackgroundBlurRadius`, a private CoreGraphics SPI. The scan is
static: a linked symbol is enough, whether or not the code can run.

Auditing the binary turned up four private APIs in total, all from gpui —
Apple's message named only the first:

| API | Appears as |
| --- | --- |
| `_CGSSetWindowBackgroundBlurRadius` | undefined symbol |
| `_CGSMainConnectionID` | undefined symbol |
| `_windowResizeNorthWestSouthEastCursor` | selector string |
| `_windowResizeNorthEastSouthWestCursor` | selector string |

The cursor selectors are dispatched at runtime, so they never appear as
undefined symbols — only a string scan finds them. Fixing only what Apple
named would have failed the next submission.

## Changes against gpui 0.2.2

**`src/platform/mac/window.rs`** — removed the `extern "C"` block declaring
`CGSMainConnectionID` and `CGSSetWindowBackgroundBlurRadius`, and collapsed
the `NSAppKitVersionNumber < NSAppKitVersionNumber12_0` branch to its modern
arm. The removed arm only ran on macOS 11 and earlier; SuperMD requires 12.0,
where upstream already uses the public `NSVisualEffectView`. SuperMD never
requests `WindowBackgroundAppearance::Blurred`, so nothing changes at runtime.

**`src/platform/mac/platform.rs`** — `CursorStyle::ResizeUpLeftDownRight` and
`ResizeUpRightDownLeft` now use the public `resizeLeftRightCursor` instead of
the two undocumented `NSCursor` class methods. macOS exposes no public
diagonal resize cursor. With native window decorations the system draws its
own at window edges, so this is not visible in practice; the cost is a
generic cursor if the app ever sets those styles itself.

## Verifying after a version bump

```sh
nm -u <binary> | grep -E "_CGS|_SLS"                    # expect nothing
strings -a <binary> | grep -E "_windowResize.*Cursor"   # expect nothing
```

Drop this vendor entirely once upstream stops referencing private APIs.
