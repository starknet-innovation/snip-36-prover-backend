//! Shared utilities for prove-block route handlers.

use std::path::PathBuf;

/// Find the snip36 CLI binary.
///
/// Search order: SNIP36_CLI_BIN env, sibling of current exe (covers both
/// `cargo run` and installed layouts since both workspace binaries land in
/// the same target/{debug,release} directory), then PATH fallback.
pub fn find_snip36_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("SNIP36_CLI_BIN") {
        return PathBuf::from(bin);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cli = dir.join("snip36");
            if cli.exists() {
                return cli;
            }
        }
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        for profile in &["release", "debug"] {
            let candidate = PathBuf::from(&manifest_dir)
                .join("../../target")
                .join(profile)
                .join("snip36");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from("snip36")
}
