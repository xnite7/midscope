//! User settings, stored as JSON in %APPDATA%\midscope\config.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Look up names of players in streamer mode. Riot has banned accounts
    /// for tools that do this, so it is off unless the user turns it on.
    pub reveal_hidden_names: bool,
    /// Show the overlay only while this key is held (virtual-key code). Tab = 0x09.
    pub hold_key: u16,
    /// Seconds between polls while in a match / in menus.
    pub poll_match_secs: u64,
    pub poll_menu_secs: u64,
    pub show_agent_select: bool,
    pub show_loading: bool,
    pub show_scoreboard: bool,
    pub show_skins: bool,
    pub show_party_lines: bool,
    /// Show midscope in screenshots and streams. Off: it's hidden from every
    /// capture, which also keeps its own screen reads clean.
    pub show_in_captures: bool,
    /// Light up SteelSeries / Razer / Logitech gear with the game.
    pub rgb: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            reveal_hidden_names: false,
            hold_key: 0x09,
            poll_match_secs: 2,
            poll_menu_secs: 5,
            show_agent_select: true,
            show_loading: true,
            show_scoreboard: true,
            show_skins: true,
            show_party_lines: true,
            show_in_captures: false,
            rgb: true,
        }
    }
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("midscope")
}

/// Carries settings and calibrated positions over from the app's old name.
pub fn migrate_old_dir() {
    let new = data_dir();
    if new.exists() {
        return;
    }
    if let Some(old) = new.parent().map(|p| p.join("recon")) {
        if old.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

impl Config {
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(data_dir());
        let _ = std::fs::write(data_dir().join("config.json"), serde_json::to_vec_pretty(self).unwrap_or_default());
    }

    pub fn load() -> Config {
        let path = data_dir().join("config.json");
        match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(c) => c,
            None => {
                let c = Config::default();
                let _ = std::fs::create_dir_all(data_dir());
                let _ = std::fs::write(&path, serde_json::to_vec_pretty(&c).unwrap_or_default());
                c
            }
        }
    }
}
