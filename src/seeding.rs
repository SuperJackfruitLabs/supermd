//! First-run plugin seeding: installers ship a default plugin set
//! (platform::bundled_plugins_dir); on startup any default the user
//! hasn't installed — and hasn't previously deleted — is copied into
//! ~/.supermd/plugins. A marker file records what was seeded so user
//! deletions stick and user modifications are never overwritten.

use std::path::Path;

#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeededMarker {
    #[serde(default)]
    pub entries: Vec<SeededEntry>,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeededEntry {
    pub name: String,
    pub version: String,
    pub hash: String,
}

#[derive(Debug, PartialEq)]
pub enum SeedAction {
    /// Copy a bundled plugin the user has never had.
    Install(String),
    /// Replace an untouched seeded plugin with a newer bundled one.
    Refresh(String),
}

/// True when version `a` is newer than `b` (dotted numeric compare).
fn newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    parse(a) > parse(b)
}

/// sha256 over the sorted (relative path, contents) of every file.
pub fn content_hash(dir: &Path) -> String {
    use sha2::Digest as _;
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if let (Ok(rel), Ok(bytes)) =
                (path.strip_prefix(root), std::fs::read(&path))
            {
                out.push((rel.to_string_lossy().into_owned(), bytes));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = sha2::Sha256::new();
    for (rel, bytes) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update(&bytes);
    }
    format!("{:x}", hasher.finalize())
}

/// The seeding truth table. `bundled` = (name, version, hash);
/// `installed` = (name, current content hash).
pub fn plan_seeding(
    bundled: &[(String, String, String)],
    installed: &[(String, String)],
    marker: &SeededMarker,
) -> Vec<SeedAction> {
    let mut plan = Vec::new();
    for (name, version, _hash) in bundled {
        let seeded = marker.entries.iter().find(|e| &e.name == name);
        let current = installed.iter().find(|(n, _)| n == name).map(|(_, h)| h);
        match (current, seeded) {
            // Not installed, never seeded: first contact.
            (None, None) => plan.push(SeedAction::Install(name.clone())),
            // Not installed but previously seeded: the user deleted it.
            (None, Some(_)) => {}
            // Installed and seeded: refresh only if untouched (the
            // installed hash still equals what we seeded) and the
            // bundled version is newer.
            (Some(current_hash), Some(entry)) => {
                if current_hash == &entry.hash && newer(version, &entry.version) {
                    plan.push(SeedAction::Refresh(name.clone()));
                }
            }
            // Installed but never seeded: the user's own copy.
            (Some(_), None) => {}
        }
    }
    plan
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Version from a bundled plugin's manifest ("0.0.0" when unreadable).
fn manifest_version(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("plugin.toml"))
        .ok()
        .and_then(|src| src.parse::<toml::Value>().ok())
        .and_then(|v| v.get("version").and_then(|s| s.as_str()).map(str::to_string))
        .unwrap_or_else(|| "0.0.0".to_string())
}

/// Thin I/O over `plan_seeding`: read the bundled set and the marker,
/// apply the plan, rewrite the marker. Any error degrades to a log
/// line — seeding never blocks startup.
pub fn run_seeding(bundled_dir: &Path, plugins_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(bundled_dir) else { return };
    let bundled: Vec<(String, String, String, std::path::PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let dir = e.path();
            Some((name, manifest_version(&dir), content_hash(&dir), dir))
        })
        .collect();

    let marker_path = plugins_dir.join("seeded.toml");
    let marker: SeededMarker = std::fs::read_to_string(&marker_path)
        .ok()
        .and_then(|src| toml::from_str(&src).ok())
        .unwrap_or_default();

    let installed: Vec<(String, String)> = bundled
        .iter()
        .filter(|(name, ..)| plugins_dir.join(name).is_dir())
        .map(|(name, ..)| (name.clone(), content_hash(&plugins_dir.join(name))))
        .collect();

    let plan_input: Vec<(String, String, String)> = bundled
        .iter()
        .map(|(n, v, h, _)| (n.clone(), v.clone(), h.clone()))
        .collect();
    let plan = plan_seeding(&plan_input, &installed, &marker);
    if plan.is_empty() {
        return;
    }

    let mut marker = marker;
    for action in &plan {
        let name = match action {
            SeedAction::Install(n) | SeedAction::Refresh(n) => n,
        };
        let Some((_, version, hash, src)) = bundled.iter().find(|(n, ..)| n == name) else {
            continue;
        };
        let dst = plugins_dir.join(name);
        let _ = std::fs::remove_dir_all(&dst);
        if let Err(e) = copy_dir(src, &dst) {
            eprintln!("supermd: seeding {name} failed: {e}");
            continue;
        }
        marker.entries.retain(|e| &e.name != name);
        marker.entries.push(SeededEntry {
            name: name.clone(),
            version: version.clone(),
            hash: hash.clone(),
        });
        eprintln!("supermd: seeded plugin {name} {version}");
    }
    match toml::to_string(&marker) {
        Ok(src) => {
            if let Err(e) = std::fs::create_dir_all(plugins_dir)
                .and_then(|()| std::fs::write(&marker_path, src))
            {
                eprintln!("supermd: seeding marker write failed: {e}");
            }
        }
        Err(e) => eprintln!("supermd: seeding marker encode failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(n: &str, v: &str, h: &str) -> (String, String, String) {
        (n.into(), v.into(), h.into())
    }

    fn marker(entries: &[(&str, &str, &str)]) -> SeededMarker {
        SeededMarker {
            entries: entries
                .iter()
                .map(|(n, v, h)| SeededEntry {
                    name: (*n).into(),
                    version: (*v).into(),
                    hash: (*h).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn version_compare_is_numeric() {
        assert!(newer("0.0.10", "0.0.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn fresh_install_seeds_everything() {
        let plan = plan_seeding(
            &[b("dot", "1.0.0", "h1"), b("toc", "1.0.0", "h2")],
            &[],
            &SeededMarker::default(),
        );
        assert_eq!(
            plan,
            vec![SeedAction::Install("dot".into()), SeedAction::Install("toc".into())]
        );
    }

    #[test]
    fn deleted_seeded_plugin_never_returns() {
        let m = marker(&[("dot", "1.0.0", "h1")]);
        assert!(plan_seeding(&[b("dot", "1.0.0", "h1")], &[], &m).is_empty());
        // even a newer bundled version stays out once deleted
        assert!(plan_seeding(&[b("dot", "2.0.0", "h9")], &[], &m).is_empty());
    }

    #[test]
    fn untouched_plugin_refreshes_on_newer_bundled_version() {
        let m = marker(&[("dot", "1.0.0", "h1")]);
        let plan = plan_seeding(
            &[b("dot", "2.0.0", "h9")],
            &[("dot".into(), "h1".into())],
            &m,
        );
        assert_eq!(plan, vec![SeedAction::Refresh("dot".into())]);
        // same version → nothing to do
        assert!(plan_seeding(&[b("dot", "1.0.0", "h1")], &[("dot".into(), "h1".into())], &m)
            .is_empty());
    }

    #[test]
    fn user_modified_plugin_is_never_touched() {
        let m = marker(&[("dot", "1.0.0", "h1")]);
        assert!(plan_seeding(
            &[b("dot", "2.0.0", "h9")],
            &[("dot".into(), "hX".into())],
            &m
        )
        .is_empty());
    }

    #[test]
    fn unseeded_preexisting_plugin_is_left_alone() {
        // The user installed "dot" manually before seeding existed.
        assert!(plan_seeding(
            &[b("dot", "1.0.0", "h1")],
            &[("dot".into(), "hX".into())],
            &SeededMarker::default()
        )
        .is_empty());
    }

    #[test]
    fn marker_roundtrips_through_toml() {
        let m = marker(&[("dot", "1.0.0", "h1"), ("toc", "0.1.0", "h2")]);
        let toml_src = toml::to_string(&m).unwrap();
        let back: SeededMarker = toml::from_str(&toml_src).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn content_hash_is_stable_and_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plugin.toml"), "name=\"x\"\n").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/f.scm"), "(a)").unwrap();
        let h1 = content_hash(dir.path());
        let h2 = content_hash(dir.path());
        assert_eq!(h1, h2, "stable");
        std::fs::write(dir.path().join("sub/f.scm"), "(b)").unwrap();
        assert_ne!(h1, content_hash(dir.path()), "content-sensitive");
    }

    #[test]
    fn run_seeding_end_to_end() {
        let bundled = tempfile::tempdir().unwrap();
        for name in ["alpha", "beta"] {
            let d = bundled.path().join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("plugin.toml"), format!("name=\"{name}\"\nversion=\"1.0.0\"\n"))
                .unwrap();
            std::fs::write(d.join("plugin.wasm"), b"wasm").unwrap();
        }
        let plugins = tempfile::tempdir().unwrap();
        run_seeding(bundled.path(), plugins.path());
        assert!(plugins.path().join("alpha/plugin.toml").exists());
        assert!(plugins.path().join("beta/plugin.wasm").exists());
        assert!(plugins.path().join("seeded.toml").exists());
        // delete one and rerun: it stays gone
        std::fs::remove_dir_all(plugins.path().join("alpha")).unwrap();
        run_seeding(bundled.path(), plugins.path());
        assert!(!plugins.path().join("alpha").exists(), "deletion sticks");
        assert!(plugins.path().join("beta/plugin.toml").exists());
    }
}
