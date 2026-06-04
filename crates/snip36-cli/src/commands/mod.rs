pub mod deploy;
pub mod doctor;
pub mod extract;
pub mod fund;
pub mod prove;
pub mod setup;
pub mod submit;

use std::process::Output;

/// Warn when the prebuilt deps stamp doesn't match the release this snip36
/// build expects. Source-built deps write no stamp and stay quiet.
pub fn warn_deps_version_mismatch(config: &snip36_core::Config) {
    let Ok(tag) = std::fs::read_to_string(config.deps_dir.join(".deps-version")) else {
        return;
    };
    let tag = tag.trim();
    if !tag.is_empty() && tag != setup::EXPECTED_DEPS_TAG {
        tracing::warn!(
            "deps were provisioned from release '{tag}' but this snip36 build expects '{}'; \
             run `snip36 setup --prebuilt` to refresh if proving misbehaves",
            setup::EXPECTED_DEPS_TAG
        );
    }
}

/// Extract a hex value (0x...) from text after a flexible key match.
///
/// Handles sncast output where "class_hash" may appear as "Class Hash",
/// "class hash", "class_hash", etc.
pub fn parse_hex_from_output(key: &str, text: &str) -> Option<String> {
    let pattern = key.replace('_', "[_ ]");
    let re = regex_lite::Regex::new(&format!("(?i){pattern}")).ok()?;
    for line in text.lines() {
        if re.is_match(line) {
            if let Some(m) = regex_lite::Regex::new(r"0x[0-9a-fA-F]+").ok()?.find(line) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

/// Extract a long hex value (50+ hex chars) from text, useful for class hashes.
#[allow(dead_code)]
pub fn parse_long_hex(text: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"0x[0-9a-fA-F]{50,}").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

/// Format a subprocess output for error reporting.
pub fn format_cmd_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        format!("stdout: {stdout}\nstderr: {stderr}")
    } else {
        stdout.to_string()
    }
}
