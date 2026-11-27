//! What gets drawn on each game screen, attached to the game's own UI.

use midscope_core::config::Config;
use midscope_core::content::Content;
use midscope_core::game::{Player, Snapshot};
use midscope_core::iconmatch;
use midscope_core::scan;
use midscope_core::layout::{Layout, Rect, Screen};
use midscope_core::rank::{tier_color, tier_name, PARTY_COLORS};

use crate::capture;
use crate::paint::{self, text, Font, Prim, CELL, DANGER, INK, LAMP, MUTED, RED, ROOM0, TEAL, TEXT, YELLOW};
use crate::tray;

// ─────────────────────────── reading the screen ───────────────────────────

fn pixel(x: f32, y: f32) -> Option<[u8; 3]> {
    capture::grab(x as i32, y as i32, 1, 1).map(|p| [p[2], p[1], p[0]])
}

fn rgb_u32(c: [u8; 3]) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

/// Which player sits in each on-screen slot (card or row), found by
/// matching the slot's agent portrait. Each team has unique agents.
#[derive(Default)]
pub struct Seen {
    /// (ally?, slot index, puuid, slot background colour)
    slots: Vec<(bool, usize, String, u32)>,
}

impl Seen {
    fn who(&self, ally: bool, i: usize) -> Option<(&str, u32)> {
        self.slots.iter().find(|s| s.0 == ally && s.1 == i).map(|s| (s.2.as_str(), s.3))
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
}

/// What a caller knows about one on-screen slot.
pub struct Probe {
    /// Where the agent portrait should be, in screen pixels.
    pub portrait: Rect,
    /// The slot is highlighted as the local player (yellow).
    pub is_self: bool,
}

/// Best similarity of a captured region to a template, trying small shifts
/// (the game's UI can sit a few pixels off our anchors).
fn best_score(px: &[u8], pw: usize, ph: usize, w: usize, h: usize, tmpl: &iconmatch::Thumb) -> f32 {
    let mut best = f32::MIN;
    let (mx, my) = (pw - w, ph - h);
    for dy in [0, my / 2, my] {
        for dx in [0, mx / 2, mx] {
            let off = (dy * pw + dx) * 4;
            let t = iconmatch::thumb(&px[off..], w, h, pw * 4, true, [0.0; 3]);
            best = best.max(iconmatch::similarity(&t, tmpl));
        }
    }
    best
}

/// Works out which player is in which slot. Portraits are matched against
/// each player's agent icon (each team's agents are unique); the yellow
/// local-player slot is pinned to you; a single leftover is paired up.
pub fn read_slots(snap: &Snapshot, content: &Content, teams: &[bool], probe: impl Fn(bool, usize) -> Probe) -> Seen {
    let mut seen = Seen::default();
    // One screen grab covering every portrait (plus the search margin).
    let all: Vec<Probe> = teams
        .iter()
        .flat_map(|&ally| {
            let n = snap.players.iter().filter(|p| p.ally == ally).count();
            (0..n).map(|i| probe(ally, i)).collect::<Vec<_>>()
        })
        .collect();
    let frame = (!all.is_empty()).then(|| {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for pr in &all {
            let r = pr.portrait;
            x0 = x0.min(r.x - r.w * 0.2);
            y0 = y0.min(r.y - r.h * 0.2);
            x1 = x1.max(r.x + r.w * 1.2);
            y1 = y1.max(r.y + r.h * 1.2);
        }
        capture::Frame::grab(x0 as i32, y0 as i32, (x1 - x0) as i32 + 2, (y1 - y0) as i32 + 2)
    }).flatten();
    for &ally in teams {
        let team: Vec<&Player> = snap.players.iter().filter(|p| p.ally == ally).collect();
        let mut scores: Vec<Vec<Option<f32>>> = Vec::new();
        let mut bgs = Vec::new();
        let mut self_slot = None;
        for i in 0..team.len() {
            let Probe { portrait: r, is_self } = probe(ally, i);
            if is_self && ally {
                self_slot = Some(i);
            }
            let bg = frame.as_ref().and_then(|f| f.rgb(r.x + r.w - 2.0, r.y + 2.0)).unwrap_or([31, 39, 49]);
            bgs.push(rgb_u32(bg));
            let bgf = [bg[0] as f32, bg[1] as f32, bg[2] as f32];
            // Capture with a ~12% margin so shifted portraits still line up.
            let (w, h) = (r.w.max(4.0) as usize, r.h.max(4.0) as usize);
            let (m_x, m_y) = ((r.w * 0.12) as usize, (r.h * 0.12) as usize);
            let (pw, ph) = (w + 2 * m_x, h + 2 * m_y);
            let grabbed = frame.as_ref().and_then(|f| f.crop(r.x as i32 - m_x as i32, r.y as i32 - m_y as i32, pw as i32, ph as i32));
            scores.push(
                team.iter()
                    .map(|p| {
                        let px = grabbed.as_ref()?;
                        content.icon_thumb(&p.agent, bgf).map(|t| best_score(px, pw, ph, w, h, &t))
                    })
                    .collect(),
            );
        }
        // You are always the yellow slot.
        if let (Some(slot), Some(me)) = (self_slot, team.iter().position(|p| p.is_self)) {
            for (i, row) in scores.iter_mut().enumerate() {
                for (c, sc) in row.iter_mut().enumerate() {
                    if (i == slot) != (c == me) {
                        *sc = None;
                    } else if i == slot {
                        *sc = Some(2.0);
                    }
                }
            }
        }
        let mut hits = iconmatch::assign_scores(&scores, team.len(), 0.0);
        let free_slots: Vec<usize> = (0..hits.len()).filter(|&i| hits[i].is_none()).collect();
        let free_players: Vec<usize> = (0..team.len()).filter(|c| !hits.contains(&Some(*c))).collect();
        if free_slots.len() == 1 && free_players.len() == 1 {
            hits[free_slots[0]] = Some(free_players[0]);
        }
        let desc: Vec<String> = hits
            .iter()
            .enumerate()
            .map(|(i, h)| match h {
                Some(c) => format!("{}={}({:.2})", i, team[*c].agent, scores[i][*c].unwrap_or(0.0)),
                None => format!("{i}=?"),
            })
            .collect();
        tray::log(&format!("{} slots: {}", if ally { "ally" } else { "enemy" }, desc.join(" ")));
        for (i, h) in hits.into_iter().enumerate() {
            if let Some(c) = h {
                seen.slots.push((ally, i, team[c].puuid.clone(), bgs[i]));
            }
        }
    }
    seen
}

/// Free-for-all: every player is a candidate for every slot (stored as
/// "ally" slots 0..n). Reuses the team matcher on a teamless copy.
pub fn read_list(snap: &Snapshot, content: &Content, probe: impl Fn(usize) -> Probe) -> Seen {
    let mut all = snap.clone();
    all.players.iter_mut().for_each(|p| p.ally = true);
    read_slots(&all, content, &[true], |_, i| probe(i))
}

/// Is this pixel the game's local-player yellow?
pub fn is_yellow([r, g, b]: [u8; 3]) -> bool {
    r > 180 && g > 150 && (b as i32) < (g as i32) - 50
}

/// Share of yellow pixels in a screen region (yellow name text, outlines).
pub fn yellow_share(r: Rect) -> f32 {
    let Some(px) = capture::grab(r.x as i32, r.y as i32, r.w as i32, r.h as i32) else { return 0.0 };
    let n = px.len() / 4;
    px.chunks(4).filter(|p| is_yellow([p[2], p[1], p[0]])).count() as f32 / n.max(1) as f32
}

// ─────────────────────── finding the scoreboard ───────────────────────

/// The scoreboard as found on screen this Tab press.
pub struct Found {
    pub layout: Layout,
    /// Your row in the ally block, from its yellow outline.
    pub self_row: Option<usize>,
    /// Each row's own colour (for patches that must hide text underneath).
    pub bg: [[u32; 5]; 2],
    /// Where it was found on screen (left, ally top, enemy top, row height), to
    /// tell when the scoreboard has stopped animating in.
    pub at: Option<[f32; 4]>,
    /// Pixels per layout unit for this table (its real size on screen).
    pub ui: f32,
    /// Vandals and Phantoms in the loadout column.
    pub guns: Vec<Gun>,
}

/// A gun seen in a row's loadout cell.
pub struct Gun {
    pub ally: bool,
    pub row: usize,
    pub name: String,
    /// The silhouette's box on screen.
    pub at: Rect,
    /// The loadout cell on screen, and pixels that paint the silhouette out
    /// with the row's own colours (see-through everywhere else).
    pub cell: Rect,
    pub cover: std::sync::Arc<paint::Patch>,
}

impl Found {
    /// Same place as another read (within a pixel and a half).
    pub fn settled_with(&self, other: &Found) -> bool {
        match (self.at, other.at) {
            (Some(a), Some(b)) => a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.5),
            _ => false,
        }
    }
}

/// Captures around where the table should be, finds its real position from
/// the row colours (see midscope_core::scan) and returns the layout
/// corrected to match: rows, width, portraits and names all follow.
pub fn locate_scoreboard(layout: &Layout, scr: Screen, content: &Content) -> Found {
    let sb = &layout.scoreboard;
    let s = scr.scale;
    let est_ally = scr.map(sb.row(true, 0));
    let est_enemy = scr.map(sb.row(false, 0));
    let mut found = Found { layout: layout.clone(), self_row: None, bg: [[CELL; 5]; 2], at: None, ui: s, guns: Vec::new() };
    // One grab of the game's whole UI area: the table is found wherever it is.
    let (x0, y0) = (scr.bx.max(0.0), scr.by.max(0.0));
    let Some(px) = capture::grab(x0 as i32, y0 as i32, scr.bw as i32, scr.bh as i32) else { return found };
    let img = scan::Img { px: &px, w: scr.bw as usize, h: scr.bh as usize };
    // Best: your row's yellow outline (size and place, no guessing).
    // Fallback: the row colours near where the table usually is.
    let f = scan::find_by_outline(&img).or_else(|| {
        let pitch = sb.pitch * s;
        scan::find(&img, scan::Guess { left: est_ally.x - x0, right: est_ally.x + est_ally.w - x0, pitch })
    });
    let Some(f) = f else {
        return found;
    };
    // Back to layout units, relative to where the anchors put things.
    let l = &mut found.layout.scoreboard;
    let unit = |px: f32| px / s;
    let p = unit(f.pitch);
    l.pitch = p;
    l.ally_row.rect.x += unit(f.left + x0 - est_ally.x);
    l.enemy_row.rect.x = l.ally_row.rect.x;
    l.ally_row.rect.y += unit(f.ally_top + y0 - est_ally.y);
    l.enemy_row.rect.y += unit(f.enemy_top + y0 - est_enemy.y);
    l.ally_row.rect.w = scan::TABLE_W * p;
    l.enemy_row.rect.w = scan::TABLE_W * p;
    l.ally_row.rect.h = p * 0.97;
    l.enemy_row.rect.h = p * 0.97;
    // Portrait and name cells in row units (measured: 30/34 and 72/34).
    l.icon = Rect { x: 0.06 * p, y: 0.03 * p, w: 0.9 * p, h: 0.9 * p };
    l.name = Rect { x: 2.05 * p, y: 0.06 * p, w: 5.0 * p, h: 0.85 * p };
    found.self_row = f.self_row;
    // What we draw follows the table's real size, not the window's.
    found.ui = f.pitch / 34.8;
    for t in 0..2 {
        for k in 0..5 {
            let (r, g, b) = f.bg[t][k];
            found.bg[t][k] = rgb_u32([r, g, b]);
        }
    }
    found.at = Some([f.left + x0, f.ally_top + y0, f.enemy_top + y0, f.pitch]);
    let (ox, oy) = ((x0 as i32) as f32, (y0 as i32) as f32);
    found.guns = read_guns(&img, &f, content);
    for g in &mut found.guns {
        g.at = Rect { x: g.at.x + ox, y: g.at.y + oy, ..g.at };
        g.cell = Rect { x: g.cell.x + ox, y: g.cell.y + oy, ..g.cell };
    }
    found
}

/// Reads the gun in each row's loadout cell by its silhouette (white, or
/// yellow on your row) and keeps the Vandals and Phantoms. For each, also
/// makes the pixels that paint the silhouette out: every silhouette pixel
/// (and a 2 px rim for its soft edge) takes the row colour found just left
/// of it, so no box shows behind the skin. Positions are in capture pixels.
fn read_guns(img: &scan::Img, f: &scan::Found, content: &Content) -> Vec<Gun> {
    let p = f.pitch;
    let (cw, ch) = ((3.4 * p) as usize, (p - 6.0 * p / 34.0) as usize);
    let mut out = Vec::new();
    for (ally, top) in [(true, f.ally_top), (false, f.enemy_top)] {
        for row in 0..5 {
            let (x0, y0) = ((f.left + 14.2 * p) as usize, (top + p * row as f32 + 3.0 * p / 34.0) as usize);
            if cw < 8 || ch < 4 || x0 + cw > img.w || y0 + ch > img.h {
                continue;
            }
            let px = |x: usize, y: usize| {
                let i = ((y0 + y) * img.w + x0 + x) * 4;
                [img.px[i], img.px[i + 1], img.px[i + 2]]
            };
            let lit: Vec<bool> = (0..cw * ch)
                .map(|k| {
                    let [b, g, r] = px(k % cw, k / cw).map(|v| v as i32);
                    (r > 200 && g > 200 && b > 200) || (r > 170 && g > 170 && b < r.min(g) - 40)
                })
                .collect();
            let on = |x: usize, y: usize| lit[y * cw + x];
            let Some(mask) = midscope_core::content::shape_mask(cw, ch, on) else { continue };
            let Some((gun, score)) = midscope_core::content::which_gun(content, &mask) else { continue };
            if score < 0.7 || !matches!(gun, "Vandal" | "Phantom") {
                continue;
            }
            let (mut bx0, mut by0, mut bx1, mut by1) = (cw, ch, 0, 0);
            for y in 0..ch {
                for x in 0..cw {
                    if on(x, y) {
                        bx0 = bx0.min(x);
                        by0 = by0.min(y);
                        bx1 = bx1.max(x);
                        by1 = by1.max(y);
                    }
                }
            }
            // The cover: silhouette pixels grown by 2, in the colour to their left.
            let near = |x: usize, y: usize| {
                (y.saturating_sub(2)..=(y + 2).min(ch - 1)).any(|yy| (x.saturating_sub(2)..=(x + 2).min(cw - 1)).any(|xx| on(xx, yy)))
            };
            let mut bgra = vec![0u8; cw * ch * 4];
            for y in 0..ch {
                let mut last = px(0, y);
                for x in 0..cw {
                    if near(x, y) {
                        bgra[(y * cw + x) * 4..][..4].copy_from_slice(&[last[0], last[1], last[2], 255]);
                    } else {
                        last = px(x, y);
                    }
                }
            }
            out.push(Gun {
                ally,
                row,
                name: gun.to_owned(),
                at: Rect { x: (x0 + bx0) as f32, y: (y0 + by0) as f32, w: (bx1 - bx0 + 1) as f32, h: (by1 - by0 + 1) as f32 },
                cell: Rect { x: x0 as f32, y: y0 as f32, w: cw as f32, h: ch as f32 },
                cover: std::sync::Arc::new(paint::Patch { w: cw, h: ch, bgra }),
            });
        }
    }
    out
}

/// A deathmatch scoreboard found on screen: one list of rows.
pub struct FoundDm {
    pub rows: Vec<Rect>,
    pub self_row: Option<usize>,
    pub at: [f32; 3],
}

impl FoundDm {
    pub fn settled_with(&self, o: &FoundDm) -> bool {
        self.rows.len() == o.rows.len() && self.at.iter().zip(o.at).all(|(a, b)| (a - b).abs() <= 1.5)
    }
}

/// Finds the deathmatch scoreboard (n rows, red, yours teal).
pub fn locate_deathmatch(layout: &Layout, scr: Screen, n: usize) -> Option<FoundDm> {
    let sb = &layout.scoreboard;
    let s = scr.scale;
    let est = scr.map(sb.row(true, 0));
    let pitch = sb.pitch * s;
    let (x0, y0) = ((est.x - 4.0 * pitch).max(0.0), (est.y - 9.0 * pitch).max(0.0));
    let (w, h) = (est.w + 8.0 * pitch, (n as f32 + 14.0) * pitch);
    let px = capture::grab(x0 as i32, y0 as i32, w as i32, h as i32)?;
    let img = scan::Img { px: &px, w: w as usize, h: h as usize };
    let f = scan::find_list(&img, scan::Guess { left: est.x - x0, right: est.x + est.w - x0, pitch }, n)?;
    let (left, top) = (f.left + x0, f.top + y0);
    let width = scan::TABLE_W * f.pitch;
    let rows = (0..n).map(|k| Rect { x: left, y: top + f.pitch * k as f32, w: width, h: f.pitch * 0.97 }).collect();
    Some(FoundDm { rows, self_row: f.self_row, at: [left, top, f.pitch] })
}

/// The portrait cell of a deathmatch row.
pub fn dm_portrait(r: Rect) -> Rect {
    let p = r.h / 0.97;
    Rect { x: r.x + 0.06 * p, y: r.y + 0.03 * p, w: 0.9 * p, h: 0.9 * p }
}

/// Deathmatch Tab list: rank and tags in the name gap, skins on the right,
/// your row cyan and everyone else red.
pub fn scoreboard_dm(snap: &Snapshot, seen: &Seen, cfg: &Config, found: &FoundDm, scr: Screen, out: &mut Vec<Prim>) {
    let s = scr.scale;
    for (i, row) in found.rows.iter().enumerate() {
        let Some((p_, _)) = player(snap, seen, true, i) else { continue };
        let p = row.h / 0.97;
        let ic = p * 0.86;
        if let Some(r) = p_.rank.as_ref().filter(|r| r.tier >= 3) {
            out.push(paint::Rank(r.tier, Rect { x: row.x + 1.06 * p, y: row.y + (row.h - ic) / 2.0, w: ic, h: ic }));
        }
        tags(p_, Rect { x: row.x + 5.6 * p, y: row.y, w: 3.0 * p, h: row.h }, s * 0.85, out);
        if cfg.show_skins {
            let cell = Rect { x: row.x + row.w, w: 200.0 * s, ..*row };
            let mine = found.self_row == Some(i);
            out.push(Prim::Fill(cell, if mine { ALLY_TINT } else { ENEMY_TINT }, if mine { 0.52 } else { 0.50 }));
            let half = (cell.w - 10.0 * s) / 2.0;
            let (v, vc) = short_skin(p_, "Vandal");
            let (ph, pc) = short_skin(p_, "Phantom");
            out.push(text(v, Font::Small, Rect { x: cell.x + 10.0 * s, w: half, ..cell }, vc));
            out.push(text(ph, Font::Small, Rect { x: cell.x + 10.0 * s + half, w: half, ..cell }, pc));
        }
    }
    if cfg.show_skins {
        if let Some(first) = found.rows.first() {
            let head = Rect { x: first.x + first.w, y: first.y - 26.0 * s, w: 200.0 * s, h: 24.0 * s };
            out.push(Prim::Fill(head, INK, 0.75));
            let half = (head.w - 10.0 * s) / 2.0;
            out.push(text("vandal", Font::Label, Rect { x: head.x + 10.0 * s, w: 60.0 * s, ..head }, MUTED));
            out.push(text("phantom", Font::Label, Rect { x: head.x + 10.0 * s + half, w: 60.0 * s, ..head }, MUTED));
        }
    }
}

/// Deathmatch VS screen: six teal cards a side, everyone for themselves.
pub fn loading_visible_dm(layout: &Layout, scr: Screen) -> bool {
    let l = &layout.loading;
    let teal = |left: bool, i: usize| {
        let r = scr.map(l.dm_card(left, i));
        pixel(r.x + 2.0, r.y + r.h * 0.5).is_some_and(|[r, g, b]| g as i32 > r as i32 + 60 && b as i32 > r as i32 + 40)
    };
    (0..6).flat_map(|i| [teal(true, i), teal(false, i)]).filter(|t| *t).count() >= 9
}

/// Slot k of the deathmatch VS screen: left column 0..6, right column 6..12.
pub fn dm_slot(layout: &Layout, k: usize) -> midscope_core::layout::Anchor {
    layout.loading.dm_card(k < 6, k % 6)
}

pub fn loading_dm(snap: &Snapshot, seen: &Seen, cfg: &Config, layout: &Layout, scr: Screen, out: &mut Vec<Prim>) {
    let l = &layout.loading;
    let s = scr.lscale;
    for k in 0..12 {
        let Some((p, _)) = player(snap, seen, true, k) else { continue };
        let (left, i) = (k < 6, k % 6);
        let chip = scr.map(l.dm_chip(left, i));
        out.push(Prim::Fill(chip, INK, 0.86));
        let edge_x = if left { chip.x } else { chip.x + chip.w - 3.0 * s };
        out.push(Prim::Fill(Rect { x: edge_x, w: 3.0 * s, ..chip }, TEAL, 1.0));
        let pad = 12.0 * s;
        let (name, col, tier) = rank_name(p);
        let icon = Rect { x: chip.x + pad, y: chip.y + 8.0 * s, w: 52.0 * s, h: 52.0 * s };
        if tier > 0 {
            out.push(paint::Rank(tier, icon));
        }
        let tx = icon.x + icon.w + 8.0 * s;
        let tw = chip.x + chip.w - tx - 6.0 * s;
        out.push(text(name, Font::Heading, Rect { x: tx, y: chip.y + 10.0 * s, w: tw, h: 24.0 * s }, col));
        if let Some(r) = p.rank.as_ref().filter(|r| r.tier >= 3) {
            out.push(text(format!("{} rr", r.rr), Font::Small, Rect { x: tx, y: chip.y + 34.0 * s, w: tw, h: 20.0 * s }, MUTED));
        }
        tags(p, Rect { x: chip.x + pad, y: chip.y + 68.0 * s, w: chip.w, h: 20.0 * s }, s, out);
        if cfg.show_skins {
            let (v, vc) = short_skin(p, "Vandal");
            out.push(text(format!("vandal  {v}"), Font::Small, Rect { x: chip.x + pad, y: chip.y + chip.h - 24.0 * s, w: chip.w - pad, h: 20.0 * s }, vc));
        }
    }
}

/// The VS loading screen is up when the enemy cards' red edge strips AND the
/// ally cards' teal (yours yellow) edge strips are all where they should be.
/// Checking both sides at once is what stops orange walls or red skies in the
/// round from passing for the VS screen.
pub fn loading_visible(layout: &Layout, scr: Screen) -> bool {
    let l = &layout.loading;
    // VALORANT's enemy red is a pinkish red (about 255,70,85): low green, some blue.
    let red = |[r, g, b]: [u8; 3]| r > 190 && g < 110 && b > 50 && b < 140 && (b as i32) > g as i32 - 25;
    let teal = |[r, g, b]: [u8; 3]| g as i32 > r as i32 + 60 && b as i32 > r as i32 + 40;
    // One tall grab per side covers all five strips.
    let column = |rects: Vec<Rect>| {
        let x = rects.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let w = rects.iter().map(|r| r.x + r.w.max(3.0)).fold(f32::MIN, f32::max) - x;
        let (y0, y1) = (rects[0].y, rects[rects.len() - 1].y + rects[rects.len() - 1].h);
        capture::Frame::grab(x as i32, y0 as i32, w.ceil() as i32 + 1, (y1 - y0) as i32 + 2).map(|f| (f, rects))
    };
    let count = |side: Option<(capture::Frame, Vec<Rect>)>, want: &dyn Fn([u8; 3]) -> bool| -> usize {
        let Some((f, rects)) = side else { return 0 };
        rects
            .iter()
            .filter(|r| {
                let y = r.y + r.h / 2.0;
                let n = r.w.max(3.0) as usize;
                (0..n).filter(|k| f.rgb(r.x + *k as f32, y).is_some_and(want)).count() * 2 >= n
            })
            .count()
    };
    let enemy = count(column((0..5).map(|i| scr.map(l.card(false, i).inner(l.enemy_strip))).collect()), &red);
    if enemy < 4 {
        return false;
    }
    let ally_strip = Rect { x: 1.0, y: 30.0, w: 4.0, h: 4.0 };
    let ally = count(column((0..5).map(|i| scr.map(l.card(true, i).inner(ally_strip))).collect()), &|c| teal(c) || is_yellow(c));
    ally >= 4
}

// ───────────────────────────── pieces ─────────────────────────────

fn short_skin<'a>(p: &'a Player, gun: &str) -> (&'a str, u32) {
    let skin = p.skin_for(gun).map(|k| k.skin.as_str()).unwrap_or("");
    let s = skin.strip_suffix(gun).map(str::trim_end).unwrap_or(skin);
    if s.is_empty() || s == "Default" { ("default", MUTED) } else { (s, TEXT) }
}

fn party_label(size: u8) -> &'static str {
    match size {
        2 => "duo",
        3 => "trio",
        4 => "4-stack",
        _ => "5-stack",
    }
}

fn party_color(party: u8) -> u32 {
    PARTY_COLORS[(party as usize).saturating_sub(1) % PARTY_COLORS.len()]
}

fn tag_color(t: &str) -> u32 {
    match t {
        "SMURF" => DANGER,
        "BOOSTED" => LAMP,
        _ => MUTED,
    }
}

/// Little tinted pills: smurf / boosted / tilted.
fn tags(p: &Player, r: Rect, s: f32, out: &mut Vec<Prim>) {
    let mut x = r.x;
    for t in &p.tags {
        let col = tag_color(t);
        let w = (t.len() as f32 * 7.0 + 16.0) * s;
        let rr = Rect { x, y: r.y + (r.h - 17.0 * s) / 2.0, w, h: 17.0 * s };
        out.push(Prim::Round(rr, rr.h / 2.0, col, 0.18));
        out.push(text(t.to_lowercase(), Font::Label, rr, col));
        x += w + 4.0 * s;
    }
}

fn rank_name(p: &Player) -> (String, u32, u32) {
    match &p.rank {
        Some(r) if r.tier >= 3 => (tier_name(r.tier).to_string(), tier_color(r.tier), r.tier),
        _ => ("UNRANKED".into(), MUTED, 0),
    }
}

/// A label sitting on a party line, on an ink backing so the line breaks around it.
fn line_label(label: &str, cx: f32, cy: f32, col: u32, s: f32, out: &mut Vec<Prim>) {
    let w = (label.len() as f32 * 7.4 + 20.0) * s;
    let r = Rect { x: cx - w / 2.0, y: cy - 9.0 * s, w, h: 18.0 * s };
    out.push(Prim::Glow(cx, cy, w * 0.8, col, 0.25));
    out.push(Prim::Round(r, r.h / 2.0, ROOM0, 1.0));
    out.push(Prim::RoundLine(r, r.h / 2.0, col, 1.5 * s));
    out.push(text(label, Font::Label, r, col));
}

/// Party line under a row of cards: a tick down from each member, joined
/// along the bottom, with DUO / TRIO in the middle.
fn bracket_under(members: &[Rect], y: f32, label: &str, col: u32, s: f32, out: &mut Vec<Prim>) {
    let w = 2.0 * s;
    let xs: Vec<f32> = members.iter().map(|r| r.x + r.w / 2.0).collect();
    for (r, x) in members.iter().zip(&xs) {
        out.push(Prim::Line(*x, r.y + r.h, *x, y, col, w));
    }
    let (lo, hi) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
    out.push(Prim::Line(lo - w / 2.0, y, hi + w / 2.0, y, col, w));
    line_label(label, (lo + hi) / 2.0, y, col, s, out);
}

/// Party line beside a column of rows: a tick out from each member's edge
/// at `edge_x` to a vertical lane at `lane_x`, label in the middle.
fn bracket_side(members: &[Rect], edge_x: f32, lane_x: f32, label: &str, col: u32, s: f32, out: &mut Vec<Prim>) {
    let w = 2.0 * s;
    let ys: Vec<f32> = members.iter().map(|r| r.y + r.h / 2.0).collect();
    for y in &ys {
        out.push(Prim::Line(edge_x, *y, lane_x, *y, col, w));
    }
    let (lo, hi) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
    out.push(Prim::Line(lane_x, lo - w / 2.0, lane_x, hi + w / 2.0, col, w));
    line_label(label, lane_x, (lo + hi) / 2.0, col, s, out);
}

/// Groups visible slots by party: (party, size, slot rects).
fn party_groups(snap: &Snapshot, seen: &Seen, ally: bool, rect: impl Fn(usize) -> Rect) -> Vec<(u8, u8, Vec<Rect>)> {
    let mut groups: Vec<(u8, u8, Vec<Rect>)> = Vec::new();
    for i in 0..5 {
        let Some((puuid, _)) = seen.who(ally, i) else { continue };
        let Some(p) = snap.players.iter().find(|p| p.puuid == puuid) else { continue };
        if p.party == 0 {
            continue;
        }
        match groups.iter_mut().find(|g| g.0 == p.party) {
            Some(g) => g.2.push(rect(i)),
            None => groups.push((p.party, p.party_size, vec![rect(i)])),
        }
    }
    groups.retain(|g| g.2.len() >= 2);
    groups
}

fn player<'a>(snap: &'a Snapshot, seen: &Seen, ally: bool, i: usize) -> Option<(&'a Player, u32)> {
    let (puuid, bg) = seen.who(ally, i)?;
    snap.players.iter().find(|p| p.puuid == puuid).map(|p| (p, bg))
}

// ───────────────────────────── screens ─────────────────────────────

/// Agent select: rank icon, rank and level above each card; party lines
/// joining duo/trio cards underneath.
pub fn agent_select(snap: &Snapshot, seen: &Seen, cfg: &Config, layout: &Layout, scr: Screen, out: &mut Vec<Prim>) {
    let a = &layout.agent_select;
    let s = scr.scale;
    for i in 0..5 {
        let Some((p, _)) = player(snap, seen, true, i) else { continue };
        let card = scr.map(a.card_at(i));
        let block = Rect { x: card.x, y: card.y - 50.0 * s, w: card.w, h: 46.0 * s };
        out.push(Prim::Fill(block, INK, 0.82));
        out.push(Prim::Fill(Rect { y: block.y + block.h - 2.0 * s, h: 2.0 * s, ..block }, TEAL, 0.9));
        let (name, col, tier) = rank_name(p);
        let icon = Rect { x: block.x + 4.0 * s, y: block.y + 3.0 * s, w: 40.0 * s, h: 40.0 * s };
        if tier > 0 {
            out.push(paint::Rank(tier, icon));
        }
        let tx = icon.x + icon.w + 4.0 * s;
        let tw = block.x + block.w - tx - 2.0 * s;
        out.push(text(name, Font::SmallBold, Rect { x: tx, y: block.y + 3.0 * s, w: tw, h: 20.0 * s }, col));
        let rr = p.rank.as_ref().filter(|r| r.tier >= 3).map(|r| format!("{} rr  ", r.rr)).unwrap_or_default();
        let lvl = if p.level > 0 { format!("lvl {}", p.level) } else { String::new() };
        out.push(text(format!("{rr}{lvl}"), Font::Small, Rect { x: tx, y: block.y + 22.0 * s, w: tw, h: 20.0 * s }, MUTED));
        if !p.tags.is_empty() {
            tags(p, Rect { x: card.x, y: block.y - 20.0 * s, w: card.w, h: 18.0 * s }, s, out);
        }
        if p.revealed {
            let nm = scr.map(a.card_at(i).inner(a.name));
            out.push(Prim::Fill(nm, CELL, 0.96));
            out.push(text(p.name.clone(), Font::Name, Rect { x: nm.x + 6.0 * s, ..nm }, YELLOW));
        }
    }
    if cfg.show_party_lines {
        let groups = party_groups(snap, seen, true, |i| scr.map(a.card_at(i)));
        for (n, (party, size, cards)) in groups.iter().enumerate() {
            let y = cards[0].y + cards[0].h + (9.0 + 14.0 * n as f32) * s;
            bracket_under(cards, y, party_label(*size), party_color(*party), s, out);
        }
    }
}

/// VS loading screen: a rank chip in the gap beside each card, and party
/// lines in the lane between the cards and the chips.
pub fn loading(snap: &Snapshot, seen: &Seen, cfg: &Config, layout: &Layout, scr: Screen, out: &mut Vec<Prim>) {
    let l = &layout.loading;
    let s = scr.lscale;
    for ally in [true, false] {
        for i in 0..5 {
            let Some((p, bg)) = player(snap, seen, ally, i) else { continue };
            let chip = scr.map(l.chip(ally, i));
            out.push(Prim::Fill(chip, INK, 0.86));
            let edge_x = if ally { chip.x } else { chip.x + chip.w - 3.0 * s };
            out.push(Prim::Fill(Rect { x: edge_x, w: 3.0 * s, ..chip }, if ally { TEAL } else { RED }, 1.0));
            let pad = 12.0 * s;
            let (name, col, tier) = rank_name(p);
            let icon = Rect { x: chip.x + pad, y: chip.y + 10.0 * s, w: 58.0 * s, h: 58.0 * s };
            if tier > 0 {
                out.push(paint::Rank(tier, icon));
            }
            let tx = icon.x + icon.w + 8.0 * s;
            let tw = chip.x + chip.w - tx - 6.0 * s;
            out.push(text(name, Font::Heading, Rect { x: tx, y: chip.y + 14.0 * s, w: tw, h: 24.0 * s }, col));
            if let Some(r) = p.rank.as_ref().filter(|r| r.tier >= 3) {
                out.push(text(format!("{} rr", r.rr), Font::Small, Rect { x: tx, y: chip.y + 40.0 * s, w: tw, h: 20.0 * s }, MUTED));
            }
            let mut y = chip.y + 76.0 * s;
            if let Some(r) = p.rank.as_ref().filter(|r| r.peak > r.tier && r.peak >= 3) {
                out.push(paint::Rank(r.peak, Rect { x: chip.x + pad, y, w: 20.0 * s, h: 20.0 * s }));
                out.push(text(format!("peak {}", tier_name(r.peak)), Font::Small, Rect { x: chip.x + pad + 26.0 * s, y, w: chip.w, h: 20.0 * s }, MUTED));
            }
            y += 24.0 * s;
            tags(p, Rect { x: chip.x + pad, y, w: chip.w, h: 20.0 * s }, s, out);
            if cfg.show_skins {
                let (v, vc) = short_skin(p, "Vandal");
                out.push(text(format!("vandal  {v}"), Font::Small, Rect { x: chip.x + pad, y: chip.y + chip.h - 24.0 * s, w: chip.w - pad, h: 20.0 * s }, vc));
            }
            if p.revealed {
                let nm = scr.map(l.card(ally, i).inner(l.name));
                out.push(Prim::Fill(nm, bg, 1.0));
                out.push(text(p.name.clone(), Font::Name, Rect { x: nm.x + 6.0 * s, ..nm }, YELLOW));
            }
        }
        if cfg.show_party_lines {
            let groups = party_groups(snap, seen, ally, |i| scr.map(l.card(ally, i)));
            for (n, (party, size, cards)) in groups.iter().enumerate() {
                let c = cards[0];
                // Lanes a label's width apart so two parties' labels never overlap.
                let (edge, lane) = if ally {
                    (c.x + c.w, c.x + c.w + (11.0 + 13.0 * n as f32) * s)
                } else {
                    (c.x, c.x - (11.0 + 13.0 * n as f32) * s)
                };
                bracket_side(cards, edge, lane, party_label(*size), party_color(*party), s, out);
            }
        }
    }
}

/// Translucent cyan / red for the columns we add to the scoreboard.
const ALLY_TINT: u32 = 0x37B4A5;
const ENEMY_TINT: u32 = 0xC8475A;

/// Tab scoreboard, worked into the game's table:
/// - a rank icon and tags in the empty gap between the name and ultimate columns;
/// - each player's Vandal / Phantom skin painted on the gun in the loadout column;
/// - party lines down the left side, outside the speaker icons.
pub fn scoreboard(snap: &Snapshot, seen: &Seen, cfg: &Config, found: &Found, scr: Screen, out: &mut Vec<Prim>) {
    let layout = &found.layout;
    let sb = &layout.scoreboard;
    let s = found.ui;
    let p = sb.pitch * scr.scale; // row height in pixels
    // Skins go on the guns themselves: the player's own Vandal / Phantom
    // picture laid over the white silhouette in the loadout column.
    if cfg.show_skins {
        for g in &found.guns {
            let Some((p_, _)) = player(snap, seen, g.ally, g.row) else { continue };
            let Some(url) = p_.skin_for(&g.name).and_then(|k| k.art.clone()) else { continue };
            let Some((iw, ih)) = midscope_core::content::skin_art_size(&url) else { continue };
            // Paint the silhouette out (only its own pixels), then the skin over
            // where it was, as wide as the silhouette and centred on it.
            out.push(Prim::Pixels(g.cover.clone(), g.cell));
            let at = g.at;
            let w = at.w * 1.04;
            let h = w * ih as f32 / iw.max(1) as f32;
            out.push(Prim::Image(paint::Img::Skin(url), Rect { x: at.x + at.w / 2.0 - w / 2.0, y: at.y + at.h / 2.0 - h / 2.0, w, h }));
        }
    }
    for ally in [true, false] {
        for i in 0..5 {
            let Some((p_, _)) = player(snap, seen, ally, i) else { continue };
            let row = scr.map(sb.row(ally, i));
            // Rank icon in the gap between the agent portrait and the name;
            // tags in the empty space after the name.
            let ic = p * 0.86;
            if let Some(r) = p_.rank.as_ref().filter(|r| r.tier >= 3) {
                out.push(paint::Rank(r.tier, Rect { x: row.x + 1.06 * p, y: row.y + (row.h - ic) / 2.0, w: ic, h: ic }));
            }
            tags(p_, Rect { x: row.x + 5.6 * p, y: row.y, w: 3.0 * p, h: row.h }, s * 0.85, out);
            let bg = found.bg[usize::from(!ally)][i];
            if p_.revealed {
                let nm = scr.map(sb.row(ally, i).inner(sb.name));
                out.push(Prim::Fill(nm, bg, 1.0));
                out.push(text(p_.name.clone(), Font::Name, nm, YELLOW));
            }
        }
        if cfg.show_party_lines {
            // Down the left side: ticks stop short of the speaker icons, the
            // lane runs outside them, one lane per party a label's width apart.
            let groups = party_groups(snap, seen, ally, |i| scr.map(sb.row(ally, i)));
            for (n, (party, size, rows)) in groups.iter().enumerate() {
                // Allies have speaker icons left of the table; enemies don't.
                let edge = if ally { rows[0].x - 1.05 * p } else { rows[0].x - 4.0 * s };
                let lane = edge - (26.0 + 46.0 * n as f32) * s;
                bracket_side(rows, edge, lane, party_label(*size), party_color(*party), s, out);
            }
        }
    }
}
