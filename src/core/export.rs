//! Where exported files go.
//!
//! A bundled application launched from the Finder has no meaningful working
//! directory, so exports go to a fixed, predictable folder in the user's home
//! and the interface always shows the full path it wrote.

use std::path::PathBuf;

pub fn dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("mouse-testing-exports")
}

/// Full path for a new export. `stamp` should be a caller-supplied counter or
/// time so two exports in one session do not overwrite each other.
pub fn path_for(name: &str, stamp: &str, extension: &str) -> PathBuf {
    dir().join(format!("{name}-{stamp}.{extension}"))
}

pub fn write(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// A filesystem-safe stamp from the wall clock, for naming exports.
pub fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Plain seconds since the epoch: sorts correctly, needs no date library,
    // and cannot produce a name that differs between platforms.
    format!("{secs}")
}
