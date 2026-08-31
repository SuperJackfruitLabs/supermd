//! The objc2 half of security-scoped bookmarks. Deliberately thin: no
//! decisions live here, only the four Foundation calls. Policy is in
//! `bookmarks.rs`, which is pure and fully tested.
#![cfg(all(target_os = "macos", feature = "mas"))]

use crate::bookmarks::{hex_decode, hex_encode, Resolution};
use objc2_foundation::{NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions};
use std::path::Path;

fn url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&objc2_foundation::NSString::from_str(&path.to_string_lossy()))
}

/// Capture the current sandbox grant for `path` as a hex blob.
pub fn create(path: &Path) -> Option<String> {
    let data = url(path)
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .ok()?;
    Some(hex_encode(&data.to_vec()))
}

/// Resolve a stored blob and START accessing it. Callers must pair this
/// with `stop` when the workspace closes.
pub fn resolve(blob: &str) -> Resolution {
    let Some(bytes) = hex_decode(blob) else {
        return Resolution::Missing;
    };
    let data = objc2_foundation::NSData::with_bytes(&bytes);
    let mut stale = objc2::runtime::Bool::NO;
    // SAFETY: `stale` is a live local; the call writes at most one Bool.
    let resolved = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithSecurityScope,
            None,
            &mut stale,
        )
    };
    let Ok(u) = resolved else {
        return Resolution::Missing;
    };
    if !unsafe { u.startAccessingSecurityScopedResource() } {
        return Resolution::Missing;
    }
    let Some(path) = u.path() else {
        return Resolution::Missing;
    };
    let path = std::path::PathBuf::from(path.to_string());
    if stale.as_bool() {
        Resolution::Stale(path)
    } else {
        Resolution::Fresh(path)
    }
}

/// Release the scoped grant. Unbalanced `resolve` calls leak kernel
/// scoped-resource slots, so every open workspace stops on close.
pub fn stop(path: &Path) {
    unsafe { url(path).stopAccessingSecurityScopedResource() };
}
