# Grammar Plugins

Grammar plugins add syntax highlighting for new languages — in fenced code blocks and in standalone files — with **no Rust code at all**. A grammar plugin is three files:

```
my-grammar/
  plugin.toml
  grammar.wasm      # a compiled tree-sitter parser
  highlights.scm    # tree-sitter highlight queries
```

## The manifest

```toml
name = "graphql"
version = "0.1.0"

[[grammars]]
name = "graphql"                  # the fence language token
extensions = ["graphql", "gql"]   # file extensions to claim
```

With this installed, ` ```graphql ` fences highlight, and opening `schema.graphql` highlights the whole file. The bundled GraphQL plugin is exactly this shape — [see it in the repo](https://github.com/SuperJackfruitLabs/supermd/tree/master/plugins/graphql).

## Building `grammar.wasm`

The parser comes from any tree-sitter grammar repository, compiled once with the tree-sitter CLI (0.23.x) and Emscripten:

```sh
scripts/build_grammar_wasm.sh <grammar-repo-dir> <out-dir>
```

This regenerates the parser at a compatible ABI and produces `grammar.wasm` (~40–300 KB). Users never need these tools — you build once and ship the file. One requirement: the wasm's exported symbol must match your grammar `name` (a grammar generated as `tree_sitter_graphql` must be named `graphql`).

## `highlights.scm`

Standard tree-sitter highlight queries, using the Helix capture vocabulary (`@keyword`, `@type`, `@string`, `@comment`, `@function`, `@constant`, …). If the grammar you're wrapping is used by the Helix editor, its `runtime/queries/<lang>/highlights.scm` usually works as-is — check the node names match your parser's version.

## Rules

- **Built-ins win**: a grammar named `rust` cannot shadow SuperMD's bundled Rust highlighting.
- **Several grammars per plugin**: give each `[[grammars]]` entry a `files = "<stem>"`, and ship `<stem>.wasm` + `<stem>.scm` per grammar.
- **Failures are contained**: a broken query or incompatible wasm marks the plugin failed in the load report; a grammar that misbehaves at parse time degrades that language to plain text. The editor never crashes over a grammar.
