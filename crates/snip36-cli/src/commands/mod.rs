pub mod deploy;
pub mod e2e;
pub mod e2e_coinflip;
pub mod e2e_messages;
pub mod e2e_settlement;
pub mod extract;
pub mod fund;
pub mod health;
pub mod prove;
pub mod setup;
pub mod submit;

use std::process::Output;

/// Extract a hex value (0x...) from text after a flexible key match.
///
/// Handles sncast output where "class_hash" may appear as "Class Hash",
/// "class hash", "class_hash", etc.
pub fn parse_hex_from_output(key: &str, text: &str) -> Option<String> {
    let pattern = key.replace('_', "[_ ]");
    let re = regex_lite::Regex::new(&format!("(?i){pattern}")).ok()?;
    for line in text.lines() {
        if re.is_match(line) {
            if let Some(m) = regex_lite::Regex::new(r"0x[0-9a-fA-F]+")
                .ok()?
                .find(line)
            {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

/// Extract a long hex value (50+ hex chars) from text, useful for class hashes.
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
