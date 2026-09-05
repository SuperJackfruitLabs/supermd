# Code

Fenced blocks are highlighted with tree-sitter, on open and on edit,
never per frame. The fence delimiters stay visible and render faded —
a code block is not something you want silently folded away.

#guide #code

```rust
/// Byte offsets are the universal currency: every position, span and
/// selection in the editor is a byte offset into the rope.
pub fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0)
}
```

```python
def fibonacci(n: int) -> list[int]:
    a, b, out = 0, 1, []
    while a < n:
        out.append(a)
        a, b = b, a + b
    return out
```

```typescript
type Link = { target: string; wiki: boolean };

export function classify(link: Link): "external" | "wiki" | "relative" {
  if (link.wiki) return "wiki";
  return /^https?:\/\//i.test(link.target) ? "external" : "relative";
}
```

```go
func Sum(xs []int) int {
	total := 0
	for _, x := range xs {
		total += x
	}
	return total
}
```

```sql
SELECT note.path, count(link.id) AS backlinks
FROM note
JOIN link ON link.target = note.path
GROUP BY note.path
ORDER BY backlinks DESC
LIMIT 10;
```

```bash
#!/usr/bin/env bash
set -euo pipefail
cargo test --no-default-features --features mas
```

```json
{ "name": "supermd", "version": "0.0.15", "private": true }
```

```graphql
query NoteWithBacklinks($path: String!) {
  note(path: $path) {
    title
    backlinks { path context }
  }
}
```

## A block with no language

```
No language tag, so no highlighting — just monospace text.
  Indentation is preserved exactly.
```

## A block containing fence-like text

````
This block is opened with four backticks, so it can contain ``` inside
it without ending early.
````

Next: [[Diagrams]]
