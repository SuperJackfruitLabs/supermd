//! Paste enricher: a pasted bare https URL becomes a titled markdown
//! link. Runs asynchronously after the paste (net-capable plugins
//! never block the paste path); each new domain prompts a one-time
//! consent banner.

wit_bindgen::generate!({ path: "../wit-v3", world: "extension" });

use supermd::extension::host_api;
use supermd::extension::types as t;

/// The pasted text iff it is exactly one bare https URL (http is left
/// alone — no silent upgrades).
pub fn bare_https_url(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    (trimmed.starts_with("https://")
        && trimmed.len() > "https://".len()
        && !trimmed.contains(char::is_whitespace))
    .then_some(trimmed)
}

/// The <title> of an HTML page: entity-unescaped, whitespace-
/// collapsed, capped at 200 chars.
pub fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open_end = start + html[start..].find('>')? + 1;
    let close = open_end + lower[open_end..].find("</title")?;
    let unescaped = html[open_end..close]
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    let cleaned = unescaped.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty()).then(|| cleaned.chars().take(200).collect())
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_: String, _: String, _: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }

    fn run_command(_: String, _: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }

    fn render_inline(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn format_document(d: String) -> Result<String, String> {
        Ok(d)
    }

    fn process_paste(text: String) -> Result<Option<String>, String> {
        let Some(url) = bare_https_url(&text) else {
            return Ok(None);
        };
        // Consent-shaped fetch errors propagate to raise the banner.
        let resp = host_api::fetch(&host_api::FetchRequest {
            method: "GET".into(),
            url: url.into(),
            headers: vec![("accept".into(), "text/html".into())],
            body: None,
        })?;
        if resp.status != 200 {
            return Ok(None);
        }
        let html = String::from_utf8_lossy(&resp.body);
        Ok(extract_title(&html).map(|title| format!("[{title}]({url})")))
    }

    fn export_document(_: String, _: String, _: t::Theme) -> Result<Vec<ExportFile>, String> {
        Err("unused".into())
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::{bare_https_url, extract_title};

    #[test]
    fn detects_only_single_bare_https_urls() {
        assert_eq!(bare_https_url("https://a.com/x"), Some("https://a.com/x"));
        assert_eq!(bare_https_url("  https://a.com/x \n"), Some("https://a.com/x"));
        assert_eq!(bare_https_url("http://a.com/x"), None); // http left alone
        assert_eq!(bare_https_url("see https://a.com"), None); // not bare
        assert_eq!(bare_https_url("https://"), None);
        assert_eq!(bare_https_url("hello"), None);
    }

    #[test]
    fn extracts_and_cleans_titles() {
        assert_eq!(
            extract_title("<html><head><title>Hi &amp; Bye</title></head></html>"),
            Some("Hi & Bye".to_string())
        );
        assert_eq!(
            extract_title("<TITLE>\n  Spaced\n  Out </TITLE>"),
            Some("Spaced Out".to_string())
        );
        assert_eq!(extract_title("<html>no title</html>"), None);
        assert_eq!(extract_title("<title></title>"), None);
    }
}
