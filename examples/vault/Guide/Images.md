# Images

An image is a claimed block like a table or a diagram: it renders while
the cursor is outside it, and returns to `![alt](path)` source when you
move into it.

#guide

## A local image

![The SuperMD icon](../assets/supermd-icon.png)

The path is relative to this file. Click into the line above and the
Markdown reappears; click away and the picture comes back.

## Pasting an image

Copy any image to the clipboard and press **⌘V**. SuperMD writes it
into `assets/` beside your note, names it after the note and the time,
and inserts the link — so a pasted screenshot is a real file in your
folder, not an attachment locked inside a proprietary database.

## An image that does not exist

The link below points at a missing file. It should show as a broken
image in place, not blank the document:

![Missing on purpose](../assets/nothing-here.png)

## An image with a title and awkward alt text

![Alt text with **markers**, a [link](Editing.md), and a ] bracket](../assets/supermd-icon.png "A title in quotes")

Next: [[Plugins]]
