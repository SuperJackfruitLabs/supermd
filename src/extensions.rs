//! WebAssembly extension host: manifest discovery under
//! ~/.supermd/plugins/, wasmtime component instances, and the
//! capability contract (Phase 1: pure functions only). Plugin
//! failures are data, never crashes.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub fences: Vec<String>,
    pub commands: Vec<CommandInfo>,
    pub dir: PathBuf,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Option<Vec<String>>,
    #[serde(default)]
    fences: Vec<String>,
    #[serde(default)]
    commands: Vec<CommandInfo>,
    /// Reserved for later phases — rejected explicitly so old builds
    /// give a clear error for new manifests.
    #[serde(default)]
    capabilities: Option<toml::Value>,
}

pub fn parse_manifest(dir: &Path, toml_src: &str) -> Result<PluginMeta, String> {
    let file: ManifestFile = toml::from_str(toml_src).map_err(|e| e.to_string())?;
    if file.capabilities.is_some() {
        return Err(
            "manifest declares `capabilities`, which this SuperMD version does not \
             support yet (Phase 1 plugins are pure)"
                .to_string(),
        );
    }
    let _ = (file.description, file.authors);
    Ok(PluginMeta {
        name: file.name,
        version: file.version,
        fences: file.fences,
        commands: file.commands,
        dir: dir.to_path_buf(),
    })
}

/// Scan a plugins directory: each subdir needs plugin.toml + plugin.wasm.
/// Returns loaded metas and per-directory failures (never fatal).
pub fn discover(plugins_dir: &Path) -> (Vec<PluginMeta>, Vec<(PathBuf, String)>) {
    let mut loaded = Vec::new();
    let mut failures = Vec::new();
    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(e) => e,
        Err(_) => return (loaded, failures), // no dir yet: nothing installed
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("plugin.toml");
        if !manifest_path.exists() {
            continue; // stray dir, not a plugin
        }
        let result = std::fs::read_to_string(&manifest_path)
            .map_err(|e| e.to_string())
            .and_then(|src| parse_manifest(&dir, &src))
            .and_then(|meta| {
                if dir.join("plugin.wasm").exists() {
                    Ok(meta)
                } else {
                    Err("plugin.wasm missing".to_string())
                }
            });
        match result {
            Ok(meta) => loaded.push(meta),
            Err(e) => failures.push((dir, e)),
        }
    }
    loaded.sort_by(|a, b| a.name.cmp(&b.name));
    (loaded, failures)
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn manifest_parses_contributions() {
        let m = parse_manifest(
            Path::new("/p/dot"),
            r#"
name = "dot"
version = "0.1.0"
fences = ["dot", "graphviz"]
[[commands]]
id = "dot.about"
title = "About Dot"
"#,
        )
        .unwrap();
        assert_eq!(m.name, "dot");
        assert_eq!(m.fences, ["dot", "graphviz"]);
        assert_eq!(m.commands[0].id, "dot.about");
        assert_eq!(m.commands[0].title, "About Dot");
    }

    #[test]
    fn capabilities_key_is_rejected_forward_compat() {
        let err = parse_manifest(
            Path::new("/p/x"),
            "name=\"x\"\nversion=\"0\"\ncapabilities=[\"net\"]\n",
        )
        .unwrap_err();
        assert!(err.contains("capabilities"), "{err}");
    }

    #[test]
    fn discover_collects_good_and_bad() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("plugin.toml"),
            "name=\"good\"\nversion=\"1\"\nfences=[\"x\"]\n",
        )
        .unwrap();
        std::fs::write(good.join("plugin.wasm"), b"stub").unwrap();
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("plugin.toml"), "not toml [").unwrap();
        let (ok, fail) = discover(dir.path());
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].name, "good");
        assert_eq!(fail.len(), 1);
    }

    #[test]
    fn discover_requires_wasm_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nowasm");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("plugin.toml"), "name=\"n\"\nversion=\"1\"\n").unwrap();
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert_eq!(fail.len(), 1);
        assert!(fail[0].1.contains("plugin.wasm"));
    }
}
