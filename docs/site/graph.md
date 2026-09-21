# The Graph

**⌘ ⇧ G** turns the whole workspace into a map: one dot per note, one line per link, laid out by a force simulation that runs in front of you and settles. It is also in **Go → Graph View** and in the command palette as *Graph View*. It needs a folder open: on a single file there is nothing to draw, and SuperMD says so rather than showing you an empty board.

Drag the background to pan, scroll to zoom, **⌘ 0** to fit the whole vault back into the window. Drag a dot to move it; the layout reflows around where you put it, and lets go when you do.

## Reading the map

**Size is link count.** A dot grows with the number of links into and out of its note, so the notes everything hangs off are the big ones. Nothing configures this — it is just degree.

**Colour is grouping.** By default notes are coloured by their top-level folder, so a vault with `Projects/`, `Journal/` and `Reference/` reads as three coloured regions. **⌘ G** cycles the colour meaning: folder → first tag → nothing (one colour for everything). Group colours come from your theme's palette and are assigned in sorted order, so a folder keeps its colour between sessions instead of depending on which note was indexed first.

Two dots are coloured for what they are rather than where they live:

- **The note you have open** takes the theme's link colour, so you can always find where you are.
- **A note that does not exist yet** — something links to `[[Roadmap]]` and there is no `Roadmap.md` — is drawn hollow. These are your to-write list, and the graph is where they are easiest to see. Clicking one creates the note beside the note that referenced it, exactly as following the link would.

**Unlinked notes are dimmed, not hidden.** A note nothing links to and that links to nothing keeps its dot, at a little under half opacity, out at the edge where the simulation leaves it. Hiding orphans would mean the map quietly disagreed with the folder; dimming them means you can still see how many there are, and go fix it. **⌘ ⇧ O** shows *only* them.

Zoomed out, names and arrowheads go away — at the whole-vault zoom a name is unreadable and an arrowhead is smaller than a pixel. Names fade in from about half zoom; arrowheads come back at the same point.

## Hovering names things

Rest the pointer on a dot and it lights up together with everything one hop from it, while the rest of the vault dims. The lit notes are named, at any zoom — that is how you read a dot in a view too dense to label.

Keep resting, and after a moment a card appears beside the dot: the note's title, the first few lines of it, and its link counts as *n out · n in*. The card waits for you on purpose. Sweeping the pointer across a cluster reads nothing off disk; only actually stopping on a note does, and stopping on the same note again is free. A hollow dot says it does not exist yet; a note deleted since the index last ran says it could not be read, on the card rather than as an error.

## Clicking peeks

Clicking a dot does **not** leave the graph. A panel opens on the right describing that note — its title, its excerpt, and every note that links to it:

- **Open** opens the note in a tab and puts the graph away.
- **Any backlink row** moves the panel to *that* note, so you can walk backwards through the vault a hop at a time without losing the view. The board pans to bring each note into sight as you go, and the zoom stays where you set it.

The board narrows by the panel's width while it is open, so nothing you might want to click sits underneath it.

## Narrowing

**⌘ F** opens a search box: type, and matching notes stay bright while the rest fade. A filter never removes a dot, because the layout would jump under you with every character — the shape you were reading stays put while matches light up. A strip along the bottom says what is currently narrowing the view and how many notes match.

The rest of the controls, all while the graph has focus:

| Key | What it does |
| --- | --- |
| **⌘ 0** | Fit the graph to the window |
| **⌘ G** | Colour by folder / tag / nothing |
| **⌘ F** | Search the graph |
| **⌘ ⇧ O** | Unlinked notes only |
| **⌘ L** | Narrow to what is near the dot under the pointer (or the note you have open), and back out to the whole vault |
| **⌘ =** / **⌘ -** | One hop further out / nearer in, once narrowed |
| **⌘ E** | Spread: tight / normal / loose |
| **⌘ .** | Freeze the layout, or let it run again |

## Escape, in order

**Esc** closes the most recent thing first, never more:

1. the peek panel, if one is open;
2. the search box and any filter;
3. the graph itself.

So escaping a note you were peeking at leaves you on the board you were exploring, and it takes one more Esc to put the board away.

## The view is remembered

Opening a note from the graph puts the view away rather than throwing it out. Come back with **⌘ ⇧ G** and you land on the layout you left — same pan, same zoom, same filter, every dot where you dragged it — instead of watching the vault re-simulate into a different arrangement.

It is remembered only while it still describes your notes. If the vault changed underneath — a note added or deleted, a link written or removed, a file renamed — the layout is rebuilt, because a map that shows a note that is gone, or misses one that arrived, is worse than starting over.

The links and tags the graph is drawn from are [Links, Tags & Graph](knowledge.md).
