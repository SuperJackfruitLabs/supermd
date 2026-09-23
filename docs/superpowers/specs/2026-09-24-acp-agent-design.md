# A vault agent, over the Agent Client Protocol

**Status:** approved design, ready for an implementation plan
**Date:** 2026-09-24
**Target:** 0.0.18 (direct build), 0.0.19 (App Store build)

## What this is

SuperMD becomes an ACP client, so a coding agent — `claude-code-acp`,
`gemini`, anything speaking the protocol — can work inside a vault the
way it works inside a repository. Not as a code agent that happens to
open Markdown, but as a **vault agent**: one that knows the link graph,
the tags and the backlinks, because SuperMD tells it.

The shape of a session:

> *"Summarise everything tagged `#research` into a new note, linked from
> Index."*
>
> The agent is handed the 41 notes that carry the tag, by path. It reads
> the ones it wants through SuperMD. It proposes `Research Summary.md`
> and one line added to `Index.md`. Both arrive as diffs. Neither
> reaches disk until accepted.

## Goals

1. A vault-wide instruction like the one above works end to end.
2. Reads are free; **no write reaches disk unreviewed**.
3. The agent starts knowing the vault's structure, not discovering it by
   brute force.
4. The transport is a seam, so the App Store build can join later without
   redesigning anything above it.

## Non-goals

Decided, not deferred by accident:

- **No MCP server exposing the index.** `session/new` accepts MCP
  servers and this would be the most protocol-native way to give the
  agent vault tools. It is ruled out for now because an MCP server needs
  a process or a listening socket, and the sandbox denies listening
  (measured — see Evidence). It would work in the direct build and
  dead-end in the App Store one, which is the divergence this design
  exists to avoid. Revisit if remote transport ships and a bridge proves
  acceptable.
- **No session persistence across restarts** in v1. A session belongs to
  a window and dies with it.
- **No terminal support.** ACP defines it; SuperMD is a notes app and
  `SuperMD never writes to the user's git repository` is a standing
  constraint. An agent that cannot run commands cannot violate it.
- **No agent-authored chrome.** Plugins render content, not chrome
  (existing non-goal); the same applies here. The panel is host-built.

## Constraints

- **Editing logic is pure Rust under test; the GPUI shell stays thin.**
- **The plain-text Markdown file is the source of truth.**
- **SuperMD never writes to the user's git repository.**
- Both suites must pass: `cargo test --bin supermd` and
  `--no-default-features --features mas`. `cargo build --bin supermd` is
  not covered by them and must be run explicitly.
- CI enforces a 90% line coverage floor; tests live inline.
- The App Store build ships sandboxed with `network.client` and **no**
  `com.apple.security.cs.*` keys. Nothing in this design may require a
  JIT entitlement or a listening socket.

## Evidence this design rests on

Measured on 2026-09-24 with a signed, sandboxed probe bundle carrying
SuperMD's own entitlements. Recorded so nobody repeats it:

| Operation | Unsandboxed (DMG) | Sandboxed (App Store) |
| --- | --- | --- |
| exec `/bin/echo` | OK | **OK** |
| exec `/opt/homebrew/bin/node` | OK | **DENIED** |
| exec a binary in `~/Downloads` | OK | **DENIED** |
| child process reads `~/.zshrc` | 5656 bytes | **0 bytes** |
| connect out to localhost | OK | **OK** |
| listen on localhost | OK | **DENIED** |

The sandbox does not forbid spawning. It forbids *executing binaries it
cannot read*, which is where every agent binary lives — Homebrew, npm
globals, `~/.local/bin`. Stdio ACP is therefore impossible in the App
Store build, and remote transport is possible because outbound
connections are allowed.

Protocol facts, from the specification:

- `fs/read_text_file` and `fs/write_text_file` are **client** methods.
  The editor performs disk access; the rationale given is security
  boundaries, visibility of modifications, and access to unsaved editor
  state.
- `session/new` takes a working directory and a list of MCP servers.
- Tool calls report edits as `{"type":"diff","path","oldText","newText"}`,
  with `oldText` null for a new file.
- `session/request_permission` returns `{"outcome":{"outcome":"selected",
  "optionId":"allow-once"}}` or `cancelled` — and the agent **MAY**
  request permission. May, not must.

Ecosystem facts: Zed is not sandboxed (no `app-sandbox` key anywhere in
its repository) and is not on the Mac App Store; neither is any JetBrains
IDE. Sandboxed ACP clients do exist and pass review — *Happy*, *Agmente*
and *Shellular* on iOS, *Tailscode* on macOS — all using a remote agent
reached over the network, which is the model 0.0.19 will follow.

## Architecture

A new `src/agent/` directory rather than one file. `graph.rs` reached
2,000 lines carrying two subjects and now sits in the backlog wanting a
split; this subsystem is larger than that on its first day.

The official `agent-client-protocol` crate (2.2.0) supplies protocol
types and traits. We write only what is ours.

### `src/agent/session.rs` — the client state machine

Session lifecycle and prompt turns: what state a session is in, what a
turn contains, which tool calls and permission requests have arrived and
what became of them. Pure, driven by injected messages, tested without a
window — the arrangement `autosave.rs` uses for its policy.

### `src/agent/transport.rs` — the seam

A trait with one method pair: send a message, receive a stream of them.
One implementation in 0.0.18 (`Stdio`: spawn the agent, own its pipes —
the shape Zed's `spawn_stdio()` has). `Remote` follows in 0.0.19.
**Nothing above this file knows which is in use.** This is the only
place the two builds differ.

### `src/agent/context.rs` — the vault rule

Pure: given the prompt text, the knowledge index, the active note and
the selection, produce the content blocks attached to the prompt.

**Context is a map, not the territory.** Paths and structure go in; the
agent pulls content itself through `fs/read_text_file`. Attaching 41
notes would cost more and tell the agent less than attaching 41 paths.

What is attached:

- the workspace root and its note count;
- the active note's path, and the selected text if any;
- for every `#tag` named in the prompt, the paths of notes carrying it;
- for every `[[link]]` named in the prompt, its resolved path, or that it
  resolves to nothing;
- the active note's backlinks and outgoing links, by path.

Nothing else. It resolves what the prompt refers to, plus the
neighbourhood you are standing in. Every list is capped at **50 entries
with an explicit `…and 37 more`**, so a ten-thousand-note vault cannot
blow up a prompt.

### `src/agent/review.rs` — the gate

A pure state machine over pending writes: which are waiting, which were
accepted, which rejected. One pending write per path; a second write to
the same path before a decision replaces the first.

### `workspace.rs` — wiring only

The thread panel, the diff review surface, permission prompts, and
applying accepted writes. No policy.

## Configuration

The agent is a command, configured in `~/.supermd/settings.toml`
alongside the existing settings, not hard-coded:

```toml
[agent]
command = "claude-code-acp"
args = []
```

No command configured means no agent, which the panel says plainly. The
two agents this is developed and tested against are **`claude-code-acp`**
and **`gemini`**; anything else speaking the protocol is expected to work
and is not verified by us.

Environment inheritance is deliberate: the spawned agent inherits
SuperMD's environment, so an agent needing an API key finds it the same
way it would in a terminal. This is one more reason the App Store build
cannot take this path — it inherits a container, not a shell.

## The review gate

The protocol says the agent **may** request permission. An agent that
does not ask is not misbehaving — so a design that relies on being asked
does not deliver "nothing reaches disk unreviewed".

The enforcement point is `fs/write_text_file`, because ACP has the client
perform disk access. Every write is held in `review.rs` whether or not
permission was requested, and the response to that call is deferred until
the user decides:

- **Accept** — perform the write, return success.
- **Reject** — return an error, so the agent learns it was refused and
  can adapt rather than assuming success.

When the agent *does* send a `diff` tool call, that is what the review
renders, using the pure diff engine from Show Changes (`src/diff.rs`).
When it does not, the review renders a diff computed from the file's
current contents against the proposed text — the same engine either way.

**Accepting an edit to an open note applies it to the buffer**, so ⌘Z
takes it back and autosave saves it on its own schedule. A note not
currently open is written directly and the watcher notices, exactly as
any external edit.

`Accept all` applies to the remaining pending writes of the current turn
only. It does not persist across turns or sessions.

## Session lifecycle

One session per window, created on first prompt with the workspace root
as `cwd`, torn down when the window closes or the agent exits. A session
whose transport dies reports that in the thread and offers to restart;
it does not silently reconnect.

Cancellation follows the protocol: cancelling a turn resolves outstanding
permission requests as `cancelled` and discards pending writes.

## Error handling

- **No agent configured, or the binary is missing** — the panel says
  which command it tried and that it was not found. No modal.
- **The agent exits mid-turn** — the thread shows the turn ended and why,
  with stderr's last lines available. Pending writes are discarded.
- **A read outside the workspace** — refused, and the refusal is returned
  to the agent as an error. The workspace root is the fence.
- **A write outside the workspace** — same, and additionally surfaced in
  the thread, because an agent trying to write outside the vault is worth
  seeing rather than silently denying.
- **A file that cannot be read** (deleted mid-session) — the error is
  returned to the agent; the thread does not raise an error strip, since
  nothing the user did failed.
- **App Store build** — the panel renders and explains that agent
  sessions need the direct download until 0.0.19, naming the reason. It
  is not hidden, and it is not a dead button.

## Testing

Pure units, inline, as the codebase does:

- `context.rs`: a prompt naming a tag attaches exactly that tag's notes;
  a prompt naming none attaches only the active note's neighbourhood;
  the 50-entry cap fires with the right remainder; a `[[link]]` that
  resolves to nothing says so rather than being dropped.
- `review.rs`: a write is pending until decided; accept yields the write,
  reject yields the error; a second write to the same path replaces the
  first; cancellation discards everything pending.
- `session.rs`: turn state transitions, including a permission request
  arriving for a tool call that was never announced, and a turn that ends
  while a write is still pending.
- `transport.rs`: the trait's contract against a fake, so `session.rs`
  never needs a real process to test.

Shell-level, with the GPUI test context:

- A write from the agent does not reach disk before acceptance — the
  test that most directly pins the promise this design makes.
- Accepting an edit to an open note leaves it undoable with ⌘Z.
- The App Store build's panel renders its explanation rather than
  attempting a session.

## Success criteria

1. *"Summarise everything tagged #research into a new note, linked from
   Index"* completes against a real vault, with both writes reviewed.
2. No write reaches disk without an explicit acceptance, proven by a
   test, including when the agent never requests permission.
3. The context attached to a prompt is inspectable and bounded.
4. `src/agent/transport.rs` is the only file that would change to add
   remote transport.

## Risks

- **An agent that ignores client filesystem methods** and reads or writes
  the disk directly. Nothing in the protocol prevents it, and in the
  direct build nothing in the operating system does either. Our gate is
  then bypassed. The mitigation is honesty: document that the guarantee
  covers agents using the protocol's filesystem methods, which is what
  the protocol asks of them.
- **Context that is subtly wrong** is worse than no context, because it
  misleads silently. Hence the pure function and its tests.
- **The panel is the largest new UI surface since the graph.** It is
  host-built and self-contained, but it will grow; the backlog entry for
  splitting `graph.rs` is the cautionary precedent.
