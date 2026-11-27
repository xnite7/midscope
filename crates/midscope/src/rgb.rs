//! RGB gear follows the game: SteelSeries (GameSense), Razer (Chroma) and
//! Logitech (G HUB). All three are the vendors' own local APIs for apps;
//! nothing touches the game. One small thread: it works out the colour the
//! moment calls for, fades to it, and sends only when the colour changes.
//!
//! - menus: midnite pink
//! - agent select and in a match: your agent's colour
//! - Riot Client closed: gear goes back to its own lighting

use std::sync::Arc;
use std::time::{Duration, Instant};

use midscope_core::game::{Phase, Snapshot};
use ureq::json;

use crate::{tray, Shared};

const NEON: u32 = 0xE0309A;
const GAME: &str = "MIDSCOPE";

/// Signature colours; agents not listed (new ones) get the pink.
fn agent_color(name: &str) -> u32 {
    match name {
        "Jett" => 0x8FDCFF,
        "Phoenix" => 0xFF6A14,
        "Sage" => 0x2FE0C0,
        "Sova" => 0x2F6BFF,
        "Brimstone" => 0xFF5A1F,
        "Viper" => 0x28D050,
        "Cypher" => 0xDCE6F0,
        "Reyna" => 0xB030FF,
        "Killjoy" => 0xFFD000,
        "Breach" => 0xFF7A00,
        "Omen" => 0x4040E0,
        "Raze" => 0xFF9500,
        "Skye" => 0x70D850,
        "Yoru" => 0x2050FF,
        "Astra" => 0x8A30FF,
        "KAY/O" => 0x50A0FF,
        "Chamber" => 0xE0B040,
        "Neon" => 0x3050FF,
        "Fade" => 0x306080,
        "Harbor" => 0x00B8A8,
        "Gekko" => 0xB0F020,
        "Deadlock" => 0xC0E8F8,
        "Iso" => 0x7050FF,
        "Clove" => 0xFF70D8,
        "Vyse" => 0x9070E0,
        "Tejo" => 0xF0A030,
        "Waylay" => 0x60FFB0,
        _ => NEON,
    }
}

/// The colour for this moment; None hands the gear back.
fn wanted(snap: &Snapshot) -> Option<u32> {
    match snap.phase {
        Phase::Offline => None,
        Phase::Menus => Some(NEON),
        Phase::Pregame | Phase::Ingame => {
            Some(snap.players.iter().find(|p| p.is_self).map_or(NEON, |p| if p.agent.is_empty() { 0xF4EAF0 } else { agent_color(&p.agent) }))
        }
    }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s & 0xFF) as f32, (b >> s & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

fn http() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_millis(400)).build()
}

// ───────────────────────────── SteelSeries ─────────────────────────────

/// One event per kind of device, so a kind GG doesn't know can't break the rest.
const SS_EVENTS: [(&str, &str, &[&str]); 9] = [
    ("KEYS", "rgb-per-key-zones", &["all"]),
    ("Z1", "rgb-1-zone", &["one"]),
    ("Z2", "rgb-2-zone", &["one", "two"]),
    ("Z3", "rgb-3-zone", &["one", "two", "three"]),
    ("Z4", "rgb-4-zone", &["one", "two", "three", "four"]),
    ("Z5", "rgb-5-zone", &["one", "two", "three", "four", "five"]),
    ("Z8", "rgb-8-zone", &["one", "two", "three", "four", "five", "six", "seven", "eight"]),
    ("MOUSE", "mouse", &["wheel", "logo", "base"]),
    ("HEADSET", "headset", &["earcups"]),
];

struct SteelSeries {
    base: String,
    events: Vec<&'static str>,
}

impl SteelSeries {
    fn connect(agent: &ureq::Agent) -> Option<SteelSeries> {
        let root = std::path::PathBuf::from(std::env::var_os("PROGRAMDATA")?).join("SteelSeries");
        let props = ["SteelSeries GG", "SteelSeries Engine 3"]
            .iter()
            .find_map(|d| std::fs::read_to_string(root.join(d).join("coreProps.json")).ok())?;
        let v: ureq::serde_json::Value = ureq::serde_json::from_str(&props).ok()?;
        let base = format!("http://{}", v.get("address")?.as_str()?);
        agent
            .post(&format!("{base}/game_metadata"))
            .send_json(json!({ "game": GAME, "game_display_name": "midscope", "developer": "midnite" }))
            .ok()?;
        let mut events = Vec::new();
        for (event, device, zones) in SS_EVENTS {
            let handlers: Vec<_> = zones
                .iter()
                .map(|z| json!({ "device-type": device, "zone": z, "mode": "context-color", "context-frame-key": "c" }))
                .collect();
            let bound = agent
                .post(&format!("{base}/bind_game_event"))
                .send_json(json!({ "game": GAME, "event": event, "value_optional": true, "handlers": handlers }));
            if bound.is_ok() {
                events.push(event);
            }
        }
        tray::log(&format!("rgb: steelseries on ({} device kinds)", events.len()));
        (!events.is_empty()).then_some(SteelSeries { base, events })
    }

    fn send(&self, agent: &ureq::Agent, rgb: u32) -> bool {
        let frame = json!({ "c": { "red": rgb >> 16 & 0xFF, "green": rgb >> 8 & 0xFF, "blue": rgb & 0xFF } });
        let mut alive = false;
        for event in &self.events {
            let sent = agent
                .post(&format!("{}/game_event", self.base))
                .send_json(json!({ "game": GAME, "event": event, "data": { "value": 1, "frame": frame } }));
            // A refusal (4xx) still means GG is there; only a dead socket drops it.
            alive |= !matches!(sent, Err(ureq::Error::Transport(_)));
        }
        alive
    }

    fn release(&self, agent: &ureq::Agent) {
        let _ = agent.post(&format!("{}/stop_game", self.base)).send_json(json!({ "game": GAME }));
    }
}

// ─────────────────────────────── Razer ───────────────────────────────

struct Razer {
    uri: String,
    ready: Instant,
}

impl Razer {
    fn connect(agent: &ureq::Agent) -> Option<Razer> {
        let v: ureq::serde_json::Value = agent
            .post("http://localhost:54235/razer/chromasdk")
            .send_json(json!({
                "title": "midscope",
                "description": "lights follow your VALORANT match",
                "author": { "name": "midnite", "contact": "github.com/xnite7/midscope" },
                "device_supported": ["keyboard", "mouse", "headset", "mousepad", "keypad", "chromalink"],
                "category": "application"
            }))
            .ok()?
            .into_json()
            .ok()?;
        let uri = v.get("uri")?.as_str()?.to_owned();
        tray::log("rgb: razer on");
        // Chroma needs a moment after the handshake before it takes effects.
        Some(Razer { uri, ready: Instant::now() + Duration::from_millis(1200) })
    }

    fn send(&self, agent: &ureq::Agent, rgb: u32) -> bool {
        if Instant::now() < self.ready {
            return true;
        }
        // Chroma colours are blue-green-red.
        let bgr = (rgb & 0xFF) << 16 | (rgb & 0xFF00) | rgb >> 16 & 0xFF;
        let mut alive = false;
        for device in ["keyboard", "mouse", "mousepad", "headset", "keypad", "chromalink"] {
            let sent = agent
                .put(&format!("{}/{device}", self.uri))
                .send_json(json!({ "effect": "CHROMA_STATIC", "param": { "color": bgr } }));
            alive |= !matches!(sent, Err(ureq::Error::Transport(_)));
        }
        alive
    }

    fn heartbeat(&self, agent: &ureq::Agent) -> bool {
        agent.put(&format!("{}/heartbeat", self.uri)).call().is_ok()
    }

    fn release(&self, agent: &ureq::Agent) {
        let _ = agent.delete(&self.uri).call();
    }
}

// ────────────────────────────── Logitech ──────────────────────────────

/// G HUB's own LED library, loaded from where G HUB installs it.
struct Logitech {
    set: unsafe extern "C" fn(i32, i32, i32) -> bool,
    shutdown: unsafe extern "C" fn(),
}

impl Logitech {
    fn connect() -> Option<Logitech> {
        use windows::core::{s, HSTRING};
        use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        let root = std::path::PathBuf::from(std::env::var_os("ProgramFiles")?).join("LGHUB");
        let dll = ["sdk_legacy_led_x64.dll", "sdks\\sdk_legacy_led_x64.dll"].iter().map(|f| root.join(f)).find(|p| p.exists())?;
        unsafe {
            let lib = LoadLibraryW(&HSTRING::from(dll.as_os_str())).ok()?;
            let init: unsafe extern "C" fn() -> bool = std::mem::transmute(GetProcAddress(lib, s!("LogiLedInit"))?);
            let target: unsafe extern "C" fn(i32) -> bool = std::mem::transmute(GetProcAddress(lib, s!("LogiLedSetTargetDevice"))?);
            let set = std::mem::transmute(GetProcAddress(lib, s!("LogiLedSetLighting"))?);
            let shutdown = std::mem::transmute(GetProcAddress(lib, s!("LogiLedShutdown"))?);
            if !init() {
                return None;
            }
            target(7); // every kind of device
            tray::log("rgb: logitech on");
            Some(Logitech { set, shutdown })
        }
    }

    fn send(&self, rgb: u32) -> bool {
        let pct = |s: u32| ((rgb >> s & 0xFF) * 100 / 255) as i32;
        unsafe { (self.set)(pct(16), pct(8), pct(0)) }
    }
}

// ─────────────────────────────── the loop ───────────────────────────────

pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("rgb".into()).stack_size(256 * 1024).spawn(move || run(&shared));
}

fn run(shared: &Shared) {
    let agent = http();
    let (mut steel, mut razer, mut logi): (Option<SteelSeries>, Option<Razer>, Option<Logitech>) = (None, None, None);
    let mut looked: Option<Instant> = None;
    // The fade: from one colour to the goal, starting at `since`.
    let (mut from, mut goal, mut since) = (NEON, NEON, Instant::now());
    let mut sent: Option<u32> = None;
    let mut last_send = Instant::now();
    loop {
        let on = shared.config().rgb;
        let want = if on { wanted(&shared.snap.lock().unwrap()) } else { None };
        let Some(want) = want else {
            // Hand the gear back and idle.
            if let Some(s) = steel.take() {
                s.release(&agent);
            }
            if let Some(r) = razer.take() {
                r.release(&agent);
            }
            if let Some(l) = logi.take() {
                unsafe { (l.shutdown)() };
            }
            (sent, looked) = (None, None);
            std::thread::sleep(Duration::from_millis(1000));
            continue;
        };
        // Look for each vendor's app now and then (they may start later).
        if looked.is_none_or(|t| t.elapsed() > Duration::from_secs(15)) {
            looked = Some(Instant::now());
            let before = (steel.is_some(), razer.is_some(), logi.is_some());
            steel = steel.or_else(|| SteelSeries::connect(&agent));
            razer = razer.or_else(|| Razer::connect(&agent));
            logi = logi.or_else(Logitech::connect);
            if before != (steel.is_some(), razer.is_some(), logi.is_some()) {
                sent = None;
            }
        }
        if want != goal {
            from = sent.unwrap_or(goal);
            goal = want;
            since = Instant::now();
        }
        let t = (since.elapsed().as_secs_f32() / 0.5).min(1.0);
        let now = mix(from, goal, t);
        // Send on change; repeat every few seconds so the vendors keep our colour.
        if sent != Some(now) || last_send.elapsed() > Duration::from_secs(5) {
            if steel.as_ref().is_some_and(|s| !s.send(&agent, now)) {
                steel = None;
            }
            if razer.as_ref().is_some_and(|r| !(r.heartbeat(&agent) && r.send(&agent, now))) {
                razer = None;
            }
            if logi.as_ref().is_some_and(|l| !l.send(now)) {
                logi = None;
            }
            sent = Some(now);
            last_send = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(if t < 1.0 { 70 } else { 400 }));
    }
}
