//! First-launch install helpers: detect running from the DMG (or an
//! App-Translocated copy) and move the bundle into /Applications.

use std::path::{Path, PathBuf};

/// True when the app should offer to move itself to /Applications:
/// running from a mounted disk image or a translocated copy.
pub fn needs_install(exe: &Path) -> bool {
    exe.starts_with("/Volumes")
        || exe
            .components()
            .any(|c| c.as_os_str() == "AppTranslocation")
}

/// Nearest `.app` ancestor of the running executable.
pub fn bundle_path(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// Copy the bundle to /Applications and launch the copy. The caller
/// quits on Ok. Never touches the running copy.
pub fn move_to_applications(bundle: &Path) -> Result<(), String> {
    let dest = Path::new("/Applications/SuperMD.app");
    let run = |cmd: &str, args: &[&str]| -> Result<(), String> {
        let out = std::process::Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    };
    run("ditto", &[&bundle.to_string_lossy(), &dest.to_string_lossy()])?;
    run("open", &[&dest.to_string_lossy()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dmg_and_translocated_paths() {
        assert!(needs_install(Path::new(
            "/Volumes/SuperMD/SuperMD.app/Contents/MacOS/supermd"
        )));
        assert!(needs_install(Path::new(
            "/private/var/folders/x/AppTranslocation/9F41/d/SuperMD.app/Contents/MacOS/supermd"
        )));
        assert!(!needs_install(Path::new(
            "/Applications/SuperMD.app/Contents/MacOS/supermd"
        )));
        assert!(!needs_install(Path::new(
            "/Users/u/Projects/supermd/target/release/supermd"
        )));
    }

    #[test]
    fn bundle_path_finds_app_ancestor() {
        assert_eq!(
            bundle_path(Path::new("/Volumes/S/SuperMD.app/Contents/MacOS/supermd")),
            Some(PathBuf::from("/Volumes/S/SuperMD.app"))
        );
        assert_eq!(bundle_path(Path::new("/usr/bin/thing")), None);
    }
}
