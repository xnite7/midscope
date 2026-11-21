//! Keeps midscope on the newest build, for everyone, with no prompts.
//!
//! A background thread reads `latest.json` from GitHub at start and every
//! few minutes. A newer version is downloaded and its signature checked
//! against the key built into this exe; only then is it staged. It is
//! swapped in when you're not in agent select or a match: the running exe
//! is renamed aside (Windows allows that), the new one takes its place, and
//! midscope restarts into it. The old file is cleaned up on the next start.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use midscope_core::game::Phase;
use midscope_core::update::{self, Manifest};

use crate::Shared;

/// How often to look for a new build.
const EVERY: Duration = Duration::from_secs(45);

fn log(s: &str) {
    #[cfg(windows)]
    crate::tray::log(s);
    #[cfg(not(windows))]
    eprintln!("{s}");
}

fn manifest_url() -> String {
    // Override for testing a release channel; the signature check still applies.
    std::env::var("MIDSCOPE_UPDATE_URL").unwrap_or_else(|_| update::MANIFEST_URL.to_string())
}

fn fetch_manifest() -> Option<Manifest> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    agent.get(&manifest_url()).call().ok()?.into_json().ok()
}

/// `midscope --check-update`: one check, printed, for troubleshooting.
pub fn check_once() {
    println!("current v{}  ·  channel {}", update::current_version(), manifest_url());
    match fetch_newer() {
        Some((path, v)) => println!("v{v} verified and staged at {}", path.display()),
        None => println!("nothing newer (or it failed its signature check)"),
    }
}

fn download(url: &str) -> Option<Vec<u8>> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build();
    let mut bytes = Vec::new();
    agent.get(url).call().ok()?.into_reader().take(64 << 20).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Downloads and checks a newer build; returns its staged path and version.
fn fetch_newer() -> Option<(PathBuf, String)> {
    let m = fetch_manifest()?;
    if !update::is_newer(&m.version, update::current_version()) {
        return None;
    }
    log(&format!("update: v{} found, downloading", m.version));
    let exe = download(&m.url)?;
    if !update::verify(update::PUBLIC_KEY, &m.version, &exe, &m.sig) {
        log(&format!("update: v{} failed its signature check, ignored", m.version));
        return None;
    }
    let dir = midscope_core::config::data_dir().join("update");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("midscope-{}.exe", m.version));
    std::fs::write(&path, &exe).ok()?;
    log(&format!("update: v{} verified and ready", m.version));
    Some((path, m.version))
}

/// Puts the new exe where the running one is. The running file is renamed
/// to `.old.exe` first, which Windows allows while it runs.
fn swap_in(new_exe: &PathBuf) -> std::io::Result<()> {
    let current = std::env::current_exe()?;
    let old = current.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&current, &old)?;
    if let Err(e) = std::fs::copy(new_exe, &current) {
        let _ = std::fs::rename(&old, &current); // put things back
        return Err(e);
    }
    Ok(())
}

/// Removes what the last update left behind.
pub fn clean_up() {
    if let Ok(current) = std::env::current_exe() {
        let _ = std::fs::remove_file(current.with_extension("old.exe"));
    }
    let _ = std::fs::remove_dir_all(midscope_core::config::data_dir().join("update"));
}

pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("updater".into()).stack_size(512 * 1024).spawn(move || {
        let mut ready: Option<(PathBuf, String)> = None;
        let set = |s: String| *shared.update_status.lock().unwrap() = s;
        loop {
            if ready.is_none() {
                ready = fetch_newer();
                if ready.is_none() {
                    set(format!("up to date. v{}.", update::current_version()));
                }
            }
            if let Some((_, v)) = &ready {
                set(format!("v{v} is ready. it goes in between matches."));
            }
            if let Some((path, version)) = &ready {
                // Never restart mid agent select or mid match.
                let phase = shared.snap.lock().unwrap().phase;
                if !matches!(phase, Phase::Pregame | Phase::Ingame) {
                    match swap_in(path) {
                        Ok(()) => {
                            log(&format!("update: switching to v{version}"));
                            *shared.updated_to.lock().unwrap() = Some(version.clone());
                            shared.restart.store(true, Ordering::Release);
                            return;
                        }
                        Err(e) => {
                            log(&format!("update: couldn't replace the exe ({e}); will retry"));
                            set(format!("couldn't swap in the update: {e}"));
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
            std::thread::sleep(EVERY);
        }
    });
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Starts the new exe (after this process has let go of its window and tray icon).
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--updated").spawn();
    }
}
