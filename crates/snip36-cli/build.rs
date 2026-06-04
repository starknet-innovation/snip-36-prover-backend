use std::path::Path;

fn main() {
    // Bake the pinned deps release into the binary for `setup --prebuilt` and
    // the deps-mismatch warning. Single source of truth: the `deps-version`
    // file at the repo root (also read by scripts/download-deps.sh and
    // .github/workflows/daily-health.yml).
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deps-version");
    let tag = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", file.display()));
    let tag = tag.trim();
    assert!(
        tag.strip_prefix("deps-v")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())),
        "deps-version must contain a deps-v<n> tag, got '{tag}'"
    );
    println!("cargo:rustc-env=SNIP36_EXPECTED_DEPS_TAG={tag}");
    println!("cargo:rerun-if-changed={}", file.display());
}
