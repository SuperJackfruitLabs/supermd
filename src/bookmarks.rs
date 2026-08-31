//! Security-scoped bookmark policy. The App Sandbox grants access to
//! what the user picks in an open panel; that grant dies with the
//! process unless it is captured as a bookmark. This module owns the
//! pure half — encoding, pruning, and what a resolution means. The
//! objc2 calls live in `bookmarks_mac.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// What resolving a stored bookmark produced.
#[derive(Debug, PartialEq)]
pub enum Resolution {
    /// Usable; access has been started.
    Fresh(PathBuf),
    /// Resolved, but macOS flagged it stale — the folder moved. Usable
    /// once, and the caller should re-create the bookmark.
    Stale(PathBuf),
    /// Gone: deleted, on an unmounted volume, or never bookmarked.
    Missing,
}

/// True when this build must capture bookmarks to keep folder access
/// across launches. Only the sandboxed macOS build does.
pub fn needs_scope() -> bool {
    cfg!(all(target_os = "macos", feature = "mas"))
}

/// Lowercase hex. Bookmark blobs are ~1KB of opaque bytes and
/// settings.toml is text, so they are stored hex-encoded.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Inverse of `hex_encode`. None on odd length or a non-hex digit.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Drop bookmarks whose path is no longer in the recents list, so the
/// map cannot outgrow the eight entries it mirrors.
pub fn prune(map: &mut BTreeMap<String, String>, recents: &[String]) {
    map.retain(|path, _| recents.iter().any(|r| r == path));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn hex_roundtrips_arbitrary_bytes() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn hex_encode_is_lowercase_and_double_width() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn hex_decode_rejects_odd_length_and_non_hex() {
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn prune_drops_bookmarks_with_no_matching_recent() {
        let mut map = BTreeMap::from([
            ("/a".to_string(), "00".to_string()),
            ("/b".to_string(), "11".to_string()),
        ]);
        prune(&mut map, &["/a".to_string()]);
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["/a"]);
    }

    #[test]
    fn prune_keeps_everything_when_all_are_recent() {
        let mut map = BTreeMap::from([("/a".to_string(), "00".to_string())]);
        prune(&mut map, &["/a".to_string(), "/b".to_string()]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn scope_is_only_needed_in_the_sandboxed_build() {
        assert_eq!(needs_scope(), cfg!(all(target_os = "macos", feature = "mas")));
    }
}
