//! The first-party plugin catalog: a JSON file in the repo listing
//! every installable plugin with an org-pinned download URL and a
//! sha256. Fetched only when the user opens "Install Plugins…" —
//! never in the background.

pub const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/master/plugins/catalog.json";

const ALLOWED_PREFIXES: [&str; 2] = [
    "https://github.com/SuperJackfruitLabs/supermd/",
    "https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/",
];

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub download: String,
    pub sha256: String,
}

#[derive(serde::Deserialize)]
struct CatalogFile {
    catalog_version: u32,
    plugins: Vec<CatalogEntry>,
}

/// Parse the catalog; unknown catalog versions are an error so old
/// builds fail clearly on a future format.
pub fn parse_catalog(json: &str) -> Result<Vec<CatalogEntry>, String> {
    let file: CatalogFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if file.catalog_version != 1 {
        return Err(format!(
            "catalog version {} is newer than this SuperMD understands",
            file.catalog_version
        ));
    }
    Ok(file.plugins)
}

/// Downloads may only come from our repo (exact org/repo prefix).
pub fn url_allowed(url: &str) -> bool {
    ALLOWED_PREFIXES.iter().any(|p| url.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "catalog_version": 1,
        "plugins": [
            {
                "name": "url-title",
                "description": "Pasted links gain their page title",
                "version": "0.1.0",
                "capabilities": ["net"],
                "download": "https://github.com/SuperJackfruitLabs/supermd/releases/download/v0.0.9/plugin-url-title.zip",
                "sha256": "abc"
            },
            {
                "name": "daily-note",
                "description": "Create today's journal note",
                "version": "0.1.0",
                "download": "https://github.com/SuperJackfruitLabs/supermd/releases/download/v0.0.9/plugin-daily-note.zip",
                "sha256": "def"
            }
        ]
    }"#;

    #[test]
    fn catalog_parses_and_rejects_unknown_versions() {
        let entries = parse_catalog(FIXTURE).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "url-title");
        assert_eq!(entries[0].capabilities, ["net"]);
        assert!(entries[1].capabilities.is_empty());
        let future = FIXTURE.replace("\"catalog_version\": 1", "\"catalog_version\": 9");
        assert!(parse_catalog(&future).is_err());
    }

    #[test]
    fn urls_are_org_pinned() {
        assert!(url_allowed(
            "https://github.com/SuperJackfruitLabs/supermd/releases/download/v1/x.zip"
        ));
        assert!(url_allowed(
            "https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/master/plugins/catalog.json"
        ));
        assert!(!url_allowed("https://github.com/evil/supermd/x.zip"));
        assert!(!url_allowed("http://github.com/SuperJackfruitLabs/supermd/x.zip"));
        assert!(!url_allowed("https://github.com.evil.com/SuperJackfruitLabs/supermd/x.zip"));
        assert!(!url_allowed("https://github.com/SuperJackfruitLabs/supermd-evil/x.zip"));
    }

    /// The committed catalog stays in lock-step with the plugin
    /// manifests in-repo: every entry matches a real plugin's name,
    /// version, and capabilities — and every dist plugin is listed.
    #[test]
    fn committed_catalog_matches_plugin_manifests() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let entries =
            parse_catalog(&std::fs::read_to_string(root.join("plugins/catalog.json")).unwrap())
                .unwrap();
        // dist set = build_plugins.sh CRATES + the graphql grammar copy
        let dist = [
            "dot", "toc", "emoji", "tidy", "todo-marks", "url-title", "html-export",
            "word-count", "csv-view", "daily-note", "graphql",
        ];
        for name in dist {
            assert!(entries.iter().any(|e| e.name == name), "catalog missing {name}");
        }
        for entry in &entries {
            let manifest_path = root.join("plugins").join(&entry.name).join("plugin.toml");
            let manifest: toml::Value = toml::from_str(
                &std::fs::read_to_string(&manifest_path)
                    .unwrap_or_else(|_| panic!("{} has no plugin dir", entry.name)),
            )
            .unwrap();
            assert_eq!(manifest["name"].as_str().unwrap(), entry.name);
            assert_eq!(
                manifest["version"].as_str().unwrap(),
                entry.version,
                "{} version drift",
                entry.name
            );
            let caps: Vec<String> = manifest
                .get("capabilities")
                .and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            assert_eq!(caps, entry.capabilities, "{} capability drift", entry.name);
            assert!(url_allowed(&entry.download), "{} URL not org-pinned", entry.name);
            assert!(
                entry.download.ends_with(&format!("plugin-{}.zip", entry.name)),
                "{} download name mismatch",
                entry.name
            );
        }
    }
}
