# Tables

A table renders as a real widget while your cursor is outside it, and
turns back into pipe-and-hyphen source the moment you enter — the same
reveal rule as every other marker, applied to a whole block.

#guide

## A plain table

| Feature        | State    | Notes                          |
| -------------- | -------- | ------------------------------ |
| Wiki links     | Shipping | Completion, backlinks, rename  |
| Tables         | Shipping | This widget                    |
| Diagrams       | Shipping | Mermaid and Graphviz           |
| Agent layer    | Planned  | 0.1.0                          |

Put the caret in a cell. **Tab** moves to the next cell, **⇧Tab** to
the previous, and Tab in the last cell of the last row adds a new row.

## Alignment

The colons in the delimiter row control alignment, and the widget
honours them.

| Left    | Centre  | Right |
| :------ | :-----: | ----: |
| alpha   | beta    |  1.00 |
| gamma   | delta   | 22.50 |
| epsilon | zeta    | 333.75|

## Ragged source still renders

The source below is not aligned at all. It is still a valid table, and
still renders as one — realigning the pipes is something you ask for,
not something that happens to your file while you type.

| a | b | c |
|---|---|---|
| 1 | 2 | 3 |
| a much longer cell | x | y |

## Awkward content

| Case | Example |
| ---- | ------- |
| Inline code | `let x = 1;` |
| A link | [Editing](Editing.md) |
| Emphasis | **bold** and *italic* |
| An empty cell | |
| Unicode | café · 日本語 · 🌍 |

Next: [[Code]]
