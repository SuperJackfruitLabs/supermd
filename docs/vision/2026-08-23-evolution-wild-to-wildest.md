# SuperMD: Wild → Wildest

**Date:** 2026-08-23
**Status:** Pure ideation. Nothing here is committed to a roadmap.

---

## The thesis: what SuperMD actually has

Before the wild stuff, the sober part — because the good ideas all fall
out of one architectural fact.

`editor/projection.rs` computes this:

```rust
project(lines, blocks, selection) -> Vec<Item>
```

A block becomes a **live widget** if the cursor isn't touching it, and
**dissolves back into plain source** the moment you look at it. Today
`Item` has three variants: `Line`, `Table`, `Image`.

That's the whole magic trick, and it's a *general-purpose primitive that
nobody else in the market has built*:

- **Notion** has live objects, no plain text underneath.
- **Obsidian** has plain text, but reading and editing are modes.
- **Typora** blends them, but there's no bidirectional projection layer
  you can extend.
- **Org-mode** conceptually has it, and looks like 1987.

SuperMD has *attention-mediated bidirectional projection over plain
CommonMark*, on a GPU, in a pure-Rust core under test. Three variants of
an enum stand between today and a very strange future.

Second fact: `editor/display.rs` calls itself "the ONE place SuperMD's
buffer-offset == rendered-offset invariant breaks, on purpose," and
already has `SegKind::Replacement`. That's the same trick at *inline*
granularity, already shipping (checkboxes, bullets).

Third fact: the core (buffer, selection, undo, spans, display,
projection, blocks, find) is pure Rust with no GPU dependency. The shell
is thin and swappable.

**So the frame for everything below is one question:**

> What else can a line of plain text become when you're not looking at
> it — and what happens when the thing it becomes can *compute*?

---

## Tier 1 — Wild

*The projection seam, pushed. All of these fit the existing engine.*

### 1. The `Item` registry

**Wild:** ▮▮▯▯▯

Turn `enum Item` into a trait + registry. A projector declares "I claim
this block kind, here's my widget, here's my dissolve rule." Every idea
below becomes a plugin instead of a fork. This is the load-bearing
refactor — do it once, and the next twenty ideas cost a weekend each.

### 2. Math that isn't an image

**Wild:** ▮▮▯▯▯

`$$\int_0^1 x^2\,dx$$` renders as real typeset math — GPU text runs, not
a rasterized PNG from a LaTeX server. Touch it, get your source back.
Nobody has done native GPU math typesetting in an editor. It would
simply be the best-looking math on macOS.

### 3. Diagrams as first-class blocks

**Wild:** ▮▮▯▯▯

` ```mermaid `, ` ```dot `, ` ```seq ` render as vector diagrams drawn
directly into the GPU scene — pan, zoom, click a node to jump to the
source line that made it. The diagram *is* the code; there is no export
step and no `.png` to go stale in `docs/assets/`.

### 4. Soulver inside prose

**Wild:** ▮▮▮▯▯

A line starting `=` becomes a calculator:

```
= 1200 * 0.08 * 3      →  288
= 45 min + 2h 10 min   →  2h 55m
= 12 GB / 340 MB       →  36.1
```

On disk: `= 1200 * 0.08 * 3`. On screen: the answer. Touch it, edit the
formula. Every planning doc, every estimate, every budget note in your
workspace becomes live — and stays a plain text file you can grep.

### 5. **Tables that are a spreadsheet**

**Wild:** ▮▮▮▮▯

Tables already project to widgets. Give cells formulas:

| Item      | Qty | Unit | Total       |
| --------- | --- | ---- | ----------- |
| Widgets   | 12  | 4.50 | `=B2*C2`    |
| Gadgets   | 3   | 22.0 | `=B3*C3`    |
| **Sum**   |     |      | `=SUM(D2:D3)` |

Rendered: computed numbers. Touched: raw pipes and formulas. On disk:
a bog-standard GitHub-flavored pipe table that renders fine everywhere
else and **diffs cleanly in git**.

This is the single most commercially interesting idea in this document.
"A spreadsheet you can code-review" is a product on its own, and SuperMD
would get it almost free from machinery that already ships.

### 6. Executable fences → kill the `.ipynb`

**Wild:** ▮▮▮▮▯

A `python` / `js` / `sql` / `sh` fence gets a run affordance. Output
lands in a sibling ` ```output ` fence directly below it.

Consequences, in order of increasing delight:

- The notebook file format is now **just Markdown**.
- No JSON. No base64 image blobs. No `nbstripout` pre-commit hook.
- Notebook diffs are *readable*. Notebook merges *work*.
- A PR review of a notebook is a normal PR review.

Jupyter's format is the worst-loved file format in data science. Nobody
has credibly replaced it because everyone tried to build a better
notebook instead of noticing the file was the problem.

### 7. Transclusion

**Wild:** ▮▮▮▯▯

`![[architecture.md#projection]]` projects a live view of another file's
block, inline, editable in place, writing through to the source. Touch
it and it's a plain link again. Multi-file documents that are still,
individually, single plain files.

### 8. Time as a scrub bar

**Wild:** ▮▮▮▮▯

`~/.supermd/backups` and atomic autosave already give you a history
substrate; git is right there for the rest. So: `⌘⇧←` and the document
**morphs backwards through its own history** — per-character, at 120fps,
words dissolving back to earlier words.

Not a diff view. Not a sidebar. The document *becoming its past*, as an
animation. This is a 10-second demo video that gets 4 million views and
does more for the product than a year of feature work.

---

## Tier 2 — Wilder

*The file stops being a file.*

### 9. The workspace is a database, and the query is a fence

**Wild:** ▮▮▮▮▯

```query
from **/*.md
where tag:#todo and not done
order by mtime desc
```

...renders as a live, **editable** list. Check a box in the query
result, and the write goes through to the file the item actually lives
in. Now every project doc can host its own live dashboard, and the
dashboard is a plain text file.

Dataview proved the demand. Nobody has built it native, fast, and
write-through.

### 10. Every block has an identity

**Wild:** ▮▮▮▯▯

Content-addressed stable IDs for headings and blocks (hash + path,
carried in a way that plain Markdown doesn't notice). Once blocks are
addressable, transclusion, queries, comments, backlinks, and
block-level merge all become the same feature.

### 11. Continuous zoom: no graph view, a *space*

**Wild:** ▮▮▮▮▯

Every "graph view" ever shipped is a decorative hairball you look at
once. Build the other thing: a force-directed spatial canvas where nodes
are **actual rendered document previews**, and zoom is *continuous* —
pull back to see the constellation of your workspace, push in until a
node fills the frame and you're simply typing in it. No mode switch, no
"open" action. One gesture from galaxy to glyph.

The GPU makes this possible. Nobody with a GPU renderer has bothered.

### 12. Local semantic memory, zero cloud

**Wild:** ▮▮▮▮▯

Embeddings computed on-device on Metal. No account, no server, no
telemetry — *this is the whole point*. Then:

- Search that finds "the bit where I argued about the amber palette"
  without you remembering a single word of it.
- A quiet margin that shows the three things **you already wrote** that
  relate to the sentence you're writing right now.

Call it **Ghostwriter of One**: your corpus autocompletes you. Nothing
is generated. Everything shown is something you already said. In 2026
this is a more interesting product than another LLM sidebar, and it's a
privacy story you can put on a billboard.

### 13. Peer-to-peer multiplayer with no server

**Wild:** ▮▮▮▮▯

Block-level CRDT over the existing block model. Cursor presence. But the
wild part is the transport: **mDNS + QUIC on the local network**. Two
people in a café co-edit a file with no account, no signup, no cloud,
no company in the middle. The document never leaves the room.

Every collaborative editor is a SaaS. This one is a *protocol*.

### 14. Version control a writer can use

**Wild:** ▮▮▮▮▯

Every autosave is a commit on a shadow branch. Conflicts don't open a
three-way merge tool — they **surface as projection**: both versions
render inline, side by side, in the flow of the document, and you
resolve by clicking the one you meant. Git for people who will never
type `git`.

### 15. The editor is the renderer is the website

**Wild:** ▮▮▮▮▯

There is already a Cloudflare account wired into this repo and
`supermd.app` on Pages. So: `⌘⇧P` publishes the open document to a URL,
rendered by *the same typography engine*, with *the same theme*, so the
web page is byte-for-byte what you were just looking at.

"WYSIWYG" has been a lie since 1984 — the editor renders one way and the
output renders another. SuperMD is the first tool that could make it
literally true, because it owns both ends. That's a positioning
statement, not a feature.

### 16. Markdown as an application format

**Wild:** ▮▮▮▮▮

Stack the above: executable fences + live queries + form inputs +
diagrams. A `.md` file is now a small application. Ship a tiny
`supermd-view` runtime, and a document can be *double-clicked and run*.

Markdown as HyperCard. Your deploy runbook doesn't *describe* the
deploy — the fence in it **is** the deploy, which means the docs
physically cannot drift from the system they document.

---

## Tier 3 — Wildest

*The editor stops being an editor.*

### 17. Text as a physical material

**Wild:** ▮▮▮▮▮

You have a GPU and per-glyph control and nobody else in this category
does. So spend it on the thing users can *feel*:

- Ink that bleeds a hair into the fibre on the Paper theme.
- Letterpress deboss — a sub-pixel shadow that makes glyphs sit *in* the
  page rather than on it.
- Phosphor bloom and scanline persistence on a terminal theme.
- Foxing and edge-warmth on aged-paper themes.

Not skeuomorphic kitsch — **a material simulation on a 0.0–1.0 dial**,
default 0.15, off if you like. No editor has had the frame budget to try
this. It would be, unambiguously, the most beautiful text on any screen,
and beauty is a moat in a writing tool.

### 18. Typography with a nervous system

**Wild:** ▮▮▮▮▯

`Theme` already carries `body_size`, `body_line_height`, `body_family` —
they're just not themable yet. Take them much further than "themable":

- Measure and leading that *breathe* with the semantic density of the
  paragraph you're in.
- Real hanging punctuation and optical margin alignment.
- Variable-font optical-size axis tracking your zoom level continuously,
  so type is correct at every scale rather than at one.

This is the Bear/Lettera DNA taken somewhere those apps structurally
cannot follow.

### 19. Flow protection

**Wild:** ▮▮▮▮▯

Measure typing cadence. While you're above the threshold, the app
**suppresses everything**: no squiggles, no autocomplete, no
notifications, tabs and chrome fade to nothing, the window becomes a
column of text in the dark. Pause for four seconds and the world comes
back.

Focus mode today is a dimmer switch. This is an interface that can tell
whether you're thinking.

### 20. The infinite margin

**Wild:** ▮▮▮▮▮

Documents are linear. Thought is not. Give the editor a second
dimension: an **infinite canvas beside the column** where you pin
fragments, cut paragraphs, screenshots, reference quotes, notes-to-self,
the sentence you can't place yet. Drag from margin to column to promote
something into the document.

On disk it's a sidecar or a comment block — the `.md` stays pristine.

Every serious writer improvises this with a second file called
`scratch.md`. Nobody has ever shipped it as a real surface.

### 21. The shape of a document

**Wild:** ▮▮▮▮▯

A minimap that shows not text but **rhythm**: paragraph mass, sentence
length variance, heading cadence, dialogue density. Writers edit music
as much as meaning, and the instrument for it has never existed.

Zoom out and you can *see* that chapter four sags.

### 22. The adversarial reader

**Wild:** ▮▮▮▮▮

Not suggestions. Not rewriting. A model reads over your shoulder and
marks **the exact sentence where it lost the thread** — a confusion
heat-map down the gutter. It never proposes a fix. It never touches your
text. It just tells you where a reader stumbled.

Every AI writing tool tries to write for you. The valuable one tells you
where you failed and then shuts up.

### 23. Provenance as a product feature

**Wild:** ▮▮▮▮▮

Every character knows its origin: typed, pasted, dictated, or generated.
Toggle a view and authorship washes across the document as a subtle
tint. Export a provenance manifest. Sign it with a hardware key.

In 2026, **"written by a human" as a verifiable claim** is the strongest
possible differentiator for a writing tool, and SuperMD's local-first,
plain-file, no-cloud posture is the only credible place to build it.
This is the idea in this document with the shortest path from "feature"
to "movement."

### 24. Write with your eyes closed

**Wild:** ▮▮▮▮▯

Local Whisper on Metal, but structure-aware: "heading two, the roadmap"
produces `## The roadmap`, not the literal words. And the inverse — a
pure-audio review mode where document structure is *audible* (headings
pitched differently, code read in a different voice), so you can edit a
draft on a walk.

Accessibility as a first-class writing mode rather than a compliance
checkbox, and the only Markdown editor you can use without a screen.

### 25. Agents that behave like the rest of the app

**Wild:** ▮▮▮▮▮

The wildest framing in this document, and the most SuperMD-native:

**An agent doesn't chat. It edits. And its edits obey the reveal rule.**

Proposed changes arrive as projection items — you *accept them by
ignoring them* and *reject them by touching them*, exactly like every
other widget in the app. AI suggestions that dissolve when you look at
them. Mechanically consistent with the entire design language, and
completely unlike the chat sidebar every competitor bolted on.

Pair it with an **MCP server inside the editor**, exposing the open
workspace so an agent reads and writes your notes *while you watch it
happen live on screen*. Not a copilot in a box — a second cursor.

---

## Tier 4 — Unhinged (and yet)

### 26. Settings are a document

**Wild:** ▮▮▮▯▯

No preferences window. Ever. `⌘,` opens a Markdown document with live
tables and checkboxes that *is* the settings file. Themes: a document.
Keybindings: a document. The app configures itself in its own medium,
and the config UI costs zero code because the editor already renders it.

Deeply on-brand, faintly insane, extremely cheap.

### 27. Publish the latency number

**Wild:** ▮▮▮▮▯

Speculative glyph rasterization: draw the character before the event
loop has finished agreeing that you typed it. Then ship `⌘⇧L`, a HUD
showing **keystroke → photon in milliseconds**, and put the number on
the website next to everyone else's.

"The lowest-latency text surface on macOS" is a claim you can *measure*,
and a benchmark is a marketing asset that compounds.

### 28. The document as an executable time capsule

**Wild:** ▮▮▮▮▮

Export any `.md` to a single self-contained ~3 MB binary that renders
itself — fonts embedded, engine embedded, zero dependencies. Openable
and pixel-identical in 2056. Anti-bit-rot publishing.

The web cannot promise this. A PDF renders but can't compute. This can
do both.

### 29. Multiplayer with the dead

**Wild:** ▮▮▮▮▮

Load a public-domain corpus. A margin voice shows how Hemingway would
have cut your sentence — by *retrieval and juxtaposition*, not
generation. Not "rewrite in the style of." Just: here is a real sentence
by a real master doing the job yours is failing at.

### 30. Your phone is a window, not a copy

**Wild:** ▮▮▮▮▯

The core is pure Rust with no GPU dependency, so it ports. Your iPhone
becomes a live second window onto the same buffer over the local
network — a capture surface and a reading view. No sync service, no
conflict resolution, no cloud, no subscription. The file never leaves
your LAN.

### 31. The covenant, kept

**Wild:** ▮▯▯▯▯ · **Importance:** ▮▮▮▮▮

Every idea above must obey the line already in the README:

> **Plain CommonMark on disk, always.**

Formulas are cell text. Notebook output is a fenced block. Provenance is
a sidecar. Transclusion is a link. Query results are computed, never
written back as noise. Any file SuperMD touches must open perfectly in
`cat`, in GitHub, in `vim`, in 2050.

Being the one editor whose superpowers **cost the file nothing** is not
a constraint on the wild ideas. It is the reason they'd be trusted.

---

## If I could only build three

- [ ] **The `Item` registry** (#1) — the refactor that turns every other
      idea from a fork into a plugin. Nothing else compounds like this.
- [ ] **Spreadsheet tables** (#5) — the most product-shaped idea here,
      nearly free from machinery that already ships, and "a spreadsheet
      you can code-review" sells itself.
- [ ] **Time as a scrub bar** (#8) — the demo that makes people
      download. Ten seconds of video, and the history substrate is
      already on disk.

Then, when there's oxygen for a bet: **provenance** (#23) or
**agents that dissolve when you look at them** (#25). Those are the two
that make SuperMD a *position* rather than a better text editor.

---

*Written against SuperMD 0.0.2 — 10k lines of Rust, 78 languages, six
themes, and one very good idea in `projection.rs`.*
