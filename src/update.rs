//! Update check: compare the running version against the latest
//! GitHub release and surface a quiet "update available" affordance.
//! No self-replacement — clicking opens the releases page.

pub const RELEASES_URL: &str = "https://github.com/SuperJackfruitLabs/supermd/releases/latest";
const LATEST_API: &str =
    "https://api.github.com/repos/SuperJackfruitLabs/supermd/releases/latest";

/// Compare dotted-numeric versions ("0.0.4" vs "0.1.0"); tolerant of a
/// leading `v` and unequal segment counts. Non-numeric segments end the
/// comparison (treated as equal from there).
pub fn is_newer(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map_while(|seg| seg.parse::<u64>().ok())
            .collect()
    };
    let (cur, new) = (parse(current), parse(latest));
    for i in 0..cur.len().max(new.len()) {
        let c = cur.get(i).copied().unwrap_or(0);
        let n = new.get(i).copied().unwrap_or(0);
        if n != c {
            return n > c;
        }
    }
    false
}

/// Extract `tag_name` from the GitHub latest-release JSON without a
/// JSON dependency — the field is a plain string.
pub fn parse_tag(json: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let at = json.find(key)? + key.len();
    let rest = &json[at..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    let tag = &rest[start..end];
    (!tag.is_empty()).then(|| tag.to_string())
}

/// Blocking: fetch the latest release tag (e.g. "v0.0.5"). Runs on the
/// background executor; any failure is a silent None.
pub fn fetch_latest_tag() -> Option<String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "10", "-H", "User-Agent: supermd", LATEST_API])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_tag(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.0.4", "0.0.5"));
        assert!(is_newer("0.0.4", "v0.1.0"));
        assert!(is_newer("0.9.0", "1.0.0"));
        assert!(is_newer("0.0.4", "0.0.4.1"));
        assert!(!is_newer("0.0.4", "0.0.4"));
        assert!(!is_newer("0.0.5", "0.0.4"));
        assert!(!is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.0.4", "not-a-version"));
    }

    #[test]
    fn tag_parsing() {
        let json = r#"{"url":"…","tag_name":"v0.0.5","name":"SuperMD v0.0.5"}"#;
        assert_eq!(parse_tag(json), Some("v0.0.5".to_string()));
        assert_eq!(parse_tag("{}"), None);
        assert_eq!(parse_tag(r#"{"tag_name":""}"#), None);
    }
}
