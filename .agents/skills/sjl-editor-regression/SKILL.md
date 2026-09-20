---
name: sjl-editor-regression
description: Reproduce and fix SuperMD editing, cursor, Unicode, table, projection, or theme regressions through its Rust and GPUI pipeline. Use for editor behavior defects and their verification, not unrelated website or documentation edits.
license: Apache-2.0
---

Identify the smallest source document, exact input sequence, selection, expected
behavior, actual behavior, product revision and host that reproduce the defect.
Read the checkout's CLAUDE.md and inspect the relevant implementation; an older
issue or screenshot is evidence of that observation, not the current binary.

## Find the owning stage

Use [the pipeline map](references/pipeline.md) to trace source bytes through
buffer operations, display mapping, projection and the GPUI shell. Keep logic in
the existing pure Rust owner where possible. A view change should drive that
logic rather than copy it. Do not introduce an LSP or change the CommonMark file
format to solve an editor defect.

For Unicode failures, preserve the exact bytes, including combining marks,
non-BMP characters, line endings and trailing spaces. Identify whether the
reported position is a UTF-8 byte offset, a character, a grapheme, a display
position or a visual column before converting it. Never use an ASCII-only
fixture to prove a Unicode boundary.

For tables, reproduce cursor position before and after alignment, navigation,
reveal/hide and the edit. For theme defects, capture the active theme, appearance
and flux conditions; changing one color without its transformation path can
leave other appearances broken.

## Reproduce, fix, verify

Add the smallest regression test at the actual behavior boundary and observe it
fail for the original defect. Assert the resulting source and selection or
mapping, rather than just the helper's return type or test count. Apply the fix
and run the relevant tests, then required repository checks. Preserve the user's
working tree while comparing before and after behavior.

If the defect depends on GPUI geometry, IME, wrapped lines, focus or rendering,
exercise that interaction on an available native host and record the observation.
A passing pure-Rust test or browser screenshot cannot establish that native
interaction. If the host or reproducer is missing, report the exact gap and
continue the source-level diagnosis that is possible.

## Deliver evidence

Report the reproducer, owning stage, change, commands and actual results, and
remaining host checks. Preserve plain Markdown and the application's read-only
Git behavior. A Cargo version change or local successful build does not prove
that a downloadable or running release contains the fix.
