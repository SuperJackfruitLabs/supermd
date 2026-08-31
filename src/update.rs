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

/// The App Store distributes updates itself, and the sandbox blocks the
/// curl subprocess this check uses. Off in the MAS build.
pub fn checks_enabled() -> bool {
    !cfg!(feature = "mas")
}

/// Blocking: fetch the latest release tag (e.g. "v0.0.5"). Runs on the
/// background executor; any failure is a silent None.
pub fn fetch_latest_tag() -> Option<String> {
    if !checks_enabled() {
        return None;
    }
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "10", "-H", "User-Agent: supermd", LATEST_API])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_tag(&String::from_utf8_lossy(&out.stdout))
}

/// What the About dialog should say about updates, given the running
/// version and whatever a check has turned up so far. Pure, so the
/// wording and the download affordance are testable without a window.
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    /// A check is in flight.
    Checking,
    /// No check has answered yet.
    Unknown,
    /// The running version is the latest known.
    UpToDate,
    /// A newer release exists; carries its tag for the download button.
    Available(String),
}

pub fn update_status(current: &str, latest: Option<&str>, checking: bool) -> UpdateStatus {
    if checking {
        return UpdateStatus::Checking;
    }
    match latest {
        None => UpdateStatus::Unknown,
        Some(tag) if is_newer(current, tag) => UpdateStatus::Available(tag.to_string()),
        Some(_) => UpdateStatus::UpToDate,
    }
}

impl UpdateStatus {
    /// The line shown under the version number.
    pub fn message(&self) -> String {
        match self {
            UpdateStatus::Checking => "Checking for updates…".to_string(),
            UpdateStatus::Unknown => "Check for updates".to_string(),
            UpdateStatus::UpToDate => "You're up to date".to_string(),
            UpdateStatus::Available(tag) => format!("Version {tag} is available"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_checks_are_off_in_the_app_store_build() {
        assert_eq!(checks_enabled(), !cfg!(feature = "mas"));
    }

    #[test]
    fn fetch_returns_none_without_checks() {
        if !checks_enabled() {
            assert_eq!(fetch_latest_tag(), None);
        }
    }

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
    fn update_status_reports_what_the_about_dialog_should_say() {
        assert_eq!(update_status("0.0.12", None, true), UpdateStatus::Checking);
        assert_eq!(update_status("0.0.12", None, false), UpdateStatus::Unknown);
        assert_eq!(
            update_status("0.0.12", Some("v0.0.12"), false),
            UpdateStatus::UpToDate
        );
        assert_eq!(
            update_status("0.0.12", Some("v0.0.13"), false),
            UpdateStatus::Available("v0.0.13".to_string())
        );
        // A check in flight wins over a stale answer.
        assert_eq!(update_status("0.0.12", Some("v0.0.13"), true), UpdateStatus::Checking);
    }

    #[test]
    fn update_status_messages_name_the_version() {
        assert_eq!(
            UpdateStatus::Available("v0.0.13".into()).message(),
            "Version v0.0.13 is available"
        );
        assert_eq!(UpdateStatus::UpToDate.message(), "You're up to date");
    }

    #[test]
    fn tag_parsing() {
        let json = r#"{"url":"…","tag_name":"v0.0.5","name":"SuperMD v0.0.5"}"#;
        assert_eq!(parse_tag(json), Some("v0.0.5".to_string()));
        assert_eq!(parse_tag("{}"), None);
        assert_eq!(parse_tag(r#"{"tag_name":""}"#), None);
    }
}
