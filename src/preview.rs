//! What a link's hover preview should show, decided in pure code.
//!
//! The GPUI shell owns the popover element, the pointer handlers and
//! the fetch task; everything about *what* to draw, *when* to draw it,
//! and whether a domain may be contacted is decided here, under test.

use std::path::{Path, PathBuf};

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
}
