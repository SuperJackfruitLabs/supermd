# Editing

SuperMD is a hybrid WYSIWYG editor: the file on disk is plain
CommonMark, and syntax markers stay hidden until your cursor touches
them. Nothing is ever rewritten behind your back.

#guide

## Markers fold away

Click inside this **bold phrase** and the `**` markers appear under the
cursor; click away and they fold up again. The same is true of
*emphasis*, `inline code`, ~~strikethrough~~, and [a link](Tables.md).

That is the whole idea. Put the caret in the middle of the word
**deliberately** and watch the asterisks arrive without the text
shifting away from you.

## The selection toolbar

Select a few words with the mouse. A small toolbar appears above the
selection: bold, italic, code, strikethrough, and a link button. Each
is one click, and each is a single contiguous edit — so **⌘Z** undoes
the whole toggle, not half of it.

**⌘B** and **⌘I** work on a selection too, and on a bare cursor: they
insert the markers and place the caret between them.

## Headings

### A third-level heading

#### A fourth-level heading

The `#` markers hide like any other marker, and the **outline** in the
right sidebar (**⌘2**) is built from them. Click an outline entry to
jump to it.

## Lists

- A bullet, whose `-` is replaced by a typographic dot
- Another one
  - Nested one level
  - And a sibling
- Back to the top level

1. Ordered lists renumber as you type
2. Press Enter at the end of this line to continue the list
3. Press Enter on an empty item to end it

- [ ] An unchecked task — click the box
- [x] A checked one — click it again to uncheck
- [ ] **⌘Z** takes back a checkbox toggle like any other edit

## Quotes and rules

> A block quote renders with a rule down its left edge rather than a
> literal `>` on every line.
>
> It can hold multiple paragraphs.

---

That horizontal rule above is three hyphens in the file.

## Long lines wrap

This paragraph is deliberately long so that it wraps across more than one visual line, which is worth having in an example vault because wrapped lines are where cursor movement, selection, and the display-to-buffer offset mapping are most likely to go wrong, and a single short paragraph would never exercise any of that.

## Escapes and edge cases

Literal asterisks: \*not bold\*. A backslash at end of line forces a\
hard break. Inline code can contain markers without them rendering:
`**not bold**`, `[[not a link]]`.

An empty list item, a trailing space, and a tab follow — all things
that have broken editors before:

-
- 	tabbed content

Next: [[Links and notes]]
