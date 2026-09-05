# Diagrams

Mermaid is built in. Graphviz DOT arrives via the bundled `dot` plugin.
Both render on a background thread and are cached — the UI only ever
reads the cache, so a complex diagram never blocks typing.

Like tables, a diagram is a claimed block: it renders as a picture
while the cursor is outside it, and returns to source when you enter.

#guide #diagrams

## Mermaid: a flowchart

```mermaid
flowchart TD
    A[Source text] --> B[spans.rs]
    B --> C[display.rs]
    C --> D{Cursor inside?}
    D -->|no| E[Markers hidden]
    D -->|yes| F[Markers revealed]
    E --> G[Rendered line]
    F --> G
```

## Mermaid: a sequence diagram

```mermaid
sequenceDiagram
    participant U as User
    participant E as Editor
    participant K as Knowledge index
    U->>E: click a [[wiki link]]
    E->>K: resolve(target)
    K-->>E: path, or nothing
    alt resolves
        E->>U: open the note
    else does not resolve
        E->>U: create it, then open
    end
```

## Graphviz, via the `dot` plugin

```dot
digraph pipeline {
    rankdir=LR;
    node [shape=box];
    buffer -> spans -> display -> projection -> view;
    projection -> widgets [style=dashed];
}
```

## A deliberately broken diagram

The block below is not valid Mermaid. It should show an error in place
rather than blanking the document or taking the app down.

```mermaid
flowchart TD
    this is not --> valid ((mermaid
```

Next: [[Plugins]]
