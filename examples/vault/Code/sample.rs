//! A small module in the shape of the real ones: doc comments, tests
//! beside the code, and no language server to lean on.

use std::ops::Range;

/// Where a link points, once you know which kind it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An `http://` or `https://` address. Everything else is a path.
    External(String),
    /// A `[[wiki]]` target, resolved by stem against the index.
    Wiki(String),
    /// A path relative to the note holding the link.
    Relative(String),
}

/// Classify before resolving: an external address must never be joined
/// onto a filesystem path, and a wiki stem is not a path at all.
pub fn classify(target: &str, wiki: bool) -> LinkTarget {
    if wiki {
        return LinkTarget::Wiki(target.to_string());
    }
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        LinkTarget::External(target.to_string())
    } else {
        LinkTarget::Relative(target.to_string())
    }
}

/// The byte range of the word containing `offset`.
pub fn word_at(text: &str, offset: usize) -> Range<usize> {
    let start = text[..offset]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map_or(text.len(), |(i, _)| offset + i);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_schemes_are_external() {
        assert_eq!(classify("https://x.dev", false), LinkTarget::External("https://x.dev".into()));
        assert_eq!(classify("mailto:a@b.c", false), LinkTarget::Relative("mailto:a@b.c".into()));
        assert_eq!(classify("Notes/a.md", false), LinkTarget::Relative("Notes/a.md".into()));
        assert_eq!(classify("Roadmap", true), LinkTarget::Wiki("Roadmap".into()));
    }

    #[test]
    fn word_at_finds_boundaries() {
        assert_eq!(word_at("alpha beta gamma", 8), 6..10);
    }
}
