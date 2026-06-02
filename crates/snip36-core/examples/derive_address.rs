// One-off utility: derive the OZ account address from a private key in an .env file.
//
// Usage:
//   cargo run -p snip36-core --example derive_address -- .env.mainnet

use starknet_rust_core::types::Felt;
use starknet_rust_core::utils::get_contract_address;
use starknet_rust_crypto::get_public_key;
use std::env;
use std::fs;
use std::path::Path;

const OZ_ACCOUNT_CLASS_HASH: &str =
    "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564";

fn load_private_key(env_path: &Path) -> Option<String> {
    let content = fs::read_to_string(env_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("STARKNET_PRIVATE_KEY=") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn main() {
    let arg = env::args().nth(1).unwrap_or_else(|| ".env".to_string());
    let path = Path::new(&arg);
    let priv_hex = load_private_key(path).unwrap_or_else(|| {
        eprintln!("STARKNET_PRIVATE_KEY not found in {}", path.display());
        std::process::exit(1);
    });

    let private_key = Felt::from_hex(&priv_hex).expect("invalid private key hex");
    let public_key = get_public_key(&private_key);
    let class_hash = Felt::from_hex(OZ_ACCOUNT_CLASS_HASH).unwrap();

    let address = get_contract_address(public_key, class_hash, &[public_key], Felt::ZERO);

    println!("public_key:      {public_key:#066x}");
    println!("account_address: {address:#066x}");
}
