//! The knowledge index: every note's outgoing links and tags, kept in
//! memory and rebuilt incrementally. Files on disk stay the only truth
//! — this is a cache the watcher keeps warm.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One outgoing link occurrence in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLink {
    /// Target as written: `Note` from `[[Note]]`, `sub/other.md` from
    /// a standard link.
    pub target: String,
    /// True for `[[wiki]]`, false for `[text](path.md)`.
    pub wiki: bool,
    /// Byte range of the whole link in the note's text.
    pub range: std::ops::Range<usize>,
    /// The line the link sits on, trimmed — backlink context.
    pub context: String,
}

/// What a link points at. Classification happens before resolution
/// because the index can only answer for workspace files — a URL has no
/// path to look up, and joining it onto the workspace root produces
/// nonsense like `<root>/https:/apple.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An http(s) URL, opened by the platform.
    External(String),
    /// `[[Note]]` — resolved by stem against the index.
    Wiki(String),
    /// A path relative to the containing file.
    Relative(String),
    /// `#heading` — a position inside the document already open, not a
    /// path. The `toc` plugin writes a page of these.
    Anchor(String),
}

/// Sort a link into one of the three kinds.
///
/// Only http and https are treated as external. Other schemes stay
/// relative and therefore fail to resolve, which is deliberate: a
/// document is untrusted content, and `file://` or `supermd://` must not
/// become a one-click action.
pub fn classify(link: &RawLink) -> LinkTarget {
    if link.wiki {
        return LinkTarget::Wiki(link.target.clone());
    }
    if let Some(anchor) = link.target.strip_prefix('#') {
        return LinkTarget::Anchor(anchor.to_string());
    }
    let lower = link.target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        LinkTarget::External(link.target.clone())
    } else {
        LinkTarget::Relative(link.target.clone())
    }
}

#[derive(Debug, Default, Clone)]
pub struct NoteData {
    pub links: Vec<RawLink>,
    pub tags: Vec<String>,
}

/// Workspace-wide index keyed by absolute path.
#[derive(Default)]
pub struct Index {
    pub root: PathBuf,
    notes: BTreeMap<PathBuf, NoteData>,
}

/// One workspace's index, shared between the workspace entity and the
/// editors and readers it owns. Every window has its own; the handle is
/// created once per workspace and its *contents* are replaced when the
/// folder changes, so anything holding a clone stays current.
pub type KnowledgeHandle = std::sync::Arc<std::sync::Mutex<Index>>;

/// Test-only carrier for a [`KnowledgeHandle`]: the editor and reader
/// test helpers install one so they can hand an editor the same handle
/// a workspace would. **Never a production global** -- the index is
/// per-workspace state (`Workspace::knowledge`), and a process-wide one
/// is exactly the bug that made a second window show the first
/// window's backlinks. The `cfg(test)` gate is what keeps it that way.
#[cfg(test)]
#[derive(Clone)]
pub struct KnowledgeState(pub KnowledgeHandle);
/// GitHub's heading slug: alphanumerics lowercased, spaces and hyphens
/// become hyphens, everything else is dropped. Must match the `toc`
/// plugin's `slug`, since that is what writes the anchors people click.
pub fn heading_slug(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Byte offset of the heading `anchor` names, or None. Fenced code is
/// skipped, and an ATX heading needs whitespace after its `#` run —
/// without that a tag line like `#guide` counts as a heading.
pub fn heading_offset(text: &str, anchor: &str) -> Option<usize> {
    let wanted = anchor.to_ascii_lowercase();
    let mut in_fence = false;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            offset += line.len();
            continue;
        }
        if !in_fence {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            if (1..=6).contains(&hashes)
                && matches!(trimmed[hashes..].chars().next(), Some(' ' | '\t'))
                && heading_slug(trimmed[hashes..].trim()) == wanted
            {
                return Some(offset);
            }
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
impl gpui::Global for KnowledgeState {}

/// Extract wiki + markdown links. Fenced code blocks and inline code
/// are skipped; `[[Target|label]]` yields `Target`; only relative
/// `.md` targets count for standard links.
///
/// This feeds the note index (backlinks, rename rewriting), which only
/// tracks note-to-note links — an external URL or a non-Markdown file
/// is not a note relationship. To find *any* link under the cursor
/// (for follow-link), use [`extract_all_links`] instead.
pub fn extract_links(text: &str) -> Vec<RawLink> {
    scan(text, true)
}

/// Extract every `[[wiki]]` and `[text](target)` link, whatever the
/// target — external URLs and non-Markdown relative paths included.
/// Used to find the link under the cursor; the note index itself
/// wants the narrower [`extract_links`].
pub fn extract_all_links(text: &str) -> Vec<RawLink> {
    scan(text, false)
}

fn scan(text: &str, notes_only: bool) -> Vec<RawLink> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut line_start = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            in_fence = !in_fence;
            line_start += line.len();
            continue;
        }
        if !in_fence {
            scan_line(trimmed, line_start, notes_only, &mut out);
        }
        line_start += line.len();
    }
    out
}

/// Links on one line, honoring inline-code spans. `notes_only` gates
/// standard `[text](target)` links to relative `.md` targets — the
/// shape the note index cares about; wiki links are always captured.
fn scan_line(line: &str, line_start: usize, notes_only: bool, out: &mut Vec<RawLink>) {
    let bytes = line.as_bytes();
    let context = line.trim().to_string();
    let mut i = 0;
    let mut in_code = false;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                in_code = !in_code;
                i += 1;
            }
            _ if in_code => i += 1,
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                // [[Target]] or [[Target|label]]
                if let Some(end) = line[i + 2..].find("]]") {
                    let inner = &line[i + 2..i + 2 + end];
                    let target = inner.split('|').next().unwrap_or("").trim();
                    if !target.is_empty() {
                        out.push(RawLink {
                            target: target.to_string(),
                            wiki: true,
                            range: line_start + i..line_start + i + 2 + end + 2,
                            context: context.clone(),
                        });
                    }
                    i += 2 + end + 2;
                } else {
                    i += 2;
                }
            }
            b'[' => {
                // [text](target)
                if let Some(close) = line[i + 1..].find(']') {
                    let after = i + 1 + close + 1;
                    if bytes.get(after) == Some(&b'(') {
                        if let Some(paren) = line[after + 1..].find(')') {
                            let target = line[after + 1..after + 1 + paren].trim();
                            let is_note_link = target.ends_with(".md") && !target.contains("://");
                            if !target.is_empty() && (is_note_link || !notes_only) {
                                out.push(RawLink {
                                    target: target.to_string(),
                                    wiki: false,
                                    range: line_start + i..line_start + after + 1 + paren + 1,
                                    context: context.clone(),
                                });
                            }
                            i = after + 1 + paren + 1;
                            continue;
                        }
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// `#tag` occurrences (letters, digits, `-`, `_`, `/`), skipping code
/// and requiring a boundary before the `#`.
pub fn extract_tags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut in_code = false;
        while i < bytes.len() {
            match bytes[i] {
                b'`' => in_code = !in_code,
                b'#' if !in_code => {
                    let boundary = i == 0
                        || bytes[i - 1].is_ascii_whitespace()
                        || bytes[i - 1] == b'(';
                    if boundary {
                        let rest = &line[i + 1..];
                        let len = rest
                            .bytes()
                            .take_while(|b| {
                                b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/')
                            })
                            .count();
                        // A tag starts with a letter (so #1 or ## are not tags).
                        if len > 0 && rest.as_bytes()[0].is_ascii_alphabetic() {
                            out.push(rest[..len].to_string());
                            i += len;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    out
}

/// Lexically normalize `.` and `..` segments.
fn normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    let mut prefix = PathBuf::new();
    for comp in path.components() {
        use std::path::Component;
        match comp {
            Component::ParentDir => {
                if stack.pop().is_none() {
                    prefix.push("..");
                }
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => prefix.push(comp.as_os_str()),
            Component::Normal(s) => stack.push(s.to_os_string()),
        }
    }
    let mut out = prefix;
    for s in stack {
        out.push(s);
    }
    out
}

/// Where `[[target]]`, written in a note living in `from_dir`, would be
/// created — or `None` when that lands anywhere but inside `root`.
///
/// `target` is unsanitised document text. It may contain `..`, it may be
/// absolute (`Path::join` with an absolute path DISCARDS the base, so
/// `[[/tmp/x]]` escapes without a single `..`), and it may travel
/// through a symlink that leaves the workspace.
///
/// Containment is decided the same way `Index::resolve` decides it — on
/// canonicalised paths, the only form that sees where a path really
/// lands — but a note that does not exist yet cannot be canonicalised,
/// and neither can a parent directory we are about to create. So the
/// deepest ancestor that *does* canonicalise is the one checked, self
/// first: a path that already exists as a symlink out of the workspace
/// is caught by its own entry. Everything below the anchor is a plain
/// name (`normalize` has collapsed every `.` and `..`), so it cannot
/// climb back out. Fails closed: anything uncanonicalisable is refused.
///
/// The path handed back is the lexical one, not the canonical one, so it
/// keeps the identity the index and the open tabs already use; it names
/// the same location the check approved.
pub fn creatable_note_path(root: &Path, from_dir: &Path, target: &str) -> Option<PathBuf> {
    if target.trim().is_empty() {
        return None;
    }
    let path = normalize(&from_dir.join(format!("{target}.md")));
    let canon_root = root.canonicalize().ok()?;
    let anchor = path.ancestors().find_map(|a| a.canonicalize().ok())?;
    anchor.starts_with(&canon_root).then_some(path)
}

fn stem_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Which of `files` (every real, non-symlink file the workspace walk
/// found) name an inode with a link the walk never reached -- i.e.
/// `nlink`, the inode's *total* link count, exceeds how many of these
/// paths share that inode. `ln ~/.ssh/id_rsa ws/leak.md` has no symlink
/// for `is_symlink()` to catch, but reads the exact same bytes as a
/// file outside the workspace, so it is excluded here.
///
/// Comparing counts rather than testing `nlink() > 1` alone is what
/// keeps this exact instead of merely cautious: a user who names one
/// note twice (`ln Roadmap.md Alias.md`), or whose vault sits inside a
/// `cp -al` style backup tree, has in-workspace files with `nlink > 1`
/// too. Flagging on `nlink` alone would drop those from the index with
/// no error -- no backlinks, no wiki-completion, silently -- which is
/// worse than the vector this closes, since git cannot carry a
/// hardlink across a clone in the first place. Every link this walk
/// observed is accounted for; only a link it *didn't* see (because it
/// lives outside `root`) makes the count come up short.
#[cfg(unix)]
fn hardlinked_outside_the_workspace(files: &[PathBuf]) -> std::collections::HashSet<PathBuf> {
    use std::collections::HashMap;
    use std::os::unix::fs::MetadataExt;

    let mut by_inode: HashMap<(u64, u64), (u64, usize)> = HashMap::new();
    let mut inode_of: Vec<(&PathBuf, (u64, u64))> = Vec::with_capacity(files.len());
    for path in files {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        let key = (meta.dev(), meta.ino());
        by_inode.entry(key).or_insert((meta.nlink(), 0)).1 += 1;
        inode_of.push((path, key));
    }
    inode_of
        .into_iter()
        .filter_map(|(path, key)| {
            let (nlink, observed) = by_inode[&key];
            (observed < nlink as usize).then(|| path.clone())
        })
        .collect()
}

#[cfg(not(unix))]
fn hardlinked_outside_the_workspace(_files: &[PathBuf]) -> std::collections::HashSet<PathBuf> {
    std::collections::HashSet::new()
}

impl Index {
    /// Scan every markdown file under `root`.
    pub fn scan(root: &Path) -> Self {
        let mut index = Index { root: root.to_path_buf(), notes: BTreeMap::new() };
        // Only real files that live under `root` may be indexed. A
        // symlink inside the workspace can point anywhere on disk and
        // `read_to_string` would follow it, so `<root>/leak.md ->
        // ~/.ssh/id_rsa` would otherwise be indexed under an in-root
        // path — and every lookup answers from the index before any
        // escape guard runs. The walker does not follow links
        // (`follow_links(false)`), so a symlink arrives here as an
        // entry of its own with `is_symlink()` set; dropping it is
        // what makes the in-index short-circuit in `resolve` safe.
        let files: Vec<PathBuf> = crate::files::workspace_walk(root)
            .flatten()
            .filter(|item| item.file_type().is_some_and(|t| t.is_file()))
            .map(|item| item.path().to_path_buf())
            .collect();
        // A hardlink is not a symlink -- the filter above never sees
        // it -- but deciding whether one escapes the workspace needs
        // every path the walk found, so it can't run until the walk
        // above has finished.
        let excluded = hardlinked_outside_the_workspace(&files);
        for path in &files {
            if excluded.contains(path) {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(text) = std::fs::read_to_string(path) {
                    index.update_file(path, &text);
                }
            }
        }
        index
    }

    /// (Re-)index one file's text.
    pub fn update_file(&mut self, path: &Path, text: &str) {
        self.notes.insert(
            path.to_path_buf(),
            NoteData { links: extract_links(text), tags: extract_tags(text) },
        );
    }

    pub fn remove_file(&mut self, path: &Path) {
        self.notes.remove(path);
    }

    /// Resolve a link written in `from` to an indexed note's path.
    pub fn resolve(&self, from: &Path, link: &RawLink) -> Option<PathBuf> {
        if link.wiki {
            let target = link.target.replace('\\', "/");
            let stem = target.rsplit('/').next().unwrap_or(&target).to_lowercase();
            let wants_path = target.contains('/');
            let suffix = format!("{}.md", target.to_lowercase());
            let mut candidates: Vec<&PathBuf> = self
                .notes
                .keys()
                .filter(|p| stem_of(p).to_lowercase() == stem)
                .filter(|p| {
                    !wants_path
                        || p.to_string_lossy().to_lowercase().replace('\\', "/").ends_with(&suffix)
                })
                .collect();
            candidates.sort_by_key(|p| {
                let same_dir = p.parent() == from.parent();
                (!same_dir, p.as_os_str().len())
            });
            candidates.first().map(|p| (*p).clone())
        } else {
            let base = from.parent()?;
            let resolved = normalize(&base.join(&link.target));
            // A known note answers straight from the index without a
            // filesystem check, which is what keeps a rename working:
            // the *old* path can still be indexed after the file has
            // already moved off disk under it, and a link written to it
            // must still resolve until the watcher catches up.
            //
            // What makes that safe is `scan` (and `on_fs_events`)
            // refusing to index anything that is not a real file
            // beneath `root` — symlinks included. It is NOT true that a
            // path merely reached via the workspace walk is inside
            // `root`: without that filter a symlinked note would be
            // indexed under an in-root path and this branch would hand
            // it back before the escape guard below ever ran.
            if self.notes.contains_key(&resolved) {
                return Some(resolved);
            }
            // Opening is a different question from indexing: any file
            // inside the workspace that exists on disk is a valid
            // target, which is what makes `[config](./config.toml)`
            // work even though the index holds only `.md`.
            if !resolved.is_file() {
                return None;
            }
            // A lexical `starts_with(&self.root)` is not enough: a
            // symlink *inside* the workspace can point outside it
            // (`<root>/esc -> /etc`). A target like `./esc/passwd` has
            // no `..` for `normalize` to collapse, so a lexical check
            // would pass it straight through, and `is_file()` above
            // already followed the symlink to wherever it really
            // leads. Canonicalise both sides so the comparison sees
            // where the path actually lands. If either side fails to
            // canonicalise, fail closed and deny the link — for a
            // workspace-escape guard the unsafe default is failing
            // open (a link that escapes), not failing closed (a link
            // that doesn't resolve), and this check exists specifically
            // to hold up under the sandboxed App Store build.
            let Ok(canon_root) = self.root.canonicalize() else { return None };
            let Ok(canon_resolved) = resolved.canonicalize() else { return None };
            canon_resolved.starts_with(&canon_root).then_some(resolved)
        }
    }

    /// Notes linking to `target`: (source path, context lines).
    pub fn backlinks(&self, target: &Path) -> Vec<(PathBuf, Vec<String>)> {
        let mut out = Vec::new();
        for (path, data) in &self.notes {
            if path == target {
                continue;
            }
            let contexts: Vec<String> = data
                .links
                .iter()
                .filter(|l| self.resolve(path, l).as_deref() == Some(target))
                .map(|l| l.context.clone())
                .collect();
            if !contexts.is_empty() {
                out.push((path.clone(), contexts));
            }
        }
        out
    }

    /// Completion source: (stem, path) for every note, sorted by stem.
    pub fn note_names(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> =
            self.notes.keys().map(|p| (stem_of(p), p.clone())).collect();
        out.sort();
        out
    }

    /// All tags with their occurrence counts, most-used first.
    /// Link targets in `path` that resolve to nothing, deduplicated
    /// and in document order. These are the notes the vault refers to
    /// but does not have — the graph draws them as ghosts.
    pub fn unresolved_links(&self, path: &Path) -> Vec<String> {
        let Some(note) = self.notes.get(path) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for link in &note.links {
            // Only wiki links: a relative path that does not exist is a
            // typo, not a note someone intends to write.
            if !link.wiki || self.resolve(path, link).is_some() {
                continue;
            }
            if !out.contains(&link.target) {
                out.push(link.target.clone());
            }
        }
        out
    }

    /// The tags on one note, in the order they appear. Used by the
    /// graph to colour nodes by tag.
    pub fn note_tags(&self, path: &Path) -> Vec<String> {
        self.notes.get(path).map(|n| n.tags.clone()).unwrap_or_default()
    }

    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for data in self.notes.values() {
            for tag in &data.tags {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut out: Vec<(String, usize)> =
            counts.into_iter().map(|(t, n)| (t.to_string(), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// Paths of notes carrying `tag`.
    pub fn notes_tagged(&self, tag: &str) -> Vec<PathBuf> {
        self.notes
            .iter()
            .filter(|(_, d)| d.tags.iter().any(|t| t == tag))
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// A note moved: re-key it and return, for every note whose links
    /// pointed at it (including its own now-stale relative links), the
    /// rewritten text. Caller persists those.
    pub fn rename_note(
        &mut self,
        old: &Path,
        new: &Path,
        read_text: impl Fn(&Path) -> Option<String>,
    ) -> Vec<(PathBuf, String)> {
        let mut changed = Vec::new();
        // Other notes first, while the index still resolves to `old`.
        let sources: Vec<PathBuf> = self.notes.keys().filter(|p| *p != old).cloned().collect();
        for path in sources {
            let Some(text) = read_text(&path) else { continue };
            if let Some(rewritten) = rewrite_links(&text, &path, old, new, self) {
                self.update_file(&path, &rewritten);
                changed.push((path, rewritten));
            }
        }
        // The moved note itself: relative targets recompute from its
        // new directory; wiki links are location-independent.
        if let Some(data) = self.notes.remove(old) {
            self.notes.insert(new.to_path_buf(), data);
        }
        if let Some(text) = read_text(new).or_else(|| read_text(old)) {
            let links = extract_links(&text);
            let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
            for link in &links {
                if link.wiki {
                    continue;
                }
                // Resolve against the OLD location, then re-relativize.
                let from_old = old
                    .parent()
                    .map(|base| normalize(&base.join(&link.target)))
                    .filter(|p| self.notes.contains_key(p) || p.exists());
                if let (Some(target), Some(new_dir)) = (from_old, new.parent()) {
                    let rel = relative_path(new_dir, &target);
                    if rel != link.target {
                        edits.push((link.range.clone(), rel));
                    }
                }
            }
            if !edits.is_empty() {
                let rewritten = splice_md_targets(&text, &links, &edits);
                self.update_file(new, &rewritten);
                changed.push((new.to_path_buf(), rewritten));
            }
        }
        changed
    }

    /// Every resolved (source, target) link pair in the workspace.
    pub fn edges(&self) -> Vec<(PathBuf, PathBuf)> {
        let mut out = Vec::new();
        for (path, data) in &self.notes {
            for link in &data.links {
                if let Some(target) = self.resolve(path, link) {
                    if &target != path {
                        out.push((path.clone(), target));
                    }
                }
            }
        }
        out
    }

    /// The link (if any) whose range contains `offset` in `text`.
    pub fn link_at(text: &str, offset: usize) -> Option<RawLink> {
        extract_all_links(text)
            .into_iter()
            .find(|l| l.range.contains(&offset))
    }
}

/// Replace the `(target)` part of the given markdown links.
fn splice_md_targets(
    text: &str,
    links: &[RawLink],
    edits: &[(std::ops::Range<usize>, String)],
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for link in links {
        let Some((_, new_target)) = edits.iter().find(|(r, _)| *r == link.range) else {
            continue;
        };
        let whole = &text[link.range.clone()];
        let open = whole.rfind('(').unwrap_or(0);
        out.push_str(&text[at..link.range.start + open + 1]);
        out.push_str(new_target);
        at = link.range.end - 1; // keep the closing paren
    }
    out.push_str(&text[at..]);
    out
}

/// Rewrite one note's links after `old` moved to `new`: wiki stems
/// swap (labels survive), relative targets are recomputed.
pub fn rewrite_links(
    text: &str,
    note_path: &Path,
    old: &Path,
    new: &Path,
    resolver: &Index,
) -> Option<String> {
    let links = extract_links(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut changed = false;
    for link in &links {
        if resolver.resolve(note_path, link).as_deref() != Some(old) {
            continue;
        }
        out.push_str(&text[at..link.range.start]);
        if link.wiki {
            let whole = &text[link.range.clone()];
            let label = whole[2..whole.len() - 2]
                .split_once('|')
                .map(|(_, l)| l.to_string());
            match label {
                Some(label) => out.push_str(&format!("[[{}|{label}]]", stem_of(new))),
                None => out.push_str(&format!("[[{}]]", stem_of(new))),
            }
        } else {
            let whole = &text[link.range.clone()];
            let open = whole.rfind('(').unwrap_or(0);
            let rel = note_path
                .parent()
                .map(|dir| relative_path(dir, new))
                .unwrap_or_else(|| new.to_string_lossy().into_owned());
            out.push_str(&whole[..open + 1]);
            out.push_str(&rel);
            out.push(')');
        }
        at = link.range.end;
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(&text[at..]);
    Some(out)
}

/// Relative path from `dir` to `target` (`../` as needed).
pub fn relative_path(dir: &Path, target: &Path) -> String {
    let dir_comps: Vec<_> = dir.components().collect();
    let tgt_comps: Vec<_> = target.components().collect();
    let common = dir_comps
        .iter()
        .zip(tgt_comps.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let ups = dir_comps.len() - common;
    let mut parts: Vec<String> = std::iter::repeat("..".to_string()).take(ups).collect();
    parts.extend(
        tgt_comps[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(target: &str, wiki: bool) -> RawLink {
        RawLink { target: target.into(), wiki, range: 0..1, context: String::new() }
    }

    #[test]
    fn classify_separates_external_wiki_and_relative() {
        assert_eq!(classify(&raw("https://apple.com", false)),
                   LinkTarget::External("https://apple.com".into()));
        assert_eq!(classify(&raw("http://localhost:8080", false)),
                   LinkTarget::External("http://localhost:8080".into()));
        assert_eq!(classify(&raw("Note", true)), LinkTarget::Wiki("Note".into()));
        assert_eq!(classify(&raw("./config.toml", false)),
                   LinkTarget::Relative("./config.toml".into()));
    }

    #[test]
    fn classify_treats_other_schemes_as_relative_not_external() {
        // Only http(s) is opened. A note is untrusted content; file://,
        // mailto: and supermd:// must not become one-click actions.
        for t in ["file:///etc/passwd", "mailto:a@b.c", "supermd://install-plugin?name=x"] {
            assert!(matches!(classify(&raw(t, false)), LinkTarget::Relative(_)), "{t}");
        }
    }

    #[test]
    fn a_wiki_link_is_wiki_even_if_it_looks_like_a_url() {
        assert_eq!(classify(&raw("https://x", true)), LinkTarget::Wiki("https://x".into()));
    }

    const NOTE: &str = "# Project\n\
        See [[Roadmap]] and [[plans/Budget|the budget]].\n\
        Also [the spec](specs/design.md) and [site](https://x.y).\n\
        `[[not-a-link]]` here.\n\
        ```\n[[also not]]\n```\n\
        Tagged #planning and #q3/goals but not#this.\n";

    #[test]
    fn links_extract_with_context_and_skip_code() {
        let links = extract_links(NOTE);
        let targets: Vec<(&str, bool)> =
            links.iter().map(|l| (l.target.as_str(), l.wiki)).collect();
        assert_eq!(
            targets,
            vec![("Roadmap", true), ("plans/Budget", true), ("specs/design.md", false)]
        );
        assert!(links[0].context.contains("See"), "{}", links[0].context);
        assert_eq!(&NOTE[links[0].range.clone()], "[[Roadmap]]");
        assert_eq!(&NOTE[links[1].range.clone()], "[[plans/Budget|the budget]]");
    }

    #[test]
    fn https_targets_and_non_md_files_are_not_note_links() {
        let links = extract_links("[a](https://x.y) [b](img.png) [c](note.md)");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "note.md");
    }

    #[test]
    fn tags_extract_and_headings_do_not_count() {
        assert_eq!(extract_tags(NOTE), vec!["planning", "q3/goals"]);
        assert_eq!(extract_tags("# heading\n## other\n"), Vec::<String>::new());
        assert_eq!(extract_tags("mid #tag, end #last\n"), vec!["tag", "last"]);
    }

    fn fixture() -> (tempfile::TempDir, Index) {
        let dir = tempfile::tempdir().unwrap();
        let w = |p: &str, t: &str| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        w("Roadmap.md", "The plan. #planning\n");
        w("plans/Budget.md", "Numbers. See [[Roadmap]].\n");
        w("specs/design.md", "Spec body. [back](../Roadmap.md) #planning\n");
        w("Project.md", NOTE);
        w("notes.txt", "not markdown [[Roadmap]]");
        let index = Index::scan(dir.path());
        (dir, index)
    }

    /// The committed example vault is a demo people open first, so a
    /// dangling link in it reads as the app being broken. Index it for
    /// real and resolve every link with the same code the editor uses,
    /// so the vault cannot rot silently as features change.
    ///
    /// The unresolved names below are deliberate: the vault teaches that
    /// clicking a link to a note that does not exist creates it, which
    /// needs links that genuinely do not resolve.
    #[test]
    fn the_example_vault_has_no_accidentally_broken_links() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/vault");
        assert!(root.is_dir(), "the example vault is committed at examples/vault");
        let index = Index::scan(&root);

        const DELIBERATELY_MISSING: &[&str] = &[
            "Ghost note",
            "Another missing page",
            "A note nobody has written",
            "Rope internals",
            // Images.md shows what a broken image looks like in place.
            "../assets/nothing-here.png",
        ];

        let mut broken = Vec::new();
        for entry in ignore::Walk::new(&root).flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap();
            // `extract_all_links` is what the click path asks, and it
            // already skips fenced and inline code — so prose *about*
            // link syntax does not count as a link.
            for link in extract_all_links(&text) {
                if matches!(classify(&link), LinkTarget::External(_)) {
                    continue;
                }
                if DELIBERATELY_MISSING.contains(&link.target.as_str()) {
                    continue;
                }
                // An in-document anchor is not a path — it must name a
                // heading in this same file. The `toc` plugin writes a
                // page of them into Plugins.md, so this checks the
                // generated table of contents actually points at
                // something.
                if let LinkTarget::Anchor(a) = classify(&link) {
                    if heading_offset(&text, &a).is_none() {
                        broken.push(format!(
                            "{}: anchor #{a} names no heading",
                            path.strip_prefix(&root).unwrap().display()
                        ));
                    }
                    continue;
                }
                if index.resolve(path, &link).is_none() {
                    broken.push(format!(
                        "{}: [[{}]]",
                        path.strip_prefix(&root).unwrap().display(),
                        link.target
                    ));
                }
            }
        }
        assert!(broken.is_empty(), "unresolved links in the example vault: {broken:#?}");
    }

    #[test]
    fn scan_indexes_markdown_only_and_resolves_wiki_stems() {
        let (dir, index) = fixture();
        assert_eq!(index.note_names().len(), 4, "txt files are not notes");
        let project = dir.path().join("Project.md");
        let links = extract_links(NOTE);
        assert_eq!(
            index.resolve(&project, &links[0]),
            Some(dir.path().join("Roadmap.md"))
        );
        // Path-suffix wiki target.
        assert_eq!(
            index.resolve(&project, &links[1]),
            Some(dir.path().join("plans/Budget.md"))
        );
        // Relative standard link.
        assert_eq!(
            index.resolve(&project, &links[2]),
            Some(dir.path().join("specs/design.md"))
        );
        // Case-insensitive stems; unknown stays unresolved.
        let ci = RawLink { target: "roadmap".into(), wiki: true, range: 0..0, context: String::new() };
        assert_eq!(index.resolve(&project, &ci), Some(dir.path().join("Roadmap.md")));
        let nope = RawLink { target: "Ghost".into(), wiki: true, range: 0..0, context: String::new() };
        assert_eq!(index.resolve(&project, &nope), None);
    }

    #[test]
    fn relative_links_resolve_to_any_file_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("note.md"), "see [c](./config.toml)").unwrap();
        std::fs::write(root.join("config.toml"), "x = 1").unwrap();
        let index = Index::scan(root);

        let link = RawLink {
            target: "./config.toml".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        assert_eq!(
            index.resolve(&root.join("note.md"), &link),
            Some(normalize(&root.join("config.toml"))),
            "a non-Markdown file that exists should resolve"
        );
    }

    #[test]
    fn relative_links_to_missing_files_do_not_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let index = Index::scan(root);
        let link = RawLink {
            target: "./nope.png".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        // Unlike a wiki link, a relative link to a missing file is not
        // created — inventing `nope.png` would be nonsense.
        assert_eq!(index.resolve(&root.join("note.md"), &link), None);
    }

    #[test]
    fn relative_links_cannot_escape_the_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "s").unwrap();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let index = Index::scan(&root);
        let link = RawLink {
            target: "../secret.txt".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        assert_eq!(index.resolve(&root.join("note.md"), &link), None,
                   "a link must not reach outside the opened folder");
    }

    #[cfg(unix)]
    #[test]
    fn relative_links_through_a_symlink_cannot_escape_the_workspace_root() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        // A symlink living *inside* the workspace but pointing outside
        // it: `../secret.txt`-style lexical checks never see this,
        // because the link text itself contains no `..`.
        symlink(&outside, root.join("esc")).unwrap();
        let index = Index::scan(&root);
        let link = RawLink {
            target: "./esc/secret.txt".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        assert_eq!(
            index.resolve(&root.join("note.md"), &link), None,
            "a symlink inside the workspace must not be usable to escape it"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_markdown_file_is_neither_indexed_nor_resolved() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "secret").unwrap();
        // The dangerous shape the *directory* symlink test above never
        // reaches: a symlinked `.md` FILE sitting directly in the
        // workspace. The walker yields it (there is nothing to descend
        // into), `scan` only checked the extension, and `read_to_string`
        // follows the link — so its contents were indexed under an
        // in-root path, and every later lookup answered from the index
        // before any escape guard could run.
        let leak = root.join("leak.md");
        symlink(&outside, &leak).unwrap();
        let index = Index::scan(&root);
        assert!(
            !index.notes.contains_key(&leak),
            "a symlinked note must never enter the index"
        );

        let relative = RawLink {
            target: "./leak.md".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        assert_eq!(
            index.resolve(&root.join("note.md"), &relative), None,
            "a relative link through a symlinked note must not resolve"
        );
        let wiki = RawLink {
            target: "leak".into(), wiki: true, range: 0..1,
            context: String::new(),
        };
        assert_eq!(
            index.resolve(&root.join("note.md"), &wiki), None,
            "a wiki link to a symlinked note must not resolve"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_hardlinked_markdown_file_is_not_indexed() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "secret").unwrap();
        // `ln outside.md ws/leak.md`: no symlink at all, so the
        // walker's `is_symlink()` filter never sees it, and
        // `read_to_string` reads the *same inode* as the file outside
        // the workspace -- git cannot carry this (hardlinks do not
        // survive a clone), but the property claimed is that nothing
        // outside the workspace is read.
        let leak = root.join("leak.md");
        std::fs::hard_link(&outside, &leak).unwrap();
        assert!(
            std::fs::metadata(&leak).unwrap().nlink() > 1,
            "precondition: the walked path really is a hardlink"
        );
        let index = Index::scan(&root);
        assert!(
            !index.notes.contains_key(&leak),
            "a hardlinked note must never enter the index"
        );
    }

    /// The failure mode `nlink() > 1` alone would cause: a note
    /// hardlinked to another name *inside* the workspace (two note
    /// titles for the same file, or a `cp -al` style backup of the
    /// vault) must stay indexed. Every one of its links was seen by
    /// this walk, so nothing outside the workspace is implicated.
    #[cfg(unix)]
    #[test]
    fn a_hardlink_entirely_inside_the_workspace_stays_indexed() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Ideas.md"), "see [[Roadmap]] often\n").unwrap();
        let original = root.join("Roadmap.md");
        std::fs::write(&original, "the plan\n").unwrap();
        // `ln Roadmap.md Alias.md`: same inode, two names, both inside
        // the workspace this walk already covers in full.
        let alias = root.join("Alias.md");
        std::fs::hard_link(&original, &alias).unwrap();
        assert!(
            std::fs::metadata(&alias).unwrap().nlink() > 1,
            "precondition: the two paths really do share an inode"
        );

        let index = Index::scan(&root);
        assert!(
            index.notes.contains_key(&original),
            "an in-workspace hardlink target must stay indexed"
        );
        assert!(
            index.notes.contains_key(&alias),
            "and so must the alias sharing its inode -- every link landed inside the workspace"
        );
        let back = index.backlinks(&original);
        assert!(
            back.iter().any(|(p, _)| p.ends_with("Ideas.md")),
            "backlinks must still resolve through it: {back:?}"
        );
    }

    #[test]
    fn creatable_note_paths_stay_inside_the_workspace() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("ws");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let dir = root.join("sub");

        assert_eq!(
            creatable_note_path(&root, &dir, "Fresh"),
            Some(dir.join("Fresh.md")),
            "a plain target is created beside the note"
        );
        assert_eq!(
            creatable_note_path(&root, &dir, "deep/nested/Fresh"),
            Some(dir.join("deep").join("nested").join("Fresh.md")),
            "directories that do not exist yet are still creatable"
        );
        assert_eq!(
            creatable_note_path(&root, &dir, "../Sibling"),
            Some(root.join("Sibling.md")),
            "a `..` that stays inside the workspace is fine"
        );

        assert_eq!(
            creatable_note_path(&root, &dir, "../../escape"), None,
            "`..` must not climb out of the workspace"
        );
        let absolute = base.path().join("victim");
        assert_eq!(
            creatable_note_path(&root, &dir, &absolute.display().to_string()), None,
            "an absolute target replaces the base entirely and must be refused"
        );
        assert_eq!(creatable_note_path(&root, &dir, ""), None, "empty target");
        assert_eq!(creatable_note_path(&root, &dir, "  "), None, "blank target");
        assert_eq!(
            creatable_note_path(&base.path().join("gone"), &dir, "Fresh"), None,
            "a root that cannot be canonicalised fails closed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_creatable_note_path_cannot_travel_through_a_symlink() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        let outside = base.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("victim.md"), "precious").unwrap();
        // A directory symlink leaving the workspace: `esc/victim` has no
        // `..` for `normalize` to collapse, so only canonicalisation
        // sees the escape.
        symlink(&outside, root.join("esc")).unwrap();
        assert_eq!(
            creatable_note_path(&root, &root, "esc/victim"), None,
            "a symlinked directory must not be a route out"
        );
        assert_eq!(
            creatable_note_path(&root, &root, "esc/brand-new"), None,
            "…including for a note that does not exist yet"
        );
        // A symlinked *file* is caught by its own entry, not its parent.
        symlink(outside.join("victim.md"), root.join("leak.md")).unwrap();
        assert_eq!(
            creatable_note_path(&root, &root, "leak"), None,
            "an in-root name that is a symlink out must be refused"
        );
    }

    #[test]
    fn backlinks_collect_sources_with_context() {
        let (dir, index) = fixture();
        let mut back = index.backlinks(&dir.path().join("Roadmap.md"));
        back.sort();
        let sources: Vec<&Path> = back.iter().map(|(p, _)| p.as_path()).collect();
        assert_eq!(
            sources,
            vec![
                dir.path().join("Project.md").as_path(),
                dir.path().join("plans/Budget.md").as_path(),
                dir.path().join("specs/design.md").as_path(),
            ]
        );
        let budget_ctx = &back.iter().find(|(p, _)| p.ends_with("plans/Budget.md")).unwrap().1;
        assert!(budget_ctx[0].contains("See [[Roadmap]]"));
    }

    #[test]
    fn tags_aggregate_with_counts() {
        let (dir, index) = fixture();
        let tags = index.tags();
        assert_eq!(tags[0], ("planning".to_string(), 3));
        assert!(tags.iter().any(|(t, n)| t == "q3/goals" && *n == 1));
        let mut tagged = index.notes_tagged("planning");
        tagged.sort();
        assert_eq!(tagged.len(), 3);
        assert!(tagged.contains(&dir.path().join("Roadmap.md")));
    }

    #[test]
    fn update_and_remove_keep_the_index_fresh() {
        let (dir, mut index) = fixture();
        let extra = dir.path().join("Extra.md");
        index.update_file(&extra, "links [[Roadmap]] #planning");
        assert!(index
            .backlinks(&dir.path().join("Roadmap.md"))
            .iter()
            .any(|(p, _)| p == &extra));
        index.remove_file(&extra);
        assert!(!index
            .backlinks(&dir.path().join("Roadmap.md"))
            .iter()
            .any(|(p, _)| p == &extra));
    }

    #[test]
    fn relative_paths_walk_up_and_down() {
        let d = Path::new("/w/specs");
        assert_eq!(relative_path(d, Path::new("/w/Roadmap.md")), "../Roadmap.md");
        assert_eq!(relative_path(d, Path::new("/w/specs/x.md")), "x.md");
        assert_eq!(relative_path(Path::new("/w"), Path::new("/w/a/b.md")), "a/b.md");
    }

    #[test]
    fn rename_rewrites_wiki_and_relative_links_everywhere() {
        let (dir, mut index) = fixture();
        let old = dir.path().join("Roadmap.md");
        let new = dir.path().join("plans/Vision.md");
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::fs::rename(&old, &new).unwrap();
        let changed = index.rename_note(&old, &new, |p| std::fs::read_to_string(p).ok());
        let by_path: BTreeMap<_, _> = changed.into_iter().collect();

        let budget = &by_path[&dir.path().join("plans/Budget.md")];
        assert!(budget.contains("[[Vision]]"), "{budget}");
        let spec = &by_path[&dir.path().join("specs/design.md")];
        assert!(spec.contains("[back](../plans/Vision.md)"), "{spec}");
        let project = &by_path[&dir.path().join("Project.md")];
        assert!(project.contains("[[Vision]]"), "{project}");

        // The index itself now answers for the new path.
        assert!(index.note_names().iter().any(|(n, _)| n == "Vision"));
        assert!(!index.note_names().iter().any(|(n, _)| n == "Roadmap"));
    }

    /// The `toc` plugin writes `[Heading](#heading)` links. They were
    /// classified as relative paths, joined onto a directory, resolved
    /// to nothing, and did nothing when clicked — a plugin we ship
    /// generating links the editor could not follow.
    #[test]
    fn an_anchor_is_its_own_kind_not_a_relative_path() {
        let link = RawLink {
            target: "#calc--arithmetic".into(),
            wiki: false,
            range: 0..0,
            context: String::new(),
        };
        assert_eq!(classify(&link), LinkTarget::Anchor("calc--arithmetic".into()));
    }

    #[test]
    fn heading_offset_finds_the_heading_an_anchor_names() {
        let doc = "# Top\n\nbody\n\n## calc — arithmetic in prose\n\nmore\n";
        let at = heading_offset(doc, "calc--arithmetic-in-prose").expect("found");
        assert_eq!(&doc[at..at + 6], "## cal");
        assert_eq!(heading_offset(doc, "top"), Some(0));
        assert_eq!(heading_offset(doc, "nothing-like-this"), None);
    }

    /// Same two traps the `toc` plugin had: a fenced `# heading` is not
    /// a heading, and `#guide` is a tag, not a level-1 heading.
    #[test]
    fn heading_offset_ignores_fenced_code_and_tag_lines() {
        assert_eq!(heading_offset("```\n# Fenced\n```\n", "fenced"), None);
        assert_eq!(heading_offset("#guide #plugins\n", "guide-plugins"), None);
    }

    #[test]
    fn link_at_finds_the_span_under_a_cursor() {
        let text = "before [[Target]] after [x](y.md)";
        let hit = Index::link_at(text, 10).expect("inside wiki link");
        assert_eq!(hit.target, "Target");
        let hit = Index::link_at(text, text.len() - 2).expect("inside md link");
        assert_eq!(hit.target, "y.md");
        assert!(Index::link_at(text, 3).is_none());
    }
}
