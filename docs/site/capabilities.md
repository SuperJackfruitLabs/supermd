# Capabilities

Plugins are pure by default: no filesystem, no network, no environment, no clock. Capabilities widen that — always host-mediated, always with user consent. This page is the author-facing contract.

## Declaring capabilities

```toml
capabilities = ["workspace-read", "net"]
```

Unknown capability names reject the plugin at load, so older SuperMD versions fail your plugin clearly instead of running it half-working.

## `workspace-read`

After a one-time consent banner, the user's open workspace is mounted **read-only** at `/workspace` inside your sandbox. Standard file APIs work under that path; everything else stays invisible. Until consent is granted, your plugin's calls return a consent-shaped error and SuperMD shows the banner — after granting, the user retries the action.

## `net`

Declaring `net` gives you a host `fetch` function — your plugin never opens a socket. The host enforces:

- **Per-domain consent**: the first request to a domain returns the error `consent required: <domain>` and shows the user a banner naming that domain. Granted domains are remembered in settings and revocable. Denied domains fail quietly thereafter.
- **HTTPS only** — `http://` URLs are rejected.
- **Limits**: 5-second timeout per request, 2 MB response cap, at most 4 fetches per plugin call, redirects followed only to domains the user has granted.

Requests take a method (GET or POST), URL, headers, and optional body; responses carry status, headers, and body bytes. Propagate fetch errors as your own errors — that's what routes the consent banner.

A useful pattern: net-capable paste plugins run *after* the paste, asynchronously — the pasted text lands instantly and your replacement is applied only if the user hasn't typed in the meantime. Design for both outcomes.

## Files are never yours to place

There is no write capability. Exporters and templates return **content**; SuperMD owns every path:

- `export-document` returns files as relative paths + bytes; the user picks the destination in a save dialog. Paths containing `..` or absolute paths are rejected outright.
- `render-template` returns a workspace-relative filename + content; SuperMD validates it, creates it inside the open workspace, and never overwrites an existing file.

## Time limits and crash isolation

- Compute is capped at **2 seconds** per call; a call that exceeds it is interrupted and returns an error. Net-capable calls get additional allowance for network time, so a slow server isn't misread as a hang.
- A panic inside your plugin becomes an error result, and your plugin gets a fresh instance on the next call. You cannot take the editor down.
- Failures are always data: a failing renderer shows an error where the diagram would be; a failing formatter leaves the document untouched; a failing hook lets the save proceed.
