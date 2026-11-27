//! The main loop. Two layered windows over the game:
//! - `overlay`: click-through, draws the info attached to the game's UI;
//! - `ui`: our settings tab and page in the main menu. Only its drawn
//!   pixels catch clicks, so everything else still goes to the game.
//! Both redraw only when what they show changes.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_F6, VK_F7, VK_F8, VK_LBUTTON, VK_LEFT, VK_NEXT,
    VK_PRIOR, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetCursorPos, GetForegroundWindow,
    GetMessageW, GetSystemMetrics, GetWindowTextW, PostQuitMessage, RegisterClassW, SetTimer, ShowWindow,
    TranslateMessage, HMENU, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_DESTROY, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use midscope_core::config::Config;
use midscope_core::game::{Phase, Snapshot};
use midscope_core::layout::{Anchor, Layout, Rect, Screen};

use crate::menu::{Action, Menu};
use crate::paint::{text, Font, Painter, Prim, CELL, MUTED, TEAL, TEXT, YELLOW};
use crate::views::{self, Seen};
use crate::{tray, Shared};

// ─────────────────────────── calibrate mode ───────────────────────────

const TARGETS: [&str; 8] =
    ["Agent card", "Card name", "Ally row", "Enemy row", "Row portrait", "Row name", "Loading ally card", "Loading enemy card"];

fn target(l: &mut Layout, i: usize) -> (&mut Rect, Option<&mut f32>) {
    let (a, b, c) = (&mut l.agent_select, &mut l.scoreboard, &mut l.loading);
    match i {
        0 => (&mut a.card.rect, Some(&mut a.step.0)),
        1 => (&mut a.name, None),
        2 => (&mut b.ally_row.rect, Some(&mut b.pitch)),
        3 => (&mut b.enemy_row.rect, Some(&mut b.pitch)),
        4 => (&mut b.icon, None),
        5 => (&mut b.name, None),
        6 => (&mut c.ally_card.rect, Some(&mut c.pitch)),
        _ => (&mut c.enemy_card.rect, Some(&mut c.pitch)),
    }
}

fn calibrate_view(layout: &Layout, sel: usize, scr: Screen, out: &mut Vec<Prim>) {
    let (a, b, l) = (&layout.agent_select, &layout.scoreboard, &layout.loading);
    let col = |i: usize| if i == sel { YELLOW } else { MUTED };
    let mut outline = |anchor: Anchor, c: u32| out.push(Prim::Outline(scr.map(anchor), c));
    for i in 0..5 {
        match sel {
            0 | 1 => {
                outline(a.card_at(i), col(0));
                outline(a.card_at(i).inner(a.name), col(1));
            }
            2..=5 => {
                for ally in [true, false] {
                    outline(b.row(ally, i), col(if ally { 2 } else { 3 }));
                    outline(b.row(ally, i).inner(b.icon), col(4));
                    outline(b.row(ally, i).inner(b.name), col(5));
                    outline(b.extra(ally, i), TEAL);
                }
            }
            _ => {
                outline(l.card(true, i), col(6));
                outline(l.card(false, i), col(7));
                outline(l.chip(true, i), TEAL);
                outline(l.chip(false, i), TEAL);
            }
        }
    }
    let s = scr.scale;
    let bar = scr.top_left(Rect { x: 360.0, y: 60.0, w: 1200.0, h: 34.0 });
    out.push(Prim::Fill(bar, CELL, 0.95));
    out.push(text(
        format!(
            "CALIBRATE: {}   ·   F8 next   ·   arrows move (Shift ×10)   ·   Ctrl+arrows size   ·   PgUp/PgDn spacing   ·   F7 save",
            TARGETS[sel].to_uppercase()
        ),
        Font::Small,
        Rect { x: bar.x + 12.0 * s, ..bar },
        TEXT,
    ));
}

// ───────────────────────────── helpers ─────────────────────────────

/// Edge-triggered key presses with auto-repeat while held.
struct Keys {
    held: [Option<Instant>; 256],
}

impl Keys {
    fn down(vk: u16) -> bool {
        unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
    }
    fn pressed(&mut self, vk: u16) -> bool {
        let now = Instant::now();
        let slot = &mut self.held[vk as usize & 0xFF];
        match (Keys::down(vk), *slot) {
            (false, _) => {
                *slot = None;
                false
            }
            (true, None) => {
                *slot = Some(now + Duration::from_millis(350));
                true
            }
            (true, Some(next)) if now >= next => {
                *slot = Some(now + Duration::from_millis(40));
                true
            }
            _ => false,
        }
    }
    /// One event per click, no auto-repeat.
    fn clicked(&mut self, vk: u16) -> bool {
        let slot = &mut self.held[vk as usize & 0xFF];
        match (Keys::down(vk), slot.is_some()) {
            (true, false) => {
                *slot = Some(Instant::now() + Duration::from_secs(3600));
                true
            }
            (false, _) => {
                *slot = None;
                false
            }
            _ => false,
        }
    }
}

/// The focused VALORANT window's client area, in screen pixels.
fn game_screen() -> Option<Screen> {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(hwnd, &mut buf);
        if !String::from_utf16_lossy(&buf[..n.max(0) as usize]).starts_with("VALORANT") {
            return None;
        }
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc).ok()?;
        let mut origin = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(hwnd, &mut origin);
        Some(Screen::new(origin.x, origin.y, rc.right - rc.left, rc.bottom - rc.top))
    }
}

fn mouse() -> (f32, f32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x as f32, p.y as f32)
}

const MA_NOACTIVATE: isize = 3;

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == tray::WM_TRAY {
        // Left or right click on the tray icon opens the menu.
        if matches!(lp.0 as u32 & 0xFFFF, 0x0202 | 0x0205 | 0x007B) {
            tray::Tray::menu(hwnd);
        }
        return LRESULT(0);
    }
    if msg == WM_MOUSEACTIVATE {
        // Clicking our settings must never take focus from the game.
        return LRESULT(MA_NOACTIVATE);
    }
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe fn make_window(ex: WINDOW_EX_STYLE, size: (i32, i32)) -> Result<HWND> {
    let instance = GetModuleHandleW(None)?;
    CreateWindowExW(
        ex | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        w!("MidscopeOverlay"),
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        size.0,
        size.1,
        None,
        HMENU::default(),
        instance,
        None,
    )
}

/// Shows `prims` on a window if `next` differs from what it shows now.
/// What a window is showing, and since when (for its fade-in).
#[derive(Default)]
struct Shown {
    fp: u64,
    since: Option<Instant>,
    alpha: u8,
}

/// Draws when the content changed. Things fade in over a few frames like the
/// game's own UI does, and go away at once.
fn present(hwnd: HWND, painter: &mut Painter, prims: &[Prim], scale: f32, next: u64, shown: &mut Shown) {
    unsafe {
        if prims.is_empty() {
            if shown.fp != next || shown.since.is_some() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            *shown = Shown { fp: next, since: None, alpha: 0 };
            return;
        }
        let since = *shown.since.get_or_insert_with(Instant::now);
        let t = (since.elapsed().as_secs_f32() / 0.10).min(1.0);
        let alpha = ((0.3 + 0.7 * (1.0 - (1.0 - t) * (1.0 - t))) * 255.0) as u8;
        if shown.fp != next {
            painter.set_scale(scale);
            match painter.paint(prims, alpha) {
                Ok(()) => {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                Err(e) => tray::log(&format!("paint failed: {e}")),
            }
            shown.fp = next;
            shown.alpha = alpha;
        } else if alpha != shown.alpha {
            painter.fade(alpha);
            shown.alpha = alpha;
        }
    }
}

/// A cheap fingerprint of whatever a view depends on.
fn fp(parts: &[u64]) -> u64 {
    parts.iter().fold(0xcbf29ce484222325, |h, p| (h ^ p).wrapping_mul(0x100000001b3))
}

fn scr_fp(s: Screen) -> u64 {
    fp(&[s.bx.to_bits() as u64, s.by.to_bits() as u64, s.bw.to_bits() as u64, s.bh.to_bits() as u64])
}

/// Screen reads tied to one phase: dropped when the phase or match changes.
#[derive(Default)]
struct Reads {
    key: u64,
    agent_select: Option<(Seen, Instant)>,
    loading: Option<Seen>,
    loading_checked: Option<Instant>,
    loading_up: bool,
    board: Option<(Seen, views::Found)>,
    /// A scoreboard read waiting for a second one to confirm it has settled.
    pending: Option<views::Found>,
    board_dm: Option<(Seen, views::FoundDm)>,
    /// When the VS screen was first read, and how many reads so far.
    loading_reads: (Option<Instant>, u8),
    /// This Tab press already has a settled read.
    fresh: bool,
    /// Bumped on every new read, so the drawing refreshes.
    matches: u64,
    pending_dm: Option<views::FoundDm>,
    tab_since: Option<Instant>,
    /// The scoreboard is really on screen this press (not the shop, etc.).
    live: bool,
    misses: u8,
    next_scan: Option<Instant>,
    /// When this phase began: the VS screen is only looked for early on.
    born: Option<Instant>,
}

// ───────────────────────────── main loop ─────────────────────────────

pub fn run(shared: Arc<Shared>) {
    if let Err(e) = run_inner(shared) {
        tray::log(&format!("overlay failed: {e}"));
    }
}

impl Reads {
    /// The VS screen is read straight away (it may still be fading in), then
    /// once more 400 ms later when it has settled.
    fn loading_due(&mut self) -> bool {
        let (first, n) = self.loading_reads;
        let due = match (first, n) {
            (None, _) => true,
            (Some(t), 1) => t.elapsed() >= Duration::from_millis(400),
            _ => false,
        };
        if due {
            self.loading_reads = (first.or(Some(Instant::now())), n + 1);
        }
        due
    }
}

/// The VS / loading screen. None when it isn't up; otherwise what was drawn.
/// It's looked for a few times a second, and only in the first two minutes
/// of a phase, so nothing is captured mid-match.
#[allow(clippy::too_many_arguments)]
fn loading_view(reads: &mut Reads, snap: &Snapshot, cfg: &Config, layout: &Layout, scr: Screen, shared: &Shared, latest: u64, prims: &mut Vec<Prim>) -> Option<u64> {
    let born = *reads.born.get_or_insert_with(Instant::now);
    let dm = snap.is_deathmatch();
    let looking = reads.loading_up || born.elapsed() < Duration::from_secs(120);
    if looking && reads.loading_checked.is_none_or(|t| t.elapsed() > Duration::from_millis(90)) {
        reads.loading_checked = Some(Instant::now());
        let up = if dm { views::loading_visible_dm(layout, scr) } else { views::loading_visible(layout, scr) };
        if up != reads.loading_up {
            tray::log(if up { "vs screen: up" } else { "vs screen: gone" });
        }
        reads.loading_up = up;
        if up && cfg.show_loading && reads.loading_due() {
            let l = &layout.loading;
            let content = shared.content();
            reads.loading = Some(if dm {
                views::read_list(snap, &content, |k| views::Probe { portrait: scr.map(views::dm_slot(layout, k).inner(l.dm_icon)), is_self: false })
            } else {
                views::read_slots(snap, &content, &[true, false], |ally, i| {
                    let card = scr.map(l.card(ally, i));
                    views::Probe {
                        portrait: scr.map(l.card(ally, i).inner(if ally { l.ally_icon } else { l.enemy_icon })),
                        // Your card's edge strip is yellow.
                        is_self: ally && views::yellow_share(Rect { x: card.x + 1.0, y: card.y + card.h * 0.3, w: 3.0, h: card.h * 0.4 }) > 0.5,
                    }
                })
            });
        }
    }
    if !reads.loading_up {
        return None;
    }
    Some(match &reads.loading {
        Some(seen) if cfg.show_loading && dm => {
            views::loading_dm(snap, seen, cfg, layout, scr, prims);
            fp(&[8, latest, scr_fp(scr), shared.config_gen(), reads.loading_reads.1 as u64])
        }
        Some(seen) if cfg.show_loading => {
            views::loading(snap, seen, cfg, layout, scr, prims);
            fp(&[6, latest, scr_fp(scr), shared.config_gen(), reads.loading_reads.1 as u64])
        }
        _ => 1,
    })
}

fn run_inner(shared: Arc<Shared>) -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let size = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(None)?.into(),
            lpszClassName: w!("MidscopeOverlay"),
            ..Default::default()
        });
        let overlay = make_window(WS_EX_TRANSPARENT, size)?;
        let ui = make_window(WINDOW_EX_STYLE(0), size)?;
        let base = size.1 as f32 / 1080.0;
        let mut overlay_painter = Painter::new(overlay, size, base)?;
        let mut ui_painter = Painter::new(ui, size, base)?;
        let mut tray = tray::Tray::new(overlay, shared.just_updated);
        let mut hidden_from_capture: Option<bool> = None;
        let mut shown_status = String::new();
        let _ = SetTimer(overlay, 1, 16, None);

        let mut layout = Layout::load();
        let mut keys = Keys { held: [None; 256] };
        let mut enabled = true;
        let mut calibrating = false;
        let mut sel = 0usize;
        let mut snap = Snapshot::empty(Phase::Offline);
        let mut latest = u64::MAX;
        let mut overlay_shown = Shown::default();
        let mut ui_shown = Shown::default();
        let mut reads = Reads::default();
        let mut menu = Menu::default();
        let mut msg = MSG::default();

        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);

            if shared.restart.load(Ordering::Acquire) {
                // A new build is in place: close cleanly so main can start it.
                PostQuitMessage(0);
            }
            let status = shared.status.lock().unwrap().clone();
            if status != shown_status {
                tray.set_status(&status);
                shown_status = status.clone();
            }
            let mut toggle_calibrate = false;
            match tray::COMMAND.swap(0, Ordering::AcqRel) {
                tray::CMD_TOGGLE => enabled = !enabled,
                tray::CMD_CALIBRATE => toggle_calibrate = true,
                tray::CMD_FOLDER => tray::Tray::open_folder(),
                tray::CMD_EXIT => PostQuitMessage(0),
                _ => {}
            }
            if keys.pressed(VK_F6.0) {
                if Keys::down(VK_CONTROL.0) && Keys::down(VK_SHIFT.0) {
                    PostQuitMessage(0);
                }
                enabled = !enabled;
            }

            let gen = shared.gen.load(Ordering::Acquire);
            if gen != latest {
                snap = shared.snap.lock().unwrap().clone();
                latest = gen;
            }
            let key = fp(&[snap.match_id.bytes().fold(0u64, |h, b| h.wrapping_mul(31) ^ b as u64), snap.phase as u64]);
            if key != reads.key {
                reads = Reads { key, ..Default::default() };
            }
            let cfg = shared.config();
            // Keep our own drawing out of screen captures (midscope's reads, and
            // screenshots / streams) unless the user wants it in them.
            let hide = !cfg.show_in_captures;
            if hidden_from_capture != Some(hide) {
                let mode = if hide { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
                let _ = SetWindowDisplayAffinity(overlay, mode);
                let _ = SetWindowDisplayAffinity(ui, mode);
                hidden_from_capture = Some(hide);
            }
            let scr = game_screen();
            let m = mouse();

            // ── settings tab + page (main menu only) ──
            let mut ui_prims = Vec::new();
            let mut ui_next = 1u64;
            if let Some(scr) = scr.filter(|_| snap.phase == Phase::Menus && !calibrating) {
                if keys.clicked(VK_LBUTTON.0) {
                    match menu.hit(m) {
                        Some(Action::Tab) => menu.open = !menu.open,
                        Some(Action::Page(p)) => menu.page = p,
                        Some(Action::Set(st, v)) => shared.update_config(|c| *st.flag(c) = v),
                        Some(Action::Calibrate) => {
                            menu.open = false;
                            toggle_calibrate = true;
                        }
                        Some(Action::ResetLayout) => {
                            layout = Layout::default();
                            layout.save();
                        }
                        Some(Action::OpenFolder) => tray::Tray::open_folder(),
                        Some(Action::Exit) => PostQuitMessage(0),
                        // A click anywhere in the game closes the page, like its own tabs.
                        None if !menu.owns(m) => menu.open = false,
                        None => {}
                    }
                }
                if menu.open && keys.pressed(VK_ESCAPE.0) {
                    menu.open = false;
                }
                let updates = format!("midscope updates itself between matches. {}", shared.update_status.lock().unwrap());
                menu.draw(&cfg, &layout, scr, m, &status, &updates, &mut ui_prims);
                ui_next = fp(&[
                    2,
                    scr_fp(scr),
                    menu.open as u64,
                    menu.page as u64,
                    menu.hover_key(m).map_or(u64::MAX, |h| h as u64),
                    shared.config_gen(),
                    status.len() as u64,
                    shared.update_status.lock().unwrap().len() as u64,
                    layout.menu.tab.rect.x.to_bits() as u64,
                ]);
                present(ui, &mut ui_painter, &ui_prims, scr.scale, ui_next, &mut ui_shown);
            } else {
                menu.open = false;
                present(ui, &mut ui_painter, &[], 1.0, ui_next, &mut ui_shown);
            }

            if keys.pressed(VK_F7.0) || toggle_calibrate {
                calibrating = !calibrating;
                if !calibrating {
                    layout.save();
                    reads = Reads { key, ..Default::default() };
                }
            }

            // ── game info overlay ──
            let mut prims = Vec::new();
            #[allow(unused_assignments)]
            let mut vs: Option<u64> = None;
            let next = match scr {
                None => 1,
                Some(scr) if calibrating => {
                    let mut changed = 0u64;
                    if keys.pressed(VK_F8.0) {
                        sel = (sel + 1) % TARGETS.len();
                    }
                    let step = if Keys::down(VK_SHIFT.0) { 10.0 } else { 1.0 };
                    let sizing = Keys::down(VK_CONTROL.0);
                    let (r, pitch) = target(&mut layout, sel);
                    for (vk, dx, dy) in [(VK_LEFT, -1.0, 0.0), (VK_RIGHT, 1.0, 0.0), (VK_UP, 0.0, -1.0), (VK_DOWN, 0.0, 1.0)] {
                        if keys.pressed(vk.0) {
                            if sizing {
                                r.w = (r.w + dx * step).max(4.0);
                                r.h = (r.h + dy * step).max(4.0);
                            } else {
                                r.x += dx * step;
                                r.y += dy * step;
                            }
                        }
                    }
                    if let Some(p) = pitch {
                        if keys.pressed(VK_PRIOR.0) {
                            *p -= step;
                        }
                        if keys.pressed(VK_NEXT.0) {
                            *p += step;
                        }
                    }
                    for v in serde_json_fp(&layout) {
                        changed = fp(&[changed, v]);
                    }
                    calibrate_view(&layout, sel, scr, &mut prims);
                    fp(&[3, scr_fp(scr), sel as u64, changed])
                }
                Some(_) if !enabled || snap.players.is_empty() => 1,
                Some(scr) => match snap.phase {
                    // The VS screen can come up while Riot still says agent select:
                    // show it (with who we know so far) instead of the cards.
                    Phase::Pregame if {
                        vs = loading_view(&mut reads, &snap, &cfg, &layout, scr, &shared, latest, &mut prims);
                        vs.is_some()
                    } => vs.unwrap_or(1),
                    Phase::Pregame if cfg.show_agent_select => {
                        // Re-read the cards every 1.5 s: portraits appear as people hover.
                        let stale = reads.agent_select.as_ref().is_none_or(|(_, t)| t.elapsed() > Duration::from_millis(600));
                        if stale {
                            let a = &layout.agent_select;
                            let seen = views::read_slots(&snap, &shared.content(), &[true], |_, i| {
                                let card = scr.map(a.card_at(i));
                                // Your card's name is yellow.
                                let name = scr.map(a.card_at(i).inner(a.name));
                                views::Probe {
                                    // The portrait: upper part of the card.
                                    portrait: Rect { x: card.x + card.w * 0.1, y: card.y, w: card.w * 0.8, h: card.h * 0.5 },
                                    is_self: views::yellow_share(name) > 0.02,
                                }
                            });
                            reads.agent_select = Some((seen, Instant::now()));
                        }
                        let seen = &reads.agent_select.as_ref().unwrap().0;
                        views::agent_select(&snap, seen, &cfg, &layout, scr, &mut prims);
                        fp(&[4, latest, scr_fp(scr), seen.len() as u64, shared.config_gen()])
                    }
                    Phase::Ingame if cfg.show_scoreboard && Keys::down(cfg.hold_key) => {
                        // The table is looked for every frame until two reads agree (it
                        // has stopped animating in), then a few times a second to follow
                        // it: rows that swap, guns that change, or the table going away
                        // (shop open). Nothing is drawn unless it's really on screen;
                        // the moment it is, the last read is drawn while the new settles.
                        let dm = snap.is_deathmatch();
                        let since = *reads.tab_since.get_or_insert_with(Instant::now);
                        let now = Instant::now();
                        let due = (!reads.fresh && since.elapsed() < Duration::from_millis(1500)) || reads.next_scan.is_none_or(|t| now >= t);
                        if due {
                            reads.next_scan = Some(now + Duration::from_millis(if reads.fresh { 300 } else { 200 }));
                            let content = shared.content();
                            let waited = since.elapsed() >= Duration::from_millis(1200);
                            let mut hit = false;
                            if dm {
                                if let Some(found) = views::locate_deathmatch(&layout, scr, snap.players.len()) {
                                    hit = true;
                                    if reads.fresh || waited || reads.pending_dm.as_ref().is_some_and(|p| found.settled_with(p)) {
                                        let seen = views::read_list(&snap, &content, |i| views::Probe {
                                            portrait: views::dm_portrait(found.rows[i]),
                                            is_self: found.self_row == Some(i),
                                        });
                                        if !reads.fresh {
                                            tray::log(&format!("deathmatch board: row {:.0},{:.0} you {:?}", found.at[0], found.at[1], found.self_row));
                                        }
                                        reads.fresh = true;
                                        reads.board_dm = Some((seen, found));
                                        reads.pending_dm = None;
                                        reads.matches += 1;
                                    } else {
                                        reads.pending_dm = Some(found);
                                    }
                                }
                            } else {
                                let mut found = views::locate_scoreboard(&layout, scr, &content);
                                if found.at.is_some() {
                                    hit = true;
                                    if reads.fresh || waited || reads.pending.as_ref().is_some_and(|p| found.settled_with(p)) {
                                        let sb = found.layout.scoreboard.clone();
                                        let seen = views::read_slots(&snap, &content, &[true, false], |ally, i| views::Probe {
                                            portrait: scr.map(sb.row(ally, i).inner(sb.icon)),
                                            is_self: ally && found.self_row == Some(i),
                                        });
                                        if !reads.fresh {
                                            tray::log(&format!("scoreboard: at {:?} you {:?} guns {}", found.at, found.self_row, found.guns.len()));
                                        }
                                        // A table that hasn't moved keeps its exact place (no wobble).
                                        if let Some((_, old)) = reads.board.take() {
                                            if reads.fresh && found.settled_with(&old) {
                                                found.layout = old.layout;
                                                found.at = old.at;
                                                found.ui = old.ui;
                                            }
                                        }
                                        reads.fresh = true;
                                        reads.board = Some((seen, found));
                                        reads.pending = None;
                                        reads.matches += 1;
                                    } else {
                                        reads.pending = Some(found);
                                    }
                                }
                            }
                            if hit {
                                reads.live = true;
                                reads.misses = 0;
                            } else {
                                reads.misses = reads.misses.saturating_add(1);
                                if reads.misses >= 2 {
                                    reads.live = false;
                                    reads.fresh = false;
                                    reads.pending = None;
                                    reads.pending_dm = None;
                                }
                            }
                        }
                        match (&reads.board, &reads.board_dm) {
                            _ if !reads.live => 1,
                            (_, Some((b, found))) if dm => {
                                views::scoreboard_dm(&snap, b, &cfg, found, scr, &mut prims);
                                fp(&[7, latest, scr_fp(scr), shared.config_gen(), reads.matches])
                            }
                            (Some((b, found)), _) if !dm => {
                                views::scoreboard(&snap, b, &cfg, found, scr, &mut prims);
                                fp(&[5, latest, scr_fp(scr), shared.config_gen(), reads.matches])
                            }
                            _ => 1,
                        }
                    }
                    Phase::Ingame => {
                        // Tab up: keep the last boards (drawn as soon as the table shows
                        // next press), minus the guns, which change every round.
                        reads.tab_since = None;
                        reads.fresh = false;
                        reads.live = false;
                        reads.misses = 0;
                        reads.next_scan = None;
                        reads.pending = None;
                        reads.pending_dm = None;
                        if let Some((_, f)) = &mut reads.board {
                            f.guns.clear();
                        }
                        loading_view(&mut reads, &snap, &cfg, &layout, scr, &shared, latest, &mut prims).unwrap_or(1)
                    }
                    _ => 1,
                },
            };
            let tab_up = reads.live && snap.phase == Phase::Ingame && !snap.is_deathmatch();
            let fscale = match (scr, reads.loading_up && snap.phase != Phase::Menus) {
                // Scoreboard text follows the table's real size.
                (Some(_), false) if tab_up && reads.board.is_some() => reads.board.as_ref().map_or(1.0, |b| b.1.ui),
                (Some(s), true) => s.lscale,
                (Some(s), false) => s.scale,
                _ => 1.0,
            };
            present(overlay, &mut overlay_painter, &prims, fscale, next, &mut overlay_shown);
        }
        Ok(())
    }
}

/// Layout numbers as bits, so calibrate repaints only when something moved.
fn serde_json_fp(l: &Layout) -> Vec<u64> {
    let r = |a: Rect| [a.x, a.y, a.w, a.h].map(|v| v.to_bits() as u64);
    let mut v = Vec::new();
    for a in [l.agent_select.card.rect, l.agent_select.name, l.scoreboard.ally_row.rect, l.scoreboard.enemy_row.rect, l.scoreboard.icon, l.scoreboard.name, l.loading.ally_card.rect, l.loading.enemy_card.rect] {
        v.extend(r(a));
    }
    v.extend([l.agent_select.step.0, l.scoreboard.pitch, l.loading.pitch].map(|x| x.to_bits() as u64));
    v
}
