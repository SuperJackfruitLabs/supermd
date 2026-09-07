# Links and notes

SuperMD indexes the folder you open. There is no database — the files
are the truth, and the index is rebuilt from them.

#guide #links

## Wiki links

Type `[[` anywhere and every note in the folder is a completion away.

- [[Editing]] — a plain wiki link
- [[Tables|the table guide]] — a labelled link; the file is `Tables`,
  the words you read are "the table guide"
- [[Notes/Meeting 2026-09-02]] — a link into a subfolder

Click one to follow it. **⌘[** goes back, **⌘]** goes forward.

## Links to notes that do not exist yet

[[A note nobody has written]] resolves to nothing. Clicking it creates
the file next to this one and opens it — the fastest way to write your
way outward into a new topic.

## Links to files that are not notes

A link does not have to point at Markdown:

- [The Rust sample](../Code/sample.rs)
- [The TOML config](../Code/config.toml)
- [The CSV data](../Data/quarterly.csv)

These open in the viewer-plus editor rather than the Markdown view.

## External links

- [The SuperMD site](https://supermd.app)
- [The repository](https://github.com/SuperJackfruitLabs/supermd)

An `http://` or `https://` address opens in your browser. Anything
else — `mailto:`, `file://`, a bare path — is treated as a path inside
the workspace, so a document can never talk the editor into opening
something outside the folder you opened.

## Hover to see what is on the other side

Rest the pointer on any link above for a moment. A note shows its title
and opening lines, a code file its first lines, a link to a note that
does not exist says so.

An external link shows its destination domain and full address — and
warns you when the visible text names a *different* site than the link
actually goes to, like [supermd.app](https://example.test/not-supermd)
above. That check is local: nothing is requested to make it.

Nothing is fetched until you say so. Hovering an external link never
reaches the network; the popover offers **Enable previews for this
site**, one site at a time, and only that click makes a request.

## Tags

Tags are just `#words` in the text. This note carries `#guide` and
`#links` at the top. The tag index is live: add one now and it appears
in the panel immediately.

Some more, so the graph has something to cluster on: #markdown #rust
#editor

## Backlinks and the graph

Press **⌘3**. The panel shows every note linking *to* this one, with
the line each link sits on for context, plus a live force-directed
graph of the neighbourhood. The layout is deterministic — the same
vault always draws the same graph.

[[Editing]] and [[README]] both link here, so both should appear.

## Renaming

Rename this file in the sidebar. Every link pointing at it — in every
other note — is rewritten to match, and any open tab retargets to the
new path. Nothing is left dangling, and SuperMD never touches your git
repository to do it.

Next: [[Tables]]
