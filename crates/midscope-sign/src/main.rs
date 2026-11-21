//! Release tool for CI (and for making the key once).
//!
//!   midscope-sign keygen
//!       prints a new secret key (put it in the repo's MIDSCOPE_SIGNING_KEY
//!       secret) and its public key (goes in crates/midscope-core/update-key.pub)
//!
//!   midscope-sign manifest <exe> <version> <download url> [notes]
//!       signs the exe with MIDSCOPE_SIGNING_KEY and prints latest.json

use midscope_core::update::{self, Manifest};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("keygen") => {
            let kp = ed25519_compact::KeyPair::generate();
            println!("secret {}", hex(kp.sk.as_ref()));
            println!("public {}", hex(kp.pk.as_ref()));
        }
        Some("manifest") if args.len() >= 5 => {
            let exe = std::fs::read(&args[2]).expect("read exe");
            let key = std::env::var("MIDSCOPE_SIGNING_KEY").expect("MIDSCOPE_SIGNING_KEY not set");
            let sig = update::sign(&key, &args[3], &exe).expect("bad signing key");
            assert!(update::verify(update::PUBLIC_KEY, &args[3], &exe, &sig), "signing key doesn't match update-key.pub");
            let m = Manifest { version: args[3].clone(), url: args[4].clone(), sig, notes: args.get(5).cloned().unwrap_or_default() };
            println!("{}", serde_json::to_string_pretty(&m).unwrap());
        }
        _ => {
            eprintln!("usage: midscope-sign keygen | manifest <exe> <version> <url> [notes]");
            std::process::exit(2);
        }
    }
}
