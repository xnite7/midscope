//! midscope's settings, living inside VALORANT's main menu: the midnite "m"
//! sits in the game's top bar right after the store, and opens a page laid
//! out like the game's own screens (big title, side menu, rows) but dressed
//! in the midnite room: OLED black, Satoshi, a lamp low in one corner and
//! the neon's spill high in the other, pills for everything you press,
//! lowercase and dry.
//!
//! Positions are in 1080p units from the game's UI box (see layout.rs).

use midscope_core::config::Config;
use midscope_core::layout::{Layout, Rect, Screen};

use crate::paint::{text, Font, Img, Prim, DANGER, FILL, INK4, LAMP, MUTED, NEON, ROOM0, ROOM1, ROOM2, TEXT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Setting {
    AgentSelect,
    Loading,
    Scoreboard,
    Skins,
    PartyLines,
    RevealNames,
}

impl Setting {
    pub fn flag(self, c: &mut Config) -> &mut bool {
        match self {
            Setting::AgentSelect => &mut c.show_agent_select,
            Setting::Loading => &mut c.show_loading,
            Setting::Scoreboard => &mut c.show_scoreboard,
            Setting::Skins => &mut c.show_skins,
            Setting::PartyLines => &mut c.show_party_lines,
            Setting::RevealNames => &mut c.reveal_hidden_names,
        }
    }
    fn get(self, c: &Config) -> bool {
        *self.flag(&mut c.clone())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Tab,
    Page(usize),
    Set(Setting, bool),
    Calibrate,
    ResetLayout,
    OpenFolder,
    Exit,
}

/// (menu label, the one dry line under the page title)
const PAGES: [(&str, &str); 4] = [
    ("overlay", "what shows up, and where."),
    ("privacy", "the risky stuff lives here."),
    ("layout", "for when a patch moves things."),
    ("about", "the quiet part."),
];

pub struct Menu {
    pub open: bool,
    pub page: usize,
    hits: Vec<(Rect, Action)>,
    page_rect: Rect,
}

impl Default for Menu {
    fn default() -> Self {
        Menu { open: false, page: 0, hits: Vec::new(), page_rect: Rect::default() }
    }
}

fn inside(r: Rect, (x, y): (f32, f32)) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

fn status_line(status: &str) -> String {
    let s = status.to_lowercase();
    match s.as_str() {
        "ready (in menus)" => "ready. in menus.".into(),
        "riot client isn't running" => "riot client's off. nothing on tonight.".into(),
        _ => s.replace(" (", ". ").replace(')', "."),
    }
}

impl Menu {
    pub fn hit(&self, pt: (f32, f32)) -> Option<Action> {
        self.hits.iter().rev().find(|(r, _)| inside(*r, pt)).map(|(_, a)| *a)
    }
    /// Whether a click here belongs to midscope (so it shouldn't close the page).
    pub fn owns(&self, pt: (f32, f32)) -> bool {
        self.hit(pt).is_some() || (self.open && inside(self.page_rect, pt))
    }
    /// A key for "what's highlighted", so hovering only repaints on change.
    pub fn hover_key(&self, pt: (f32, f32)) -> Option<usize> {
        self.hits.iter().rposition(|(r, _)| inside(*r, pt))
    }

    pub fn draw(&mut self, cfg: &Config, layout: &Layout, scr: Screen, mouse: (f32, f32), status: &str, out: &mut Vec<Prim>) {
        self.hits.clear();
        let s = scr.scale;
        let u = |x: f32, y: f32, w: f32, h: f32| scr.top_left(Rect { x, y, w, h });

        // ── the tab in the game's top bar: the midnite m ──
        let tab = scr.map(layout.menu.tab);
        let hot = inside(tab, mouse);
        out.push(Prim::Fill(tab, ROOM0, 0.01)); // invisible, but catches the click
        let (cx, cy) = (tab.x + tab.w / 2.0, tab.y + tab.h / 2.0);
        if hot || self.open {
            // The neon's light on the wall behind the sign.
            out.push(Prim::Glow(cx, cy, 26.0 * s, NEON, if self.open { 0.55 } else { 0.35 }));
        }
        let (mw, mh) = (24.0 * s, 26.4 * s);
        out.push(Prim::Image(Img::Logo, Rect { x: cx - mw / 2.0, y: cy - mh / 2.0, w: mw, h: mh }));
        if self.open {
            out.push(Prim::Round(Rect { x: tab.x + 12.0 * s, y: tab.y + tab.h - 4.0 * s, w: tab.w - 24.0 * s, h: 3.0 * s }, 1.5 * s, TEXT, 1.0));
        }
        self.hits.push((tab, Action::Tab));
        if !self.open {
            return;
        }

        // ── the room ──
        let page = scr.map(layout.menu.page);
        self.page_rect = page;
        out.push(Prim::Fill(page, ROOM0, 1.0));
        let lamp = u(150.0, 1010.0, 0.0, 0.0);
        out.push(Prim::Glow(lamp.x, lamp.y, 560.0 * s, LAMP, 0.10));
        let spill = u(1660.0, 90.0, 0.0, 0.0);
        out.push(Prim::Glow(spill.x, spill.y, 520.0 * s, NEON, 0.07));

        // Title, like the game's big page titles, in midnite lowercase.
        out.push(Prim::Image(Img::Logo, u(46.0, 104.0, 66.0, 72.6)));
        out.push(text("midscope", Font::Title, u(126.0, 96.0, 700.0, 90.0), TEXT));
        out.push(text(format!("v{}  ·  {}", midscope_core::update::current_version(), status_line(status)), Font::Body, u(48.0, 196.0, 700.0, 26.0), MUTED));

        // Side menu: pills; the selected one gets the screen's one neon dot.
        for (i, (name, _)) in PAGES.iter().enumerate() {
            let r = u(44.0, 290.0 + 52.0 * i as f32, 236.0, 44.0);
            if i == self.page {
                out.push(Prim::Round(r, r.h / 2.0, ROOM2, 1.0));
                let d = 8.0 * s;
                out.push(Prim::Glow(r.x + 22.0 * s, r.y + r.h / 2.0, 12.0 * s, NEON, 0.6));
                out.push(Prim::Round(Rect { x: r.x + 18.0 * s, y: r.y + (r.h - d) / 2.0, w: d, h: d }, d / 2.0, NEON, 1.0));
            } else if inside(r, mouse) {
                out.push(Prim::Round(r, r.h / 2.0, FILL, 1.0));
            }
            out.push(text(*name, Font::Heading, Rect { x: r.x + 38.0 * s, ..r }, if i == self.page { TEXT } else { MUTED }));
            self.hits.push((r, Action::Page(i)));
        }

        // The page itself.
        let (title, aside) = PAGES[self.page];
        out.push(text(title, Font::Title2, u(350.0, 284.0, 900.0, 34.0), TEXT));
        out.push(text(aside, Font::Body, u(350.0, 316.0, 900.0, 24.0), MUTED));

        type Row<'a> = (&'a str, &'a str, Option<Setting>, bool);
        let rows: Vec<Row> = match self.page {
            0 => vec![
                ("agent select", "rank, level and tags above each teammate's card.", Some(Setting::AgentSelect), false),
                ("the vs screen", "rank, peak and tags next to everyone while the match loads.", Some(Setting::Loading), false),
                ("scoreboard", "hold tab. rank, skins and tags join the end of the table.", Some(Setting::Scoreboard), false),
                ("party lines", "a line between people who queued together, with duo or trio on it.", Some(Setting::PartyLines), false),
                ("weapon skins", "vandal and phantom skins.", Some(Setting::Skins), false),
            ],
            1 => vec![(
                "show hidden names",
                "names of people in streamer mode. riot has banned accounts for tools that do this. your call.",
                Some(Setting::RevealNames),
                true,
            )],
            2 => vec![("positions", "if something sits off the game's ui after a patch, line it up again.", None, false)],
            _ => vec![
                ("midscope", "reads the riot client's own api. no injection, no game memory. just looking.", None, false),
                ("hotkeys", "f6 show or hide  ·  f7 calibrate  ·  ctrl shift f6 quit", None, false),
                ("updates", "midscope updates itself, quietly, between matches. nothing to click.", None, false),
            ],
        };

        // One settings group, like couch's: a lifted card, rows split by hairlines.
        let top = 362.0;
        let rh = 76.0;
        let group = u(346.0, top, 1452.0, rh * rows.len() as f32);
        out.push(Prim::Round(group, 22.0 * s, ROOM1, 1.0));
        for (k, (label, desc, setting, warn)) in rows.iter().enumerate() {
            let r = u(346.0, top + rh * k as f32, 1452.0, rh);
            if k > 0 {
                out.push(Prim::Fill(Rect { x: r.x + 24.0 * s, y: r.y, w: r.w - 48.0 * s, h: 1.0 }, INK4, 1.0));
            }
            out.push(text(*label, Font::Heading, Rect { x: r.x + 28.0 * s, y: r.y + 14.0 * s, w: 1000.0 * s, h: 24.0 * s }, TEXT));
            out.push(text(*desc, Font::Body, Rect { x: r.x + 28.0 * s, y: r.y + 40.0 * s, w: 1150.0 * s, h: 22.0 * s }, if *warn { DANGER } else { MUTED }));
            if let Some(st) = setting {
                // on / off: a quiet pill track; the chosen side is the bright pill.
                let on = st.get(cfg);
                let track = Rect { x: r.x + r.w - 212.0 * s, y: r.y + (rh * s - 38.0 * s) / 2.0, w: 184.0 * s, h: 38.0 * s };
                out.push(Prim::Round(track, track.h / 2.0, FILL, 1.0));
                for (j, (lbl, val)) in [("on", true), ("off", false)].into_iter().enumerate() {
                    let seg = Rect { x: track.x + 3.0 * s + 89.0 * s * j as f32, y: track.y + 3.0 * s, w: 89.0 * s, h: track.h - 6.0 * s };
                    if on == val {
                        out.push(Prim::Round(seg, seg.h / 2.0, TEXT, 1.0));
                        out.push(text(lbl, Font::Label, seg, ROOM0));
                    } else {
                        if inside(seg, mouse) {
                            out.push(Prim::Round(seg, seg.h / 2.0, 0x3A3438, 1.0));
                        }
                        out.push(text(lbl, Font::Label, seg, MUTED));
                    }
                    self.hits.push((seg, Action::Set(*st, val)));
                }
            }
        }

        // Pills under the group: the brightest one is the one to press.
        let by = top + rh * rows.len() as f32 + 28.0;
        let mut pill = |x: f32, label: &str, act: Action, kind: u8| {
            let b = u(x, by, 230.0, 46.0);
            let hot = inside(b, mouse);
            let (bg, fg) = match kind {
                0 => (TEXT, ROOM0),
                1 => (if hot { 0x3A3438 } else { FILL }, TEXT),
                _ => (if hot { 0x3A2426 } else { FILL }, DANGER),
            };
            out.push(Prim::Round(b, b.h / 2.0, bg, 1.0));
            out.push(text(label, Font::Button, b, fg));
            self.hits.push((b, act));
        };
        match self.page {
            2 => {
                pill(350.0, "calibrate", Action::Calibrate, 0);
                pill(596.0, "reset positions", Action::ResetLayout, 1);
            }
            3 => {
                pill(350.0, "open folder", Action::OpenFolder, 1);
                pill(596.0, "quit midscope", Action::Exit, 2);
            }
            _ => {}
        }
    }
}
