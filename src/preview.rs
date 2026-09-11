//! What a link's hover preview should show, decided in pure code.
//!
//! The GPUI shell owns the popover element, the pointer handlers and
//! the fetch task; everything about *what* to draw, *when* to draw it,
//! and whether a domain may be contacted is decided here, under test.

use std::path::{Path, PathBuf};

/// How long the popover survives after the pointer leaves the link.
///
/// Without this the popover vanished the instant the pointer moved off
/// the link — including when it moved *towards* the popover — so the
/// consent button could never be clicked.
pub const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_millis(300);

/// How long the pointer must rest on a link before its preview opens.
pub const DWELL: std::time::Duration = std::time::Duration::from_millis(400);

/// The first lines of a document, for a preview body. Front matter and
/// leading blank lines are skipped: a preview that opens with `---`
/// and a YAML block tells the reader nothing about the note.
pub fn excerpt(text: &str, max_lines: usize) -> String {
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim_end() == "---" {
                break;
            }
        }
    }
    let body: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take(max_lines)
        .collect();
    body.join("\n").trim_end().to_string()
}

/// A note's title: its first ATX heading, else its file stem. The
/// whitespace check is the one CommonMark requires — without it a tag
/// line like `#guide` reads as a heading.
pub fn title_of(text: &str, path: &Path) -> String {
    for line in text.lines() {
        let t = line.trim();
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes)
            && matches!(t[hashes..].chars().next(), Some(' ' | '\t'))
        {
            return t[hashes..].trim().to_string();
        }
    }
    path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// The domain of an `https://` URL. Only https: the fetch policy this
/// mirrors refuses everything else, and a preview must not be the one
/// place a `http://` or `file://` URL slips through.
pub fn domain_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| {
        url.strip_prefix("HTTPS://").or_else(|| url.strip_prefix("Https://"))
    })?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?; // drop any userinfo
    let host = host.split(':').next()?; // drop any port
    (!host.is_empty()).then_some(host)
}

/// True when a link's visible text names a different domain than the
/// link actually goes to — `[paypal.com](https://evil.example)`.
///
/// Only fires when the text genuinely looks like a hostname, so
/// ordinary prose links are never flagged. Local; needs no network,
/// and so works with no consent at all.
pub fn text_target_mismatch(link_text: &str, url: &str) -> bool {
    let text = link_text.trim().trim_end_matches('/');
    let text = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    let text = text.split('/').next().unwrap_or(text);
    // "Looks like a hostname": a dot, no spaces, and a plausible TLD.
    let looks_like_host = text.contains('.')
        && !text.contains(' ')
        && text.rsplit('.').next().is_some_and(|tld| {
            tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
        });
    if !looks_like_host {
        return false;
    }
    match domain_of(url) {
        Some(dest) => !same_site(text, dest),
        // A non-https destination under hostname-shaped text is worth
        // warning about on its own.
        None => true,
    }
}

/// Hostnames match if one is the other, or a subdomain of it.
fn same_site(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim_end_matches('.'), b.trim_end_matches('.'));
    a.eq_ignore_ascii_case(b)
        || a.to_ascii_lowercase().ends_with(&format!(".{}", b.to_ascii_lowercase()))
        || b.to_ascii_lowercase().ends_with(&format!(".{}", a.to_ascii_lowercase()))
}

/// The visible text of a link, from its source slice.
///
/// `[the spec](https://x)` reads "the spec"; `[[Note|label]]` reads
/// "label". Needed because `RawLink::context` is the whole containing
/// line — useful for backlink display, useless for asking whether the
/// text a reader sees names a different site than the destination.
pub fn display_text(slice: &str) -> &str {
    if let Some(inner) = slice.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        return inner.split('|').next_back().unwrap_or(inner).trim();
    }
    if slice.starts_with('[') {
        if let Some(close) = slice.find("](") {
            return slice[1..close].trim();
        }
    }
    slice.trim()
}

/// Whether this domain may be contacted, from the stored grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// Never asked. Show local information and offer to enable.
    Ungranted,
    /// Explicitly refused. Show local information and offer nothing.
    Denied,
    /// Allowed; a fetch may happen.
    Granted,
}

/// Read a domain's consent out of the `net:` / `denied:net:` grant
/// list — the same format `url-title` and every other networked plugin
/// already uses, so previews add no second permission model.
pub fn consent_for(domain: &str, grants: &[String]) -> Consent {
    let d = domain.to_ascii_lowercase();
    if grants.iter().any(|g| {
        g.strip_prefix("denied:net:").is_some_and(|x| x.eq_ignore_ascii_case(&d))
    }) {
        return Consent::Denied;
    }
    if grants
        .iter()
        .any(|g| g.strip_prefix("net:").is_some_and(|x| x.eq_ignore_ascii_case(&d)))
    {
        return Consent::Granted;
    }
    Consent::Ungranted
}

/// What the popover shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    /// A note that exists: its title and opening lines.
    Note { title: String, excerpt: String },
    /// A code or config file: its opening lines, and the language to
    /// highlight them as.
    Code { language: Option<String>, excerpt: String },
    /// An image on disk.
    Image { path: PathBuf },
    /// A heading in the document already open.
    Anchor { heading: String, excerpt: String },
    /// A wiki link with nothing behind it. Clicking creates the note,
    /// so the preview says so rather than looking like a failure.
    Missing { name: String },
    /// An external address. `fetched` is filled only once consent is
    /// granted and a request has completed.
    External {
        url: String,
        domain: String,
        mismatch: bool,
        consent: Consent,
        fetched: Option<FetchedMeta>,
    },
}

/// What a granted fetch produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedMeta {
    pub title: String,
    pub description: Option<String>,
}

/// How much of a page to read. Titles and descriptions live in
/// `<head>`; anything past this is body we would throw away, and a
/// hover must never pull a large download.
pub const MAX_FETCH_BYTES: usize = 128 * 1024;

/// How long a preview fetch may take before it is abandoned. Short: a
/// popover that arrives after the pointer has moved on is noise.
pub const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

/// Pull a title and description out of a page's HTML.
///
/// Deliberately not an HTML parser: this reads a few well-known tags
/// out of untrusted bytes, so it stays a small amount of code with no
/// recursion and no allocation proportional to nesting depth. Open
/// Graph wins over the plain tags when both are present, because it is
/// what the page chose to show when shared.
pub fn parse_meta(html: &str) -> Option<FetchedMeta> {
    let head = html.get(..html.len().min(MAX_FETCH_BYTES)).unwrap_or(html);
    let title = meta_content(head, "og:title")
        .or_else(|| tag_text(head, "title"))
        .map(|t| collapse(&t))
        .filter(|t| !t.is_empty())?;
    let description = meta_content(head, "og:description")
        .or_else(|| meta_content(head, "description"))
        .map(|d| collapse(&d))
        .filter(|d| !d.is_empty());
    Some(FetchedMeta { title, description })
}

/// The text of the first `<tag>…</tag>`.
fn tag_text(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find(&format!("<{tag}"))?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower[gt..].find(&format!("</{tag}"))? + gt;
    Some(unescape(&html[gt..close]))
}

/// The `content` of a `<meta>` whose name or property matches.
fn meta_content(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let needle = key.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find("<meta") {
        let start = from + rel;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        let tag_lower = &lower[start..end];
        let names_it = [format!("name=\"{needle}\""), format!("property=\"{needle}\"")]
            .iter()
            .any(|pat| tag_lower.contains(pat.as_str()));
        if names_it {
            if let Some(c) = attr(tag, "content") {
                return Some(unescape(&c));
            }
        }
        from = end.max(start + 5);
    }
    None
}

/// A double-quoted attribute's value.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &tag[at..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The handful of entities worth resolving in a title.
fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "\'")
        .replace("&nbsp;", " ")
}

/// Whitespace in markup is not whitespace on screen.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The preview for an external link, before any request is made.
pub fn external_preview(url: &str, link_text: &str, grants: &[String]) -> Preview {
    let domain = domain_of(url).unwrap_or("").to_string();
    let consent =
        if domain.is_empty() { Consent::Denied } else { consent_for(&domain, grants) };
    Preview::External {
        mismatch: text_target_mismatch(link_text, url),
        url: url.to_string(),
        domain,
        consent,
        fetched: None,
    }
}

/// The only path to the network for previews. Separate from the
/// catalog fetcher because the budgets are different: a catalog
/// download may take 30 s and 20 MB, a hover may not.
pub type PreviewFetcher = std::sync::Arc<
    dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync,
>;

/// Real transport. HTTPS only — `domain_of` already refuses anything
/// else, and this is the second place that must hold.
pub fn ureq_preview_fetcher() -> PreviewFetcher {
    std::sync::Arc::new(|url: &str| {
        if domain_of(url).is_none() {
            return Err("only https:// URLs are previewed".to_string());
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(FETCH_TIMEOUT))
            // Consent is per domain, so the request must end at the
            // domain that was granted. ureq defaults to following ten
            // redirects with https_only off, which let a granted site
            // redirect the fetch to any host, in cleartext -- to
            // `http://192.168.1.1/admin/...`, say -- chosen entirely by
            // the document's link target. That walks around the whole
            // consent model, so: no redirects, and https only.
            .https_only(true)
            .max_redirects(0)
            .build();
        let agent: ureq::Agent = config.into();
        let response = agent.get(url).call().map_err(|e| e.to_string())?;
        use std::io::Read as _;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_FETCH_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
}

/// Fetcher plus the session's memo of what has already been read.
/// Cached so moving back and forth over the same link asks once.
#[derive(Clone)]
pub struct PreviewState {
    pub fetcher: PreviewFetcher,
    pub cache: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, FetchedMeta>>>,
    /// The `supermd` preview grants, read once at construction rather
    /// than on every popover open — a file read and a TOML parse on
    /// the UI thread, every dwell. `refresh_grants` re-reads it; call
    /// that right after a grant is written, or the next hover answers
    /// from a stale copy.
    grants: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl PreviewState {
    pub fn new(fetcher: PreviewFetcher) -> Self {
        Self {
            fetcher,
            cache: Default::default(),
            grants: std::sync::Arc::new(std::sync::Mutex::new(stored_grants())),
        }
    }

    pub fn cached(&self, url: &str) -> Option<FetchedMeta> {
        self.cache.lock().ok()?.get(url).cloned()
    }

    pub fn remember(&self, url: &str, meta: FetchedMeta) {
        if let Ok(mut c) = self.cache.lock() {
            c.insert(url.to_string(), meta);
        }
    }

    /// The cached `supermd` preview grants.
    pub fn grants(&self) -> Vec<String> {
        self.grants.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Re-read the grants from disk into the cache. Call this right
    /// after writing a new one, or a hover already in flight answers
    /// from what was cached before the grant existed.
    pub fn refresh_grants(&self) {
        if let Ok(mut g) = self.grants.lock() {
            *g = stored_grants();
        }
    }
}

impl gpui::Global for PreviewState {}

/// Add a domain to the grant list, replacing any refusal of it.
/// Returns the list to store back under the `supermd` key.
pub fn grant_domain(domain: &str, grants: &[String]) -> Vec<String> {
    let d = domain.to_ascii_lowercase();
    let mut out: Vec<String> = grants
        .iter()
        .filter(|g| {
            !g.strip_prefix("denied:net:").is_some_and(|x| x.eq_ignore_ascii_case(&d))
                && !g.strip_prefix("net:").is_some_and(|x| x.eq_ignore_ascii_case(&d))
        })
        .cloned()
        .collect();
    out.push(format!("net:{d}"));
    out
}

/// When the popover should be open, as a function of pointer history
/// and time. Pure so the dwell rule is testable without a timer: the
/// shell calls `moved_to`/`left` from its handlers and `poll` from its
/// frame or its timer, and this decides.
#[derive(Debug, Default)]
pub struct HoverState<T> {
    /// The link under the pointer and when it arrived there.
    at: Option<(usize, T)>,
    open: bool,
}

impl<T: Copy + PartialOrd + std::ops::Sub<Output = std::time::Duration>> HoverState<T> {
    pub fn new() -> Self {
        Self { at: None, open: false }
    }

    /// The pointer is over the link whose span starts at `link_start`.
    /// Moving within one link keeps its dwell running; moving to a
    /// different link restarts it, so dragging along a line of links
    /// does not flash a popover for each one passed over.
    pub fn moved_to(&mut self, link_start: usize, now: T) {
        match self.at {
            Some((start, _)) if start == link_start => {}
            _ => {
                self.at = Some((link_start, now));
                self.open = false;
            }
        }
    }

    /// The pointer is over no link. Closes immediately: a preview that
    /// lingers after the pointer has gone is in the way.
    pub fn left(&mut self) {
        self.at = None;
        self.open = false;
    }

    /// True once the pointer has rested long enough. Latches, so the
    /// popover does not reopen on every poll once dismissed.
    pub fn poll(&mut self, now: T) -> bool {
        if let Some((_, since)) = self.at {
            if !self.open && now - since >= DWELL {
                self.open = true;
            }
        }
        self.open
    }

    /// The link the popover belongs to, if it is open.
    pub fn open_link(&self) -> Option<usize> {
        self.open.then(|| self.at.map(|(s, _)| s))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn excerpt_skips_front_matter_and_leading_blanks() {
        let doc = "---\ntitle: x\ntags: [a]\n---\n\n\n# Heading\n\nBody line.\nMore.\n";
        assert_eq!(excerpt(doc, 3), "# Heading\n\nBody line.");
        // No front matter: just the first lines.
        assert_eq!(excerpt("alpha\nbeta\ngamma\ndelta\n", 2), "alpha\nbeta");
        assert_eq!(excerpt("", 3), "");
    }

    #[test]
    fn title_is_the_first_real_heading_then_the_file_stem() {
        let p = Path::new("/vault/Roadmap.md");
        assert_eq!(title_of("# The Plan\n\nbody\n", p), "The Plan");
        // `#guide` is a tag, not a heading: no whitespace after the hash.
        assert_eq!(title_of("#guide #plugins\n\nbody\n", p), "Roadmap");
        assert_eq!(title_of("no heading at all\n", p), "Roadmap");
    }

    #[test]
    fn domain_of_accepts_only_https_and_strips_the_noise() {
        assert_eq!(domain_of("https://example.com/a/b?c#d"), Some("example.com"));
        assert_eq!(domain_of("https://user:pw@example.com:8443/x"), Some("example.com"));
        assert_eq!(domain_of("HTTPS://Example.com"), Some("Example.com"));
        // Everything the fetch policy refuses, refused here too.
        assert_eq!(domain_of("http://example.com"), None);
        assert_eq!(domain_of("file:///etc/passwd"), None);
        assert_eq!(domain_of("javascript:alert(1)"), None);
        assert_eq!(domain_of("https://"), None);
    }

    /// The one external check that works without consent, and the
    /// reason it is worth having at all.
    #[test]
    fn mismatch_flags_a_link_whose_text_names_another_site() {
        assert!(text_target_mismatch("paypal.com", "https://evil.example"));
        assert!(text_target_mismatch("https://apple.com/x", "https://phish.test"));
        // Same site, and subdomains of it, are not a mismatch.
        assert!(!text_target_mismatch("example.com", "https://example.com/deep/path"));
        assert!(!text_target_mismatch("example.com", "https://docs.example.com"));
        assert!(!text_target_mismatch("docs.example.com", "https://example.com"));
        // Ordinary prose is never flagged, however it is punctuated.
        assert!(!text_target_mismatch("the CommonMark spec", "https://commonmark.org"));
        assert!(!text_target_mismatch("see chapter 3.", "https://example.com"));
        assert!(!text_target_mismatch("Fig. 2", "https://example.com"));
        // Hostname-shaped text pointing somewhere non-https is worth
        // flagging on its own.
        assert!(text_target_mismatch("example.com", "notaurl"));
    }

    #[test]
    fn display_text_is_what_the_reader_sees() {
        assert_eq!(display_text("[the spec](https://x.dev)"), "the spec");
        assert_eq!(display_text("[[Note]]"), "Note");
        assert_eq!(display_text("[[Note|the label]]"), "the label");
        assert_eq!(display_text("[paypal.com](https://evil.example)"), "paypal.com");
        // Not link-shaped: the slice itself, so a caller never gets a
        // surprise empty string.
        assert_eq!(display_text("plain"), "plain");
    }

    #[test]
    fn parse_meta_reads_title_and_description() {
        let html = "<html><head><title>CommonMark Spec</title>\
                    <meta name=\"description\" content=\"A strongly defined spec.\">\
                    </head><body>ignored</body></html>";
        let m = parse_meta(html).expect("meta");
        assert_eq!(m.title, "CommonMark Spec");
        assert_eq!(m.description.as_deref(), Some("A strongly defined spec."));
    }

    /// Open Graph is what a page chose to show when shared, so it wins.
    #[test]
    fn open_graph_wins_over_the_plain_tags() {
        let html = "<head><title>fallback</title>\
                    <meta property=\"og:title\" content=\"The Real Title\">\
                    <meta property=\"og:description\" content=\"og text\">\
                    <meta name=\"description\" content=\"plain text\"></head>";
        let m = parse_meta(html).expect("meta");
        assert_eq!(m.title, "The Real Title");
        assert_eq!(m.description.as_deref(), Some("og text"));
    }

    /// Markup whitespace is not screen whitespace, and entities are not
    /// text. A popover full of `&amp;` and newlines is worse than none.
    #[test]
    fn titles_are_unescaped_and_collapsed() {
        let html = "<head><title>\n  Rust &amp; Wasm\n   guide  </title></head>";
        assert_eq!(parse_meta(html).unwrap().title, "Rust & Wasm guide");
    }

    /// Untrusted bytes: none of these may panic or hang.
    #[test]
    fn malformed_html_yields_nothing_rather_than_panicking() {
        assert_eq!(parse_meta(""), None);
        assert_eq!(parse_meta("<html><head><title>"), None, "unterminated title");
        assert_eq!(parse_meta("<meta name=\"description\" content=\"only a desc\">"), None);
        assert_eq!(parse_meta("<head><title>   </title></head>"), None, "blank title");
        assert_eq!(parse_meta("<<<<>>>><meta<meta<meta"), None);
        // A description with no closing quote must not run away.
        assert!(parse_meta("<head><title>t</title><meta name=\"description\" content=\"x").is_some());
    }

    #[test]
    fn consent_reads_the_existing_plugin_grant_format() {
        let grants = vec![
            "net:en.wikipedia.org".to_string(),
            "denied:net:evil.com".to_string(),
        ];
        assert_eq!(consent_for("en.wikipedia.org", &grants), Consent::Granted);
        assert_eq!(consent_for("EN.WIKIPEDIA.ORG", &grants), Consent::Granted);
        assert_eq!(consent_for("evil.com", &grants), Consent::Denied);
        assert_eq!(consent_for("unknown.test", &grants), Consent::Ungranted);
        assert_eq!(consent_for("anything", &[]), Consent::Ungranted);
    }

    /// A denial outranks a grant: if both are recorded for one domain,
    /// the refusal is the answer.
    /// Enabling a site must clear a previous refusal of it, or the
    /// denial would outrank the new grant and the button would appear
    /// to do nothing.
    #[test]
    fn granting_a_domain_clears_an_earlier_refusal() {
        let grants = vec![
            "net:keep.test".to_string(),
            "denied:net:x.test".to_string(),
        ];
        let after = grant_domain("X.TEST", &grants);
        assert_eq!(consent_for("x.test", &after), Consent::Granted);
        assert_eq!(
            consent_for("keep.test", &after),
            Consent::Granted,
            "other domains are untouched"
        );
        assert!(!after.iter().any(|g| g.starts_with("denied:net:x.test")));
    }

    #[test]
    fn granting_the_same_domain_twice_does_not_duplicate_it() {
        let once = grant_domain("a.test", &[]);
        let twice = grant_domain("a.test", &once);
        assert_eq!(twice, vec!["net:a.test".to_string()]);
    }

    #[test]
    fn the_cache_answers_without_a_second_request() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = calls.clone();
        let state = PreviewState::new(std::sync::Arc::new(move |_: &str| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(b"<head><title>Once</title></head>".to_vec())
        }));
        assert_eq!(state.cached("https://a.test"), None);
        let bytes = (state.fetcher)("https://a.test").unwrap();
        let meta = parse_meta(&String::from_utf8_lossy(&bytes)).unwrap();
        state.remember("https://a.test", meta.clone());
        assert_eq!(state.cached("https://a.test"), Some(meta));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1, "asked once");
    }

    #[test]
    fn a_denial_outranks_a_grant_for_the_same_domain() {
        let grants = vec!["net:x.test".to_string(), "denied:net:x.test".to_string()];
        assert_eq!(consent_for("x.test", &grants), Consent::Denied);
    }

    #[test]
    fn an_external_preview_carries_no_fetch_before_consent() {
        let p = external_preview("https://evil.example", "paypal.com", &[]);
        let Preview::External { domain, mismatch, consent, fetched, .. } = p else {
            panic!("external");
        };
        assert_eq!(domain, "evil.example");
        assert!(mismatch, "the text names another site");
        assert_eq!(consent, Consent::Ungranted);
        assert_eq!(fetched, None, "nothing is fetched before consent");
    }

    /// A URL the fetch policy would refuse must not present itself as
    /// merely un-granted, or the popover would offer to enable a site
    /// that can never be contacted.
    #[test]
    fn a_non_https_external_link_is_denied_not_merely_ungranted() {
        let p = external_preview("http://insecure.test", "insecure.test", &[]);
        let Preview::External { consent, domain, .. } = p else { panic!("external") };
        assert_eq!(consent, Consent::Denied);
        assert_eq!(domain, "");
    }

    #[test]
    fn the_popover_waits_for_the_dwell_then_latches() {
        let t0 = Instant::now();
        let mut h: HoverState<Instant> = HoverState::new();
        h.moved_to(10, t0);
        assert!(!h.poll(t0), "not immediately");
        assert!(!h.poll(t0 + DWELL - Duration::from_millis(1)), "not one tick early");
        assert!(h.poll(t0 + DWELL), "open once the dwell elapses");
        assert_eq!(h.open_link(), Some(10));
        h.left();
        assert!(!h.poll(t0 + DWELL * 2), "closes the moment the pointer leaves");
        assert_eq!(h.open_link(), None);
    }

    /// Sliding along a line of links must not flash a popover for each
    /// one crossed: arriving at a new link restarts the dwell.
    #[test]
    fn moving_to_another_link_restarts_the_dwell() {
        let t0 = Instant::now();
        let mut h: HoverState<Instant> = HoverState::new();
        h.moved_to(10, t0);
        h.moved_to(40, t0 + Duration::from_millis(390));
        assert!(!h.poll(t0 + DWELL), "the second link's dwell has not elapsed");
        assert!(h.poll(t0 + Duration::from_millis(390) + DWELL));
        assert_eq!(h.open_link(), Some(40));
    }

    /// Staying within one link keeps its dwell running rather than
    /// restarting it on every pixel of movement.
    #[test]
    fn moving_within_one_link_does_not_restart_it() {
        let t0 = Instant::now();
        let mut h: HoverState<Instant> = HoverState::new();
        h.moved_to(10, t0);
        h.moved_to(10, t0 + Duration::from_millis(200));
        h.moved_to(10, t0 + Duration::from_millis(399));
        assert!(h.poll(t0 + DWELL), "the original dwell still governs");
    }

    /// `PreviewState` reads the grants once, and only `refresh_grants`
    /// moves that cache forward -- the same shape `settings::load`
    /// itself has, minus a file read and a TOML parse on every hover.
    #[test]
    fn grants_are_read_once_and_refreshed_on_demand() {
        // HOME is process-wide: share the crate's one lock-guarded
        // helper rather than swapping it unguarded, or a test in
        // another file racing the same env var reads this test's
        // tempdir (or vice versa).
        let _home = crate::workspace::tests::temp_home();

        // A grant already exists on disk before the state is built.
        let dir = crate::settings::config_dir();
        let mut settings = crate::settings::load(&dir);
        settings.plugin_grants.insert("supermd".into(), vec!["net:first.test".into()]);
        crate::settings::save(&dir, &settings).unwrap();

        let state = PreviewState::new(std::sync::Arc::new(|_: &str| Ok(Vec::new())));
        assert_eq!(
            state.grants(),
            vec!["net:first.test".to_string()],
            "grants are read at construction"
        );

        // A second grant lands on disk exactly the way
        // `enable_previews_for_hovered_site` writes one -- straight
        // through `settings::save`, never through this state.
        settings.plugin_grants.insert(
            "supermd".into(),
            vec!["net:first.test".into(), "net:second.test".into()],
        );
        crate::settings::save(&dir, &settings).unwrap();
        assert_eq!(
            state.grants(),
            vec!["net:first.test".to_string()],
            "a stale cache must not see a new grant on its own"
        );

        state.refresh_grants();
        assert_eq!(
            state.grants(),
            vec!["net:first.test".to_string(), "net:second.test".to_string()],
            "refresh_grants must pick up what is on disk now"
        );
    }
}

/// A `Preview` as a tooltip view, for the rendered reading view.
///
/// The editor draws its own popover so the consent button can be
/// clicked; a tooltip cannot hold an interactive control, so this shows
/// what a site *would* preview and leaves enabling it to the editor.
pub struct PreviewTooltip {
    pub preview: Preview,
}

impl gpui::Render for PreviewTooltip {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{div, px, ParentElement, SharedString, Styled};
        let t = crate::theme::theme(cx);
        let (title, sub, body) = describe(&self.preview);
        div()
            .max_w(px(360.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .rounded_lg()
            .shadow_lg()
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(t.ui_size))
                    .text_color(t.fg_strong)
                    .child(SharedString::from(title)),
            )
            .children(sub.map(|s| {
                div()
                    .text_size(px(t.ui_size - 1.))
                    .text_color(t.fg_muted)
                    .child(SharedString::from(s))
            }))
            .children((!body.is_empty()).then(|| {
                div()
                    .mt_1()
                    .text_size(px(t.ui_size - 1.))
                    .text_color(t.fg)
                    .child(SharedString::from(body))
            }))
    }
}

/// A preview as (title, subtitle, body). Shared by the editor's popover
/// and the reading view's tooltip so the two cannot drift apart.
pub fn describe(preview: &Preview) -> (String, Option<String>, String) {
    match preview {
        Preview::Note { title, excerpt } => (title.clone(), None, excerpt.clone()),
        Preview::Code { language, excerpt } => (
            language.clone().unwrap_or_else(|| "Text".into()),
            None,
            excerpt.clone(),
        ),
        Preview::Anchor { heading, excerpt } => (heading.clone(), None, excerpt.clone()),
        Preview::Image { path } => (
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Some("Image".into()),
            String::new(),
        ),
        Preview::Missing { name } => (
            name.clone(),
            Some("Does not exist — click to create".into()),
            String::new(),
        ),
        Preview::External { url, domain, mismatch, consent, fetched } => {
            let title = match (consent, fetched) {
                (Consent::Granted, Some(m)) => m.title.clone(),
                _ if domain.is_empty() => url.clone(),
                _ => domain.clone(),
            };
            let mut sub = match (consent, fetched) {
                (Consent::Granted, Some(m)) => m.description.clone(),
                (Consent::Ungranted, _) => Some("Previews are off for this site".into()),
                (Consent::Denied, _) => Some("Previews refused for this site".into()),
                _ => None,
            };
            if *mismatch {
                let warn = "⚠ the link text names a different site";
                sub = Some(match sub {
                    Some(s) => format!("{warn} · {s}"),
                    None => warn.to_string(),
                });
            }
            (title, sub, url.clone())
        }
    }
}

/// The preview for a link, given a way to resolve paths and the stored
/// grants. Shared by the editor's popover and the reading view's
/// tooltip so the two cannot disagree about what a link shows.
///
/// Anchors are the caller's business: only it knows the document text.
pub fn preview_for_link(
    link: &crate::knowledge::RawLink,
    link_text: &str,
    grants: &[String],
    resolve: impl FnOnce(&crate::knowledge::RawLink) -> Option<std::path::PathBuf>,
) -> Preview {
    use crate::knowledge::LinkTarget;
    match crate::knowledge::classify(link) {
        LinkTarget::External(url) => external_preview(&url, link_text, grants),
        LinkTarget::Anchor(a) => Preview::Missing { name: format!("#{a}") },
        LinkTarget::Wiki(name) | LinkTarget::Relative(name) => {
            let Some(path) = resolve(link) else {
                return Preview::Missing { name };
            };
            if crate::files::is_image_path(&path) {
                return Preview::Image { path };
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                return Preview::Missing { name };
            };
            let is_md = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("md" | "markdown" | "mdown" | "mdx")
            );
            if is_md {
                Preview::Note { title: title_of(&text, &path), excerpt: excerpt(&text, 8) }
            } else {
                Preview::Code {
                    language: crate::reader::language_for_path(&path),
                    excerpt: excerpt(&text, 10),
                }
            }
        }
    }
}

/// The `supermd` preview grants, from disk.
pub fn stored_grants() -> Vec<String> {
    crate::settings::load(&crate::settings::config_dir())
        .plugin_grants
        .get("supermd")
        .cloned()
        .unwrap_or_default()
}
