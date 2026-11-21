//! Self-updates. Every push to main builds a new midscope, signs it with
//! midscope's private key (kept only in GitHub's secrets) and publishes it
//! as a GitHub release with a small `latest.json`. The app reads that file,
//! and only runs a new exe whose signature checks out against the public
//! key built in here, so nobody else can pose as an update.

use base64::Engine;
use serde::{Deserialize, Serialize};

/// midscope's update key (public half). The private half signs releases in CI.
pub const PUBLIC_KEY: &str = include_str!("../update-key.pub");

/// Where releases live. `releases/latest/download/…` always points at the newest.
pub const MANIFEST_URL: &str = "https://github.com/xnite7/midscope/releases/latest/download/latest.json";

/// This build's version: CI stamps it (0.6.<build number>); local builds use Cargo's.
pub fn current_version() -> &'static str {
    option_env!("MIDSCOPE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    /// Direct download of midscope.exe for this version.
    pub url: String,
    /// Base64 Ed25519 signature over [`message`].
    pub sig: String,
    #[serde(default)]
    pub notes: String,
}

/// What gets signed: a label, the version and the whole exe. Tying the
/// version in stops an old signed build being replayed as a "new" one.
pub fn message(version: &str, exe: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(exe.len() + 32);
    m.extend_from_slice(b"midscope-update\0");
    m.extend_from_slice(version.as_bytes());
    m.push(0);
    m.extend_from_slice(exe);
    m
}

fn key_bytes(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    (0..hex.len())
        .step_by(2)
        .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect()
}

/// True when `sig` is a valid signature of this exe and version.
pub fn verify(public_hex: &str, version: &str, exe: &[u8], sig_b64: &str) -> bool {
    let Some(pk) = key_bytes(public_hex).and_then(|b| ed25519_compact::PublicKey::from_slice(&b).ok()) else {
        return false;
    };
    let Some(sig) = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .ok()
        .and_then(|b| ed25519_compact::Signature::from_slice(&b).ok())
    else {
        return false;
    };
    pk.verify(message(version, exe), &sig).is_ok()
}

/// Signs with a hex secret key (64 bytes, as `keygen` prints it).
pub fn sign(secret_hex: &str, version: &str, exe: &[u8]) -> Option<String> {
    let sk = ed25519_compact::SecretKey::from_slice(&key_bytes(secret_hex)?).ok()?;
    let sig = sk.sign(message(version, exe), None);
    Some(base64::engine::general_purpose::STANDARD.encode(sig.as_ref()))
}

fn parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Is `candidate` a newer version than `current`? (numeric, part by part)
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.6.10", "0.6.9"));
        assert!(is_newer("v0.6.1", "0.5.1"));
        assert!(!is_newer("0.6.9", "0.6.9"));
        assert!(!is_newer("0.5.9", "0.6.0"));
    }

    #[test]
    fn signatures_bind_exe_and_version() {
        let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([7u8; 32]));
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let (sk, pk) = (hex(kp.sk.as_ref()), hex(kp.pk.as_ref()));
        let exe = b"MZ pretend exe".to_vec();
        let sig = sign(&sk, "0.6.3", &exe).unwrap();
        assert!(verify(&pk, "0.6.3", &exe, &sig));
        // A changed byte, a different version, or another key all fail.
        let mut bad = exe.clone();
        bad[3] ^= 1;
        assert!(!verify(&pk, "0.6.3", &bad, &sig));
        assert!(!verify(&pk, "0.6.4", &exe, &sig));
        let other = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([9u8; 32]));
        assert!(!verify(&hex(other.pk.as_ref()), "0.6.3", &exe, &sig));
        assert!(!verify(&pk, "0.6.3", &exe, "not base64!"));
    }
}
