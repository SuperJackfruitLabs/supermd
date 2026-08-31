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

// ── download + validate + install ─────────────────────────────────────

/// The only path to the network for catalog/zip fetches. Host-side and
/// user-initiated (no plugin sandbox involved); tests inject a mock.
pub type Fetcher = std::sync::Arc<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>;

const FETCH_LIMIT_BYTES: u64 = 20 * 1024 * 1024;

pub fn ureq_fetcher() -> Fetcher {
    std::sync::Arc::new(|url: &str| {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build();
        let agent: ureq::Agent = config.into();
        let response = agent.get(url).call().map_err(|e| e.to_string())?;
        use std::io::Read as _;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(FETCH_LIMIT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > FETCH_LIMIT_BYTES {
            return Err("download exceeds the 20 MB limit".to_string());
        }
        Ok(bytes)
    })
}

/// Global fetcher (ureq in the app; tests inject mocks).
pub struct CatalogFetcher(pub Fetcher);

impl gpui::Global for CatalogFetcher {}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// A plugin zip must contain exactly one top-level directory named
/// `expected_name`, traversal-free entries, a parseable manifest, and
/// the files that manifest requires.
pub fn validate_plugin_zip(bytes: &[u8], expected_name: &str) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("not a zip: {e}"))?;
    let mut roots = std::collections::BTreeSet::new();
    let mut files = std::collections::BTreeSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name is None for traversal/absolute entries.
        let Some(path) = entry.enclosed_name() else {
            return Err(format!("unsafe zip entry: {}", entry.name()));
        };
        let mut components = path.components();
        let Some(root) = components.next() else { continue };
        roots.insert(root.as_os_str().to_string_lossy().into_owned());
        if !entry.is_dir() {
            files.insert(path.to_string_lossy().into_owned());
        }
    }
    if roots.len() != 1 || !roots.contains(expected_name) {
        return Err(format!(
            "zip must contain exactly one plugin folder named {expected_name}"
        ));
    }
    let manifest_path = format!("{expected_name}/plugin.toml");
    if !files.contains(&manifest_path) {
        return Err("zip has no plugin.toml".to_string());
    }
    let mut manifest_src = String::new();
    {
        use std::io::Read as _;
        archive
            .by_name(&manifest_path)
            .map_err(|e| e.to_string())?
            .read_to_string(&mut manifest_src)
            .map_err(|e| e.to_string())?;
    }
    let meta = crate::extensions::parse_manifest(std::path::Path::new(expected_name), &manifest_src)?;
    if meta.name != expected_name {
        return Err(format!(
            "manifest name {} does not match plugin {expected_name}",
            meta.name
        ));
    }
    if crate::extensions::manifest_needs_component(&meta)
        && !files.contains(&format!("{expected_name}/plugin.wasm"))
    {
        return Err("zip is missing plugin.wasm".to_string());
    }
    for g in &meta.grammars {
        let (wasm, scm) = crate::extensions::grammar_paths(std::path::Path::new(expected_name), g);
        for required in [wasm, scm] {
            if !files.contains(&required.to_string_lossy().into_owned()) {
                return Err(format!("zip is missing {}", required.display()));
            }
        }
    }
    Ok(())
}

/// Extract the plugin name from a `supermd://install-plugin?name=X`
/// handoff. Anything else — a foreign scheme, a different host, a name
/// with path separators — is rejected outright; the name is then looked
/// up in the pinned catalog, so only known plugins can ever install.
pub fn parse_install_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("supermd://install-plugin?")?;
    let name = rest
        .split('&')
        .find_map(|kv| kv.strip_prefix("name="))?
        .trim()
        .to_string();
    let safe = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then_some(name)
}

/// Look one catalog entry up by name.
pub fn entry_by_name<'a>(entries: &'a [CatalogEntry], name: &str) -> Option<&'a CatalogEntry> {
    entries.iter().find(|e| e.name == name)
}

/// Validate and install an already-fetched plugin archive. Shared by
/// the catalog install (which fetches first) and the local Import
/// command (where the user supplied the bytes).
pub fn install_plugin_from_bytes(
    bytes: &[u8],
    name: &str,
    plugins_dir: &std::path::Path,
) -> Result<(), String> {
    let destination = plugins_dir.join(name);
    if destination.exists() {
        return Err(format!("{name} is already installed"));
    }
    validate_plugin_zip(bytes, name)?;
    let staging = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    archive.extract(staging.path()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(plugins_dir).map_err(|e| e.to_string())?;
    // Cross-device safe: try rename, fall back to a copy.
    let staged = staging.path().join(name);
    if std::fs::rename(&staged, &destination).is_err() {
        copy_tree(&staged, &destination).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The single top-level directory name inside a plugin archive — the
/// plugin's name, which `validate_plugin_zip` then cross-checks against
/// the manifest.
pub fn plugin_name_from_zip(bytes: &[u8]) -> Result<String, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut roots: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| e.to_string())?;
        let root = file.name().split('/').next().unwrap_or_default().to_string();
        if !root.is_empty() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    match roots.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err("archive must contain exactly one plugin directory".to_string()),
    }
}

/// Fetch, verify, validate, and install one catalog entry. The plugins
/// dir is only touched after everything checks out.
pub fn install_plugin(
    entry: &CatalogEntry,
    plugins_dir: &std::path::Path,
    fetch: &Fetcher,
) -> Result<(), String> {
    if !url_allowed(&entry.download) {
        return Err(format!("download URL is not from the SuperMD repo: {}", entry.download));
    }
    if plugins_dir.join(&entry.name).exists() {
        return Err(format!("{} is already installed", entry.name));
    }
    let bytes = fetch(&entry.download)?;
    if sha256_hex(&bytes) != entry.sha256 {
        return Err("download did not match the catalog checksum".to_string());
    }
    install_plugin_from_bytes(&bytes, &entry.name, plugins_dir)
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod install_tests {
    use super::*;
    use std::io::Write as _;

    /// Build an in-memory plugin zip: entries are (path, bytes).
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (path, bytes) in entries {
                writer.start_file(*path, opts).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        buf.into_inner()
    }

    fn good_zip() -> Vec<u8> {
        make_zip(&[
            ("demo/plugin.toml", b"name=\"demo\"\nversion=\"0.1.0\"\nformats=true\n"),
            ("demo/plugin.wasm", b"\0asm-stub"),
        ])
    }

    #[test]
    fn parses_a_plugin_name_out_of_an_install_url() {
        assert_eq!(
            parse_install_url("supermd://install-plugin?name=calc"),
            Some("calc".into())
        );
    }

    #[test]
    fn rejects_install_urls_that_are_not_ours() {
        assert_eq!(parse_install_url("https://evil.example.com/?name=calc"), None);
        assert_eq!(parse_install_url("supermd://open?name=calc"), None);
        assert_eq!(parse_install_url("supermd://install-plugin"), None);
    }

    #[test]
    fn rejects_a_name_with_path_separators() {
        assert_eq!(parse_install_url("supermd://install-plugin?name=../evil"), None);
        assert_eq!(parse_install_url("supermd://install-plugin?name=a/b"), None);
    }

    /// The website generates one `supermd://install-plugin?name=X` link
    /// per shipped catalog entry (examples/build_docs.rs). A name this
    /// parser rejects would render a button that silently does nothing,
    /// so every shipped name must survive the round trip.
    #[test]
    fn every_shipped_plugin_name_survives_an_install_url() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/catalog.json");
        let entries = parse_catalog(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(!entries.is_empty());
        for entry in &entries {
            let url = format!("supermd://install-plugin?name={}", entry.name);
            assert_eq!(
                parse_install_url(&url).as_deref(),
                Some(entry.name.as_str()),
                "catalog name {:?} does not survive its own install URL",
                entry.name
            );
            assert!(
                entry_by_name(&entries, &entry.name).is_some(),
                "{} must resolve back to its entry",
                entry.name
            );
        }
    }

    #[test]
    fn finds_a_catalog_entry_by_name() {
        let entries = vec![entry_for(&good_zip())];
        assert!(entry_by_name(&entries, "demo").is_some());
        assert!(entry_by_name(&entries, "nope").is_none());
    }

    #[test]
    fn installs_a_plugin_from_local_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let zip = good_zip();
        install_plugin_from_bytes(&zip, "demo", dir.path()).unwrap();
        assert!(dir.path().join("demo/plugin.toml").exists());
    }

    #[test]
    fn refuses_to_overwrite_an_installed_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let zip = good_zip();
        install_plugin_from_bytes(&zip, "demo", dir.path()).unwrap();
        let err = install_plugin_from_bytes(&zip, "demo", dir.path()).unwrap_err();
        assert!(err.contains("already installed"), "{err}");
    }

    #[test]
    fn reads_the_plugin_name_out_of_the_zip() {
        assert_eq!(plugin_name_from_zip(&good_zip()).unwrap(), "demo");
    }

    #[test]
    fn rejects_a_zip_whose_manifest_name_disagrees() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_plugin_from_bytes(&good_zip(), "other", dir.path()).is_err());
    }

    fn entry_for(bytes: &[u8]) -> CatalogEntry {
        CatalogEntry {
            name: "demo".into(),
            description: "d".into(),
            version: "0.1.0".into(),
            capabilities: vec![],
            download: "https://github.com/SuperJackfruitLabs/supermd/releases/download/v0/plugin-demo.zip".into(),
            sha256: sha256_hex(bytes),
        }
    }

    fn fetcher_of(bytes: Vec<u8>) -> (Fetcher, std::sync::Arc<std::sync::Mutex<u32>>) {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(0));
        let calls2 = calls.clone();
        let f: Fetcher = std::sync::Arc::new(move |_url| {
            *calls2.lock().unwrap() += 1;
            Ok(bytes.clone())
        });
        (f, calls)
    }

    #[test]
    fn happy_path_installs_the_plugin() {
        let zip_bytes = good_zip();
        let entry = entry_for(&zip_bytes);
        let (fetch, _) = fetcher_of(zip_bytes);
        let dir = tempfile::tempdir().unwrap();
        install_plugin(&entry, dir.path(), &fetch).unwrap();
        let manifest = std::fs::read_to_string(dir.path().join("demo/plugin.toml")).unwrap();
        assert!(manifest.contains("name=\"demo\""));
        assert!(dir.path().join("demo/plugin.wasm").exists());
    }

    #[test]
    fn zip_validation_rejects_bad_shapes() {
        // traversal entry
        let z = make_zip(&[("../evil.toml", b"x")]);
        assert!(validate_plugin_zip(&z, "demo").is_err());
        // wrong root dir name
        assert!(validate_plugin_zip(&good_zip(), "other").is_err());
        // two top-level dirs
        let z = make_zip(&[
            ("demo/plugin.toml", b"name=\"demo\"\nversion=\"0\"\nformats=true\n"),
            ("demo/plugin.wasm", b"w"),
            ("extra/file", b"x"),
        ]);
        assert!(validate_plugin_zip(&z, "demo").is_err());
        // manifest that does not parse
        let z = make_zip(&[("demo/plugin.toml", b"not toml ["), ("demo/plugin.wasm", b"w")]);
        assert!(validate_plugin_zip(&z, "demo").is_err());
        // manifest requiring a component but no plugin.wasm in the zip
        let z = make_zip(&[("demo/plugin.toml", b"name=\"demo\"\nversion=\"0\"\nformats=true\n")]);
        assert!(validate_plugin_zip(&z, "demo").is_err());
        // the good one passes
        assert!(validate_plugin_zip(&good_zip(), "demo").is_ok());
    }

    #[test]
    fn sha_mismatch_and_bad_urls_are_rejected() {
        let zip_bytes = good_zip();
        let mut entry = entry_for(&zip_bytes);
        let dir = tempfile::tempdir().unwrap();
        // altered bytes → sha mismatch, nothing installed
        entry.sha256 = "0000".into();
        let (fetch, _) = fetcher_of(zip_bytes.clone());
        assert!(install_plugin(&entry, dir.path(), &fetch).is_err());
        assert!(!dir.path().join("demo").exists());
        // foreign URL → fetcher never invoked
        let mut entry = entry_for(&zip_bytes);
        entry.download = "https://evil.example.com/plugin-demo.zip".into();
        let (fetch, calls) = fetcher_of(zip_bytes);
        assert!(install_plugin(&entry, dir.path(), &fetch).is_err());
        assert_eq!(*calls.lock().unwrap(), 0, "fetcher must not be called");
    }

    #[test]
    fn existing_destination_is_never_touched() {
        let zip_bytes = good_zip();
        let entry = entry_for(&zip_bytes);
        let (fetch, _) = fetcher_of(zip_bytes);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("demo")).unwrap();
        std::fs::write(dir.path().join("demo/user-file"), b"mine").unwrap();
        assert!(install_plugin(&entry, dir.path(), &fetch).is_err());
        assert!(dir.path().join("demo/user-file").exists(), "user content preserved");
    }
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
            "word-count", "csv-view", "daily-note", "graphql", "calc", "chart",
            "ipynb-view",
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
