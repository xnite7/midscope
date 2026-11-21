//! Where VALORANT's own UI sits, so the overlay can attach to it.
//!
//! Positions are measured against the game window's client area, not the
//! monitor, so windowed mode works. The game lays its UI out in a "UI box":
//! the whole client area, or a centred 16:9 box when the window is wider
//! than 16:9 (black bars on ultrawide). Each anchor is pinned to an edge or
//! the centre of that box, in 1920x1080 units scaled by the box size.
//! Saved in %APPDATA%\midscope\layout2.json and nudged with calibrate mode.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// A rectangle pinned to one point of the UI box.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Anchor {
    pub rect: Rect,
    pub h: Align,
    pub v: Align,
    /// Pinned inside a 16:9 box even when the window is narrower than 16:9
    /// (the match loading screen letterboxes; other screens don't).
    #[serde(default)]
    pub boxed: bool,
}

impl Anchor {
    const fn new(h: Align, v: Align, x: f32, y: f32, w: f32, hh: f32) -> Anchor {
        Anchor { rect: Rect { x, y, w, h: hh }, h, v, boxed: false }
    }
    const fn boxed(self) -> Anchor {
        Anchor { boxed: true, ..self }
    }
    /// The same anchor moved by (dx, dy) units.
    pub fn moved(self, dx: f32, dy: f32) -> Anchor {
        Anchor { rect: Rect { x: self.rect.x + dx, y: self.rect.y + dy, ..self.rect }, ..self }
    }
    /// A child rect given relative to this one's top-left.
    pub fn inner(self, r: Rect) -> Anchor {
        Anchor { rect: Rect { x: self.rect.x + r.x, y: self.rect.y + r.y, ..r }, ..self }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentSelect {
    /// First teammate card in the bottom row.
    pub card: Anchor,
    /// Step from one card to the next (x, y).
    pub step: (f32, f32),
    /// Name text inside a card (relative to the card).
    pub name: Rect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Scoreboard {
    pub ally_row: Anchor,
    pub enemy_row: Anchor,
    pub pitch: f32,
    /// Agent portrait inside a row (relative to the row).
    pub icon: Rect,
    /// Name text inside a row (relative to the row).
    pub name: Rect,
    /// Width of the columns we add to the right of the table.
    pub extra_w: f32,
}

/// The VS loading screen: allies on the left, enemies on the right.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Loading {
    pub ally_card: Anchor,
    pub enemy_card: Anchor,
    pub pitch: f32,
    pub ally_icon: Rect,
    pub enemy_icon: Rect,
    /// Name bar inside a card.
    pub name: Rect,
    /// The enemy card's red edge strip, used to tell the screen is up.
    pub enemy_strip: Rect,
    /// Width of the chip we put in the gap beside each card.
    pub chip_w: f32,
    /// Deathmatch: six cards a column, everyone teal, portrait on the left of every card.
    pub dm_left_card: Anchor,
    pub dm_right_card: Anchor,
    pub dm_pitch: f32,
    pub dm_icon: Rect,
    pub dm_name: Rect,
}

/// VALORANT's main-menu top bar, where our settings tab lives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Menu {
    /// Our icon slot, right after the store icon.
    pub tab: Anchor,
    /// The page area under the top bar (left of the social panel).
    pub page: Anchor,
}

impl Default for Menu {
    fn default() -> Self {
        // Measured on 1920x1080: nav icons every ~79 px, store at x 653.
        Menu {
            tab: Anchor::new(Start, Start, 702.0, 0.0, 60.0, 50.0),
            page: Anchor::new(Start, Start, 0.0, 50.0, 1858.0, 1030.0),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    pub agent_select: AgentSelect,
    pub scoreboard: Scoreboard,
    pub loading: Loading,
    pub menu: Menu,
}

// Measured from a screenshot: 16:9 box 1567 px wide, letterboxed.
impl Default for Loading {
    fn default() -> Self {
        Loading {
            ally_card: Anchor::new(Start, Start, 36.8, 176.4, 533.0, 148.3).boxed(),
            enemy_card: Anchor::new(Start, Start, 1345.4, 176.4, 533.0, 148.3).boxed(),
            pitch: 167.9,
            ally_icon: Rect { x: 12.3, y: 2.5, w: 112.0, h: 108.0 },
            enemy_icon: Rect { x: 408.0, y: 2.5, w: 112.0, h: 108.0 },
            name: Rect { x: 6.0, y: 118.0, w: 260.0, h: 30.0 },
            enemy_strip: Rect { x: 525.0, y: 30.0, w: 4.0, h: 4.0 },
            chip_w: 186.0,
            // Measured from a deathmatch screenshot (1920x1080).
            dm_left_card: Anchor::new(Start, Start, 34.0, 150.0, 466.0, 126.0).boxed(),
            dm_right_card: Anchor::new(Start, Start, 1412.0, 150.0, 466.0, 126.0).boxed(),
            dm_pitch: 145.6,
            dm_icon: Rect { x: 14.0, y: 2.0, w: 100.0, h: 96.0 },
            dm_name: Rect { x: 6.0, y: 100.0, w: 240.0, h: 26.0 },
        }
    }
}

impl Loading {
    /// Deathmatch card `i` (0..6) in the left or right column.
    pub fn dm_card(&self, left: bool, i: usize) -> Anchor {
        (if left { self.dm_left_card } else { self.dm_right_card }).moved(0.0, self.dm_pitch * i as f32)
    }
    /// The chip beside a deathmatch card, in the gap towards the middle.
    pub fn dm_chip(&self, left: bool, i: usize) -> Anchor {
        let c = self.dm_card(left, i);
        let x = if left { c.rect.w + 28.0 } else { -self.chip_w - 28.0 };
        c.inner(Rect { x, y: 0.0, w: self.chip_w, h: c.rect.h })
    }
    pub fn card(&self, ally: bool, i: usize) -> Anchor {
        (if ally { self.ally_card } else { self.enemy_card }).moved(0.0, self.pitch * i as f32)
    }
    /// The chip in the gap: right of ally cards, left of enemy cards.
    pub fn chip(&self, ally: bool, i: usize) -> Anchor {
        let c = self.card(ally, i);
        // A 24-unit lane next to the card carries the party line.
        let x = if ally { c.rect.w + 28.0 } else { -self.chip_w - 28.0 };
        c.inner(Rect { x, y: 0.0, w: self.chip_w, h: c.rect.h })
    }
}

use Align::*;

// Agent select and the scoreboard are measured from real screenshots.
impl Default for AgentSelect {
    fn default() -> Self {
        AgentSelect {
            card: Anchor::new(Center, End, -339.0, -239.0, 128.0, 206.0),
            step: (137.5, 0.0),
            name: Rect { x: 0.0, y: 151.0, w: 128.0, h: 24.0 },
        }
    }
}

impl Default for Scoreboard {
    fn default() -> Self {
        Scoreboard {
            ally_row: Anchor::new(Center, Center, -397.2, -198.0, 793.3, 33.6),
            enemy_row: Anchor::new(Center, Center, -397.2, 36.6, 793.3, 33.6),
            pitch: 34.8,
            icon: Rect { x: 2.0, y: 1.0, w: 31.0, h: 31.0 },
            name: Rect { x: 71.0, y: 2.0, w: 170.0, h: 29.0 },
            extra_w: 330.0,
        }
    }
}

/// Maps layout units to screen pixels for one game window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub scale: f32,
    /// The UI box in screen pixels.
    pub bx: f32,
    pub by: f32,
    pub bw: f32,
    pub bh: f32,
    /// The letterboxed 16:9 box and its scale.
    pub lscale: f32,
    pub lx: f32,
    pub ly: f32,
}

impl Screen {
    /// `x, y, w, h`: the game's client area in screen pixels.
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Screen {
        let (x, y, w, h) = (x as f32, y as f32, w as f32, h.max(1) as f32);
        let (bw, bh) = if w / h > 16.0 / 9.0 { (h * 16.0 / 9.0, h) } else { (w, h) };
        let (lw, lh) = if w / h > 16.0 / 9.0 { (h * 16.0 / 9.0, h) } else { (w, w * 9.0 / 16.0) };
        Screen {
            // The game's UI scales with window height (Unreal's shortest-side rule).
            scale: bh / 1080.0,
            bx: x + (w - bw) / 2.0,
            by: y,
            bw,
            bh,
            lscale: lw / 1920.0,
            lx: x + (w - lw) / 2.0,
            ly: y + (h - lh) / 2.0,
        }
    }

    pub fn map(&self, a: Anchor) -> Rect {
        if a.boxed {
            let r = a.rect;
            let s = self.lscale;
            return Rect { x: self.lx + r.x * s, y: self.ly + r.y * s, w: r.w * s, h: r.h * s };
        }
        let ox = self.bx + match a.h {
            Start => 0.0,
            Center => self.bw / 2.0,
            End => self.bw,
        };
        let oy = self.by + match a.v {
            Start => 0.0,
            Center => self.bh / 2.0,
            End => self.bh,
        };
        Rect { x: ox + a.rect.x * self.scale, y: oy + a.rect.y * self.scale, w: a.rect.w * self.scale, h: a.rect.h * self.scale }
    }

    /// A box in 1920x1080 units from the UI box's top-left (for calibrate text).
    pub fn top_left(&self, r: Rect) -> Rect {
        self.map(Anchor { rect: r, h: Start, v: Start, boxed: false })
    }
}

impl AgentSelect {
    pub fn card_at(&self, i: usize) -> Anchor {
        self.card.moved(self.step.0 * i as f32, self.step.1 * i as f32)
    }
}

impl Scoreboard {
    pub fn row(&self, ally: bool, i: usize) -> Anchor {
        let first = if ally { self.ally_row } else { self.enemy_row };
        first.moved(0.0, self.pitch * i as f32)
    }
    /// The added columns, flush against the table's right edge.
    pub fn extra(&self, ally: bool, i: usize) -> Anchor {
        let r = self.row(ally, i);
        r.inner(Rect { x: r.rect.w, y: 0.0, w: self.extra_w, h: r.rect.h })
    }
}

fn path() -> std::path::PathBuf {
    crate::config::data_dir().join("layout2.json")
}

impl Layout {
    pub fn load() -> Layout {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(crate::config::data_dir());
        let _ = std::fs::write(path(), serde_json::to_vec_pretty(self).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screenshot: a 1559x1016 client area under a 20 px title bar.
    #[test]
    fn cards_land_where_the_screenshot_shows_them() {
        let s = Screen::new(0, 20, 1559, 1016);
        let a = AgentSelect::default();
        let first = s.map(a.card_at(0));
        let last = s.map(a.card_at(4));
        // That window's cards sit ~9 px left of centre, so allow some slack.
        assert!((first.x - 448.0).abs() < 16.0, "first card x {}", first.x);
        assert!((first.y - 811.0).abs() < 4.0, "first card y {}", first.y);
        assert!((last.x + last.w - 1093.0).abs() < 16.0, "last card right {}", last.x + last.w);
    }

    /// Fullscreen 1080p crop: cards 127 px wide, ~137 px apart.
    #[test]
    fn cards_scale_with_height() {
        let s = Screen::new(0, 0, 1920, 1080);
        let a = AgentSelect::default();
        let (c0, c4) = (s.map(a.card_at(0)), s.map(a.card_at(4)));
        assert!((c0.w - 127.0).abs() < 3.0);
        assert!(((c4.x - c0.x) / 4.0 - 136.3).abs() < 2.0);
    }

    #[test]
    fn ultrawide_centres_a_16_9_box() {
        let s = Screen::new(0, 0, 2560, 1080);
        assert_eq!(s.scale, 1.0);
        assert_eq!(s.bx, 320.0);
        let r = s.map(Anchor::new(Start, Start, 0.0, 0.0, 10.0, 10.0));
        assert_eq!(r.x, 320.0);
    }

    /// The scoreboard screenshot: UI centred at x 765 in a 1530x1017 client.
    #[test]
    fn scoreboard_rows_match_screenshot() {
        let s = Screen::new(0, 0, 1530, 1017);
        let b = Scoreboard::default();
        let first = s.map(b.row(true, 0));
        let enemy = s.map(b.row(false, 4));
        assert!((first.x - 391.0).abs() < 3.0 && (first.y - 322.0).abs() < 3.0, "{first:?}");
        assert!((enemy.y - 674.0).abs() < 4.0, "{enemy:?}");
    }

    /// The loading screenshot: client area starts under a 22 px title bar; 16:9 box from y 80.
    #[test]
    fn loading_cards_match_screenshot() {
        let s = Screen::new(0, 22, 1567, 997);
        let l = Loading::default();
        let a = s.map(l.card(true, 1));
        let e = s.map(l.card(false, 0));
        assert!((a.x - 30.0).abs() < 3.0 && (a.y - 361.0).abs() < 4.0, "{a:?}");
        assert!((e.x - 1098.0).abs() < 3.0 && (e.y - 224.0).abs() < 3.0, "{e:?}");
    }

    #[test]
    fn rows_step_by_pitch() {
        let b = Scoreboard::default();
        assert_eq!(b.row(true, 2).rect.y, b.ally_row.rect.y + 2.0 * b.pitch);
        assert_eq!(b.extra(false, 0).rect.x, b.enemy_row.rect.x + b.enemy_row.rect.w);
    }
}
