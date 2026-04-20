// One-off utility: derive the OZ account address from a private key in an .env file.
//
// Usage:
//   cargo run -p snip36-core --example derive_address -- .env.mainnet

use starknet_core::types::Felt as CoreFelt;
use starknet_core::utils::get_contract_address;
use starknet_crypto_07::get_public_key as get_public_key_07;
use starknet_crypto_07::Felt as CryptoFelt07;
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

    let priv_crypto = CryptoFelt07::from_hex(&priv_hex).expect("invalid private key hex");
    let pub_crypto = get_public_key_07(&priv_crypto);

    let pub_core = CoreFelt::from_hex(&format!("{pub_crypto:#066x}")).unwrap();
    let class_hash = CoreFelt::from_hex(OZ_ACCOUNT_CLASS_HASH).unwrap();

    let address = get_contract_address(pub_core, class_hash, &[pub_core], CoreFelt::ZERO);

    println!("public_key:      {pub_core:#066x}");
    println!("account_address: {address:#066x}");
}
