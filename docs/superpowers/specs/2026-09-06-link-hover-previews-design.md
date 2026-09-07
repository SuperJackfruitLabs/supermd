# Link hover previews — design

**Status:** approved, for 0.0.15
**Date:** 2026-09-06

Hovering a link shows what is on the other side of it, before you commit
to going there. The link work in 0.0.15 made links follow on a plain
click; this makes them legible before the click.

## Decisions already taken

- **Trigger:** hover with a ~400 ms dwell. Moving away closes and
  cancels. No modifier for the common case.
- **External links fetch, but only after consent**, per domain, reusing
  the plugin system's existing grant store and policy.
- **Everything ships in 0.0.15**, including the privacy-policy
  amendment.

## Why not a badge, and why not fetch-on-hover

Two rejected alternatives, recorded so they are not re-proposed:

*Rendering the language as a badge* was rejected for the sibling
fence-delimiter problem: a badge is app chrome duplicating what the
document already says. The source line is the honest answer.

*Fetching on hover alone* is rejected because hovering is not consent.
A document is untrusted content; if the pointer passing over a link
could reach a server, a hostile note becomes a tracking pixel. The
first request to a domain always follows a click on an explicit
affordance.

## What each link kind previews

| Link | Preview |
| ---- | ------- |
| `[[Note]]`, resolved | Title and first lines, rendered |
| `[[Ghost]]`, unresolved | "Does not exist — click to create" |
| Relative `.md` | As a resolved note |
| Relative code/config | First lines, highlighted |
| Relative image | Thumbnail |
| Relative `.csv` | First rows as a table |
| `#heading` | The heading and its first lines |
| External, ungranted | Domain, full URL, mismatch warning, and an
  "Enable previews for this site" action |
| External, granted | Fetched title and description, cached |
| External, denied | Domain and URL only; no action offered |

Internal previews never touch the network and need no permission
beyond the workspace the user already opened.

## The phishing case

An external preview always shows the **destination domain**, and warns
when the link's visible text names a different domain than its target
(`[paypal.com](https://evil.example)`). This check is local, needs no
network, and is the one part of the external preview that works
without consent.

## Consent and fetching

Reuse, do not reinvent:

- Grants live in `Settings::plugin_grants` under the reserved key
  `supermd` — the pseudo-plugin name already used for built-in
  commands — as `net:<domain>` and `denied:net:<domain>`, exactly the
  format `url-title` uses.
- Fetching obeys the same policy `extensions::host_fetch` enforces:
  HTTPS only, per-domain grant checked again on every redirect hop,
  bounded hops, a response size cap and a time budget.
- Results are cached by URL for the session and dropped when the
  pointer leaves. A slow site must never stall a popover that is gone.
- `com.apple.security.network.client` is already in
  `assets/mas.entitlements`; no entitlement change is needed.

## Performance

`Index::link_at` costs ~7.65 ms on a 1 MB document. Running it per
mouse-move would drop frames on every pointer movement, so link ranges
are cached per document revision and invalidated where spans and
highlighting already are. Hit-testing a hover is then a binary search
over sorted ranges.

## Structure

Pure and tested, in `src/preview.rs`:

- `Preview` — what to draw, one variant per row of the table above.
- `excerpt(text, lines)` — the first meaningful lines of a document,
  skipping front matter and leading blanks.
- `domain_of(url)`, `text_target_mismatch(text, url)` — the local
  external checks, including the phishing warning.
- `HoverState` — a state machine over injected time: `moved_to`,
  `left`, `poll(now) -> bool`. No timers in the pure layer.

The GPUI shell owns only the popover element, the mouse handlers, and
the fetch task.

## Privacy policy

`site/privacy/index.html` says every connection is "the direct result
of an action you took" and lists three. A fourth row is added — link
previews, per site, after you enable them — and the sentence stays
true because a grant is a deliberate act. This ships in the same
release as the feature, not after it.

## Testing

- Pure: every `Preview` variant, the excerpt rules, the mismatch
  detector, and the hover state machine over injected time.
- Editor: hovering a link opens the popover; leaving closes it; an
  ungranted domain performs no fetch; a denied domain offers no action.
- The fetch path is driven through an injected transport, as the
  extension-host tests already do, so no test touches the network.
