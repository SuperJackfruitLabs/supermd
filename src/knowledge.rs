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

/// The workspace's shared index. Absent until a folder is open.
#[derive(Clone)]
pub struct KnowledgeState(pub std::sync::Arc<std::sync::Mutex<Index>>);
impl gpui::Global for KnowledgeState {}

/// Extract wiki + markdown links. Fenced code blocks and inline code
/// are skipped; `[[Target|label]]` yields `Target`; only relative
/// `.md` targets count for standard links.
pub fn extract_links(text: &str) -> Vec<RawLink> {
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
            scan_line(trimmed, line_start, &mut out);
        }
        line_start += line.len();
    }
    out
}

/// Links on one line, honoring inline-code spans.
fn scan_line(line: &str, line_start: usize, out: &mut Vec<RawLink>) {
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
                            if target.ends_with(".md") && !target.contains("://") {
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

fn stem_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Index {
    /// Scan every markdown file under `root`.
    pub fn scan(root: &Path) -> Self {
        let mut index = Index { root: root.to_path_buf(), notes: BTreeMap::new() };
        for item in crate::files::workspace_walk(root).flatten() {
            let path = item.path();
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
            self.notes.contains_key(&resolved).then_some(resolved)
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
        extract_links(text)
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
