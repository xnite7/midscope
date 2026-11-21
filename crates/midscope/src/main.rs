//! midscope: a lightweight VALORANT companion overlay, part of midnite.
//!
//! One background thread polls the Riot Client (one cheap local call per
//! poll, Riot's servers only when the match changes). The main thread owns
//! a click-through overlay that redraws only when something changed.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use midscope_core::api::{Error, Riot};
use midscope_core::config::Config;
use midscope_core::content::Content;
use midscope_core::game::{self, Phase, Snapshot};

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod paint;
#[cfg(windows)]
mod views;
#[cfg(windows)]
mod tray;
mod update;

/// The latest snapshot plus a counter the overlay watches for changes.
pub struct Shared {
    pub snap: Mutex<Snapshot>,
    pub gen: AtomicU64,
    pub status: Mutex<String>,
    content: Mutex<Arc<Content>>,
    config: Mutex<Config>,
    config_gen: AtomicU64,
    /// Set by the updater once a new build is in place: close and relaunch.
    pub restart: AtomicBool,
    pub updated_to: Mutex<Option<String>>,
    /// What the updater is doing, for the settings page.
    pub update_status: Mutex<String>,
    /// This run started right after an update.
    pub just_updated: bool,
}

impl Shared {
    fn publish(&self, s: Snapshot) {
        *self.snap.lock().unwrap() = s;
        self.gen.fetch_add(1, Ordering::Release);
    }
    fn set_status(&self, s: impl Into<String>) {
        let s = s.into();
        let mut cur = self.status.lock().unwrap();
        if *cur != s {
            #[cfg(windows)]
            tray::log(&s);
            *cur = s;
        }
    }
    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }
    /// Bumped on every settings change, so views know to repaint.
    pub fn config_gen(&self) -> u64 {
        self.config_gen.load(Ordering::Acquire)
    }
    /// Changes a setting live and saves it.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        let mut c = self.config.lock().unwrap();
        f(&mut c);
        c.save();
        self.config_gen.fetch_add(1, Ordering::Release);
    }
    /// Agent icons and names, shared with the overlay for scoreboard matching.
    pub fn content(&self) -> Arc<Content> {
        self.content.lock().unwrap().clone()
    }
}

fn main() {
    if std::env::args().any(|a| a == "--check-update") {
        update::check_once();
        return;
    }
    let just_updated = std::env::args().any(|a| a == "--updated");
    #[cfg(windows)]
    {
        // Straight after an update the old copy may still be closing: give it a moment.
        let tries = if just_updated { 40 } else { 1 };
        let mut ok = false;
        for _ in 0..tries {
            if tray::single_instance() {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !ok {
            return; // already running: the tray icon is the first copy
        }
    }
    update::clean_up();
    #[cfg(windows)]
    std::panic::set_hook(Box::new(|info| tray::log(&format!("crash: {info}"))));
    midscope_core::config::migrate_old_dir();
    let cfg = Config::load();
    let shared = Arc::new(Shared {
        snap: Mutex::new(Snapshot::empty(Phase::Offline)),
        gen: AtomicU64::new(0),
        status: Mutex::new("Waiting for Riot Client".into()),
        content: Mutex::new(Arc::new(Content::default())),
        config: Mutex::new(cfg),
        config_gen: AtomicU64::new(0),
        restart: AtomicBool::new(false),
        updated_to: Mutex::new(None),
        update_status: Mutex::new("checking for updates.".into()),
        just_updated,
    });
    update::spawn(shared.clone());

    let worker = shared.clone();
    std::thread::Builder::new()
        .name("riot-poller".into())
        .stack_size(256 * 1024)
        .spawn(move || poll_forever(&worker))
        .expect("spawn poller");

    #[cfg(windows)]
    {
        overlay::run(shared.clone());
        if shared.restart.load(Ordering::Acquire) {
            update::relaunch();
        }
    }

    // Dev mode on other platforms: print snapshots as JSON.
    #[cfg(not(windows))]
    {
        let mut seen = 0;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let g = shared.gen.load(Ordering::Acquire);
            if g != seen {
                seen = g;
                let s = shared.snap.lock().unwrap().clone();
                println!("{}", serde_json_string(&s));
            }
        }
    }
}

#[cfg(not(windows))]
fn serde_json_string(s: &Snapshot) -> String {
    format!("{s:?}")
}

fn poll_forever(shared: &Shared) {
    loop {
        let mut riot = match Riot::connect() {
            Ok(r) => r,
            Err(e) => {
                shared.set_status(e.to_string());
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        shared.set_status("Loading content");
        let content = Arc::new(Content::load(&riot.session.client_version));
        *shared.content.lock().unwrap() = content.clone();
        midscope_core::content::ensure_rank_icons();
        shared.set_status("Connected");

        let mut last = Snapshot::empty(Phase::Offline);
        loop {
            let cfg = &shared.config();
            let phase = match game::phase(&riot) {
                // The VS screen still reads as agent select; ask the match server.
                Ok(Phase::Pregame) if game::in_core_game(&mut riot) => Phase::Ingame,
                Ok(p) => p,
                Err(Error::Net(_)) | Err(Error::NotRunning) => break, // client closed: reconnect
                Err(e) => {
                    shared.set_status(e.to_string());
                    Phase::Menus
                }
            };
            shared.set_status(match phase {
                Phase::Offline => "Connected, waiting for VALORANT",
                Phase::Menus => "Ready (in menus)",
                Phase::Pregame => "Agent select",
                Phase::Ingame => "In match (hold Tab)",
            });
            // Agent select changes every second; a running match only needs
            // one read, then a cheap check that the match id is unchanged.
            let need = match phase {
                Phase::Pregame => true,
                Phase::Ingame => last.phase != Phase::Ingame || last.players.is_empty(),
                _ => last.phase != phase,
            };
            if need {
                match game::snapshot(&mut riot, &content, cfg, phase, Some(&last)) {
                    Ok(s) => {
                        // Show the match straight away (agents, names, skins)...
                        last = s.clone();
                        shared.publish(s);
                    }
                    Err(e) => shared.set_status(format!("Error: {e}")),
                }
            }
            // ...then ranks, parties and tags, everyone looked up at once.
            // Retried next poll for anyone Riot didn't answer for (rate limits).
            if !last.players.is_empty() && game::needs_enrich(&last) {
                game::enrich(&riot, &mut last);
                shared.publish(last.clone());
            }
            let secs = match phase {
                Phase::Pregame => 1,
                Phase::Ingame => cfg.poll_match_secs,
                _ => cfg.poll_menu_secs,
            };
            std::thread::sleep(Duration::from_secs(secs));
        }
        shared.publish(Snapshot::empty(Phase::Offline));
    }
}
