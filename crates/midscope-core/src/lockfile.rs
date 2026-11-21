//! The Riot Client writes a lockfile with the port and password of its local API.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub port: u16,
    pub password: String,
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    base.join("Riot Games").join("Riot Client").join("Config").join("lockfile")
}

/// Format: `name:pid:port:password:protocol`
pub fn parse(text: &str) -> Option<Lockfile> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() < 5 {
        return None;
    }
    Some(Lockfile { port: parts[2].parse().ok()?, password: parts[3].to_string() })
}

pub fn read() -> Option<Lockfile> {
    // The client holds the file open, so read it shared and tolerate failures.
    std::fs::read_to_string(path()).ok().and_then(|t| parse(&t))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let l = super::parse("Riot Client:1234:51234:abcDEF_123:https").unwrap();
        assert_eq!(l.port, 51234);
        assert_eq!(l.password, "abcDEF_123");
        assert!(super::parse("broken").is_none());
    }
}
