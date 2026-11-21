//! Finds VALORANT's Tab scoreboard in a screen capture from its colours:
//! teal ally rows, red enemy rows (dead rows stay tinted), and the yellow
//! outline round the local player's row.
//!
//! The table's proportions are fixed (measured from five screenshots at
//! two sizes): the enemy block starts 6.77 rows below the ally block, the
//! table is 22.8 rows wide, and the name column starts 0.85 rows in.

/// Enemy block top minus ally block top, in rows.
pub const BLOCK_GAP: f32 = 6.77;
/// Table width, in rows.
pub const TABLE_W: f32 = 22.8;
/// Where the (always teal/red) name column starts, in rows from the left edge.
const NAME_COL: f32 = 0.85;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Band {
    Teal,
    Red,
    Yellow,
    None,
}

pub fn is_yellow(r: u8, g: u8, b: u8) -> bool {
    r > 180 && g > 150 && (b as i32) < (g as i32) - 50
}

fn band(r: u8, g: u8, b: u8) -> Band {
    let (ri, gi, bi) = (r as i32, g as i32, b as i32);
    if is_yellow(r, g, b) {
        Band::Yellow
    } else if gi > ri + 12 && bi > ri + 12 {
        Band::Teal
    } else if ri > gi + 8 && ri >= bi {
        Band::Red
    } else {
        Band::None
    }
}

/// A BGRA capture (stride = w * 4).
pub struct Img<'a> {
    pub px: &'a [u8],
    pub w: usize,
    pub h: usize,
}

impl Img<'_> {
    fn rgb(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * self.w + x) * 4;
        (self.px[i + 2], self.px[i + 1], self.px[i])
    }
    fn band(&self, x: usize, y: usize) -> Band {
        let (r, g, b) = self.rgb(x, y);
        band(r, g, b)
    }
}

/// Where we expect the table, in capture pixels.
#[derive(Clone, Copy, Debug)]
pub struct Guess {
    pub left: f32,
    pub right: f32,
    pub pitch: f32,
}

/// What we found, in capture pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub left: f32,
    pub ally_top: f32,
    pub enemy_top: f32,
    pub pitch: f32,
    pub self_row: Option<usize>,
    /// Each row's own colour (median along its centre line), allies then enemies.
    pub bg: [[(u8, u8, u8); 5]; 2],
}

impl Found {
    pub fn width(&self) -> f32 {
        TABLE_W * self.pitch
    }
}

/// Longest run of `want` lines (yellow counts as part of any block).
fn longest(lines: &[Band], want: Band) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut y = 0;
    while y < lines.len() {
        if lines[y] == want || lines[y] == Band::Yellow {
            let start = y;
            let mut has = false;
            while y < lines.len() && (lines[y] == want || lines[y] == Band::Yellow) {
                has |= lines[y] == want;
                y += 1;
            }
            if has && best.is_none_or(|(_, len)| y - start > len) {
                best = Some((start, y - start));
            }
        } else {
            y += 1;
        }
    }
    best
}

pub fn find(img: &Img, guess: Guess) -> Option<Found> {
    let (w, h) = (img.w, img.h);
    if w < 20 || h < 20 {
        return None;
    }
    // 24 columns across the expected table vote on each line; text and
    // icons get outvoted.
    let cols: Vec<usize> = (0..24)
        .map(|i| (guess.left + (guess.right - guess.left) * (0.05 + 0.9 * i as f32 / 23.0)) as usize)
        .filter(|&x| x < w)
        .collect();
    let lines: Vec<Band> = (0..h)
        .map(|y| {
            let v: Vec<Band> = cols.iter().map(|&x| img.band(x, y)).collect();
            let n = |b: Band| v.iter().filter(|x| **x == b).count();
            let (t, r) = (n(Band::Teal), n(Band::Red));
            if n(Band::Yellow) >= 6 {
                Band::Yellow
            } else if t >= 9 && t > r {
                Band::Teal
            } else if r >= 9 && r > t {
                Band::Red
            } else {
                Band::None
            }
        })
        .collect();

    let block = |len: usize, pitch: f32| (len as f32 - 5.0 * pitch).abs() <= 1.25 * pitch;
    let red = longest(&lines, Band::Red);
    let teal = longest(&lines, Band::Teal);
    // Row pitch from whichever block is the right size (red first: scenery is rarely red).
    let pitch = [red, teal]
        .into_iter()
        .flatten()
        .map(|(_, len)| len as f32 / 5.0)
        .find(|p| (p / guess.pitch - 1.0).abs() < 0.2)
        .unwrap_or(guess.pitch);
    let enemy = red.filter(|(_, len)| block(*len, pitch)).map(|(y, _)| y as f32);
    let ally = teal.filter(|(_, len)| block(*len, pitch)).map(|(y, _)| y as f32);
    // A block merged with scenery is placed from the other one.
    let (ally_top, enemy_top) = match (ally, enemy) {
        (Some(a), Some(e)) => (a, e),
        (Some(a), None) => (a, a + BLOCK_GAP * pitch),
        (None, Some(e)) => (e - BLOCK_GAP * pitch, e),
        (None, None) => return None,
    };

    // Left edge: the first spot where most ally rows are teal and most
    // enemy rows red at once (the name column), then back to the table edge.
    let centre = |top: f32, k: usize| ((top + pitch * (k as f32 + 0.5)) as usize).min(h - 1);
    let both = |x: usize| {
        let t = (0..5).filter(|&k| matches!(img.band(x, centre(ally_top, k)), Band::Teal | Band::Yellow)).count();
        let r = (0..5).filter(|&k| img.band(x, centre(enemy_top, k)) == Band::Red).count();
        t >= 3 && r >= 3
    };
    let span = (3.0 * pitch) as usize;
    let from = (guess.left as usize).saturating_sub(span);
    let to = ((guess.left as usize) + span).min(w.saturating_sub(12));
    let name_x = (from..to).find(|&x| (0..10).step_by(2).all(|d| both(x + d)));
    let left = match name_x {
        Some(x) => x as f32 - NAME_COL * pitch,
        // Fall back to the right edge.
        None => {
            let from = ((guess.right as usize) + span).min(w - 1);
            let to = (guess.right as usize).saturating_sub(span).max(10);
            let right = (to..=from).rev().find(|&x| (0..10).step_by(2).all(|d| both(x - d)))? as f32 + 1.0;
            right - TABLE_W * pitch
        }
    };

    // Your row: the yellow outline line inside the ally block.
    let self_row = lines
        .iter()
        .enumerate()
        .find(|(y, b)| **b == Band::Yellow && (*y as f32) >= ally_top - 2.0 && (*y as f32) < ally_top + 5.0 * pitch)
        .map(|(y, _)| ((y as f32 - ally_top) / pitch).round() as usize)
        .filter(|r| *r < 5);

    // Row colours: the median along each row's centre line.
    let mut bg = [[(0u8, 0u8, 0u8); 5]; 2];
    for (t, top) in [ally_top, enemy_top].into_iter().enumerate() {
        for (k, slot) in bg[t].iter_mut().enumerate() {
            let y = centre(top, k);
            let mut rs = Vec::new();
            let mut gs = Vec::new();
            let mut bs = Vec::new();
            for i in 0..40 {
                let x = (left + pitch * NAME_COL + (TABLE_W - NAME_COL) * pitch * i as f32 / 40.0) as usize;
                if x < w {
                    let (r, g, b) = img.rgb(x, y);
                    rs.push(r);
                    gs.push(g);
                    bs.push(b);
                }
            }
            let med = |v: &mut Vec<u8>| {
                v.sort_unstable();
                v.get(v.len() / 2).copied().unwrap_or(0)
            };
            *slot = (med(&mut rs), med(&mut gs), med(&mut bs));
        }
    }
    Some(Found { left, ally_top, enemy_top, pitch, self_row, bg })
}

/// Finds the scoreboard from the yellow outline round your own row: it spans
/// the table's full width (so its right end is the table's right edge) and
/// its two lines are one row apart (so it gives the row height at any
/// resolution). Rows above yours that are still teal put the table's top.
/// Needs no guess about where the table is or how big.
pub fn find_by_outline(img: &Img) -> Option<Found> {
    let (w, h) = (img.w, img.h);
    let yellow = |x: usize, y: usize| {
        let (r, g, b) = img.rgb(x, y);
        r > 170 && g > 150 && (b as i32) < g as i32 - 40
    };
    // Every line with a long yellow run: (y, run end).
    let mut lines: Vec<(usize, usize, usize)> = Vec::new();
    for y in 0..h {
        let (mut best, mut start, mut last, mut on) = ((0usize, 0usize), 0usize, 0usize, false);
        for x in 0..w {
            if yellow(x, y) {
                if !on || x - last > 3 {
                    start = x;
                }
                on = true;
                last = x;
                if last - start > best.1.saturating_sub(best.0) {
                    best = (start, last);
                }
            }
        }
        if best.1 - best.0 >= 250 {
            lines.push((y, best.0, best.1));
        }
    }
    // The outline: two such lines 20..80 px apart ending at the same x.
    let mut pick = None;
    'outer: for (i, a) in lines.iter().enumerate() {
        for b in &lines[i + 1..] {
            let d = b.0 - a.0;
            if (20..=80).contains(&d) && (a.2 as i32 - b.2 as i32).abs() <= 3 {
                pick = Some((a.0, b.0, a.2.max(b.2)));
                break 'outer;
            }
        }
    }
    let (y1, y2, right_px) = pick?;
    let start_px = lines.iter().filter(|l| l.0 == y1 || l.0 == y2).map(|l| l.1).min().unwrap_or(0);
    let right = right_px as f32 + 1.0;
    let mut pitch = (y2 - y1) as f32 + 1.0;
    // When the outline is unbroken its left end is the table's left edge and
    // gives the exact row height too; if something covers part of it, fall
    // back to the table's fixed width.
    let mut left = right - TABLE_W * pitch;
    if (start_px as f32 - left).abs() < 0.5 * pitch {
        left = start_px as f32;
        pitch = (right - left) / TABLE_W;
    }
    if left < -pitch {
        return None;
    }
    // Teal rows above yours (dead ones stay tinted) up to the header bar.
    // Sampled in the KDA..ping columns, clear of anything drawn on the table.
    let teal_row = |row_top: f32| {
        let y = (row_top + pitch * 0.5) as usize;
        if y >= h {
            return false;
        }
        // 24 points across the row; text and gun icons are white, so a quarter
        // teal (and more teal than red) is a row; the header bar has none.
        let xs: Vec<usize> = (0..24).map(|k| (left + pitch * (2.2 + 0.85 * k as f32)) as usize).filter(|&x| x < w).collect();
        let t = xs.iter().filter(|&&x| img.band(x, y) == Band::Teal).count();
        let r = xs.iter().filter(|&&x| img.band(x, y) == Band::Red).count();
        t * 4 >= xs.len() && t > r
    };
    let mut k = 0;
    while k < 4 && y1 as f32 - (k + 1) as f32 * pitch >= 0.0 && teal_row(y1 as f32 - (k + 1) as f32 * pitch) {
        k += 1;
    }
    let ally_top = y1 as f32 - k as f32 * pitch;
    let enemy_top = ally_top + BLOCK_GAP * pitch;

    let centre = |top: f32, r: usize| ((top + pitch * (r as f32 + 0.5)) as usize).min(h - 1);
    let mut bg = [[(0u8, 0u8, 0u8); 5]; 2];
    for (t, top) in [ally_top, enemy_top].into_iter().enumerate() {
        for (r, slot) in bg[t].iter_mut().enumerate() {
            let y = centre(top, r);
            let mut v: Vec<(u8, u8, u8)> = (0..12)
                .map(|q| ((left + pitch * (9.5 + q as f32)) as usize).min(w - 1))
                .map(|x| img.rgb(x, y))
                .collect();
            v.sort_by_key(|c| c.0 as u32 + c.1 as u32 + c.2 as u32);
            *slot = v[v.len() / 2];
        }
    }
    Some(Found { left, ally_top, enemy_top, pitch, self_row: Some(k), bg })
}

/// A free-for-all scoreboard (deathmatch): one list of `n` rows, red for
/// everyone, teal for you.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundList {
    pub left: f32,
    pub top: f32,
    pub pitch: f32,
    pub self_row: Option<usize>,
}

pub fn find_list(img: &Img, guess: Guess, n: usize) -> Option<FoundList> {
    let (w, h) = (img.w, img.h);
    if w < 20 || h < 20 || n == 0 {
        return None;
    }
    let cols: Vec<usize> = (0..24)
        .map(|i| (guess.left + (guess.right - guess.left) * (0.05 + 0.9 * i as f32 / 23.0)) as usize)
        .filter(|&x| x < w)
        .collect();
    // A line is "in the list" when most columns are red, teal or yellow.
    let lines: Vec<Option<Band>> = (0..h)
        .map(|y| {
            let v: Vec<Band> = cols.iter().map(|&x| img.band(x, y)).collect();
            let tinted = v.iter().filter(|b| **b != Band::None).count();
            (tinted >= 12).then(|| {
                let t = v.iter().filter(|b| **b == Band::Teal).count();
                let r = v.iter().filter(|b| **b == Band::Red).count();
                if t > r { Band::Teal } else { Band::Red }
            })
        })
        .collect();
    // Longest run of tinted lines; small gaps (row separators) allowed.
    let (mut best, mut y) = ((0usize, 0usize), 0usize);
    while y < h {
        if lines[y].is_some() {
            let start = y;
            let mut last = y;
            while y < h && (lines[y].is_some() || y - last <= 3) {
                if lines[y].is_some() {
                    last = y;
                }
                y += 1;
            }
            if last + 1 - start > best.1 {
                best = (start, last + 1 - start);
            }
        } else {
            y += 1;
        }
    }
    let (top, len) = (best.0 as f32, best.1 as f32);
    let pitch = len / n as f32;
    if (pitch / guess.pitch - 1.0).abs() > 0.25 {
        return None;
    }
    // Your row is the teal one.
    let self_row = (0..n).find(|&k| {
        let yc = (top + pitch * (k as f32 + 0.5)) as usize;
        lines.get(yc).copied().flatten() == Some(Band::Teal)
    });
    // Left edge: the name column, where every row (bar one) is tinted.
    // Scenery is often faintly reddish, so a looser rule latches onto it.
    let centre = |k: usize| ((top + pitch * (k as f32 + 0.5)) as usize).min(h - 1);
    let tinted = |x: usize| (0..n).filter(|&k| img.band(x, centre(k)) != Band::None).count() + 1 >= n;
    let span = (3.0 * pitch) as usize;
    let from = (guess.left as usize).saturating_sub(span);
    let to = ((guess.left as usize) + span).min(w.saturating_sub(12));
    let x = (from..to).find(|&x| (0..10).step_by(2).all(|d| tinted(x + d)))?;
    Some(FoundList { left: x as f32 - NAME_COL * pitch, top, pitch, self_row })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> Option<(Vec<u8>, usize, usize)> {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(path).ok()?;
        let (rgba, w, h) = crate::content::decode_png(&bytes)?;
        let bgra = rgba.chunks(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
        Some((bgra, w, h))
    }

    /// (file, guess left/right/pitch deliberately ~20 px off, expected left, ally top, enemy top, your row)
    const CASES: &[(&str, f32, f32, f32, f32, f32, f32, usize)] = &[
        ("sb_a.png", 87.0, 880.0, 35.0, 104.0, 84.0, 313.0, 4),
        ("sb_b.png", 92.0, 885.0, 35.0, 109.0, 73.0, 303.0, 4),
        ("sb_c.png", 87.0, 880.0, 35.0, 104.0, 71.0, 301.0, 4),
        ("sb_d.png", 21.0, 813.0, 33.0, 38.0, 49.0, 279.0, 2),
        ("sb_e.png", 374.0, 1138.0, 33.0, 391.0, 321.0, 543.0, 2),
        ("sb_f.png", 30.0, 810.0, 35.0, 44.0, 44.0, 275.0, 3),
    ];

    /// The outline method needs no guess at all: whole screenshots in, table out.
    /// (g and h have midscope's own drawing over the table, as captured then.)
    #[test]
    fn finds_scoreboard_from_your_outline() {
        let cases: &[(&str, f32, f32, f32, usize)] = &[
            ("sb_a.png", 104.0, 84.0, 313.0, 4),
            ("sb_b.png", 109.0, 73.0, 303.0, 4),
            ("sb_c.png", 104.0, 71.0, 301.0, 4),
            ("sb_d.png", 38.0, 49.0, 279.0, 2),
            ("sb_e.png", 391.0, 321.0, 543.0, 2),
            ("sb_f.png", 44.0, 44.0, 275.0, 3),
            ("sb_g.png", 169.0, 160.0, 390.0, 1),
        ];
        let mut ran = 0;
        for &(file, left, ally, enemy, me) in cases {
            let Some((px, w, h)) = load(file) else { continue };
            ran += 1;
            let f = find_by_outline(&Img { px: &px, w, h }).expect(file);
            assert!((f.left - left).abs() <= 4.0, "{file}: left {} want {left}", f.left);
            assert!((f.ally_top - ally).abs() <= 3.0, "{file}: ally {} want {ally}", f.ally_top);
            assert!((f.enemy_top - enemy).abs() <= 5.0, "{file}: enemy {} want {enemy}", f.enemy_top);
            assert_eq!(f.self_row, Some(me), "{file}: your row");
        }
        eprintln!("outline: checked {ran}");
    }

    #[test]
    fn finds_deathmatch_list() {
        let Some((px, w, h)) = load("dm_sb.png") else { return };
        let f = find_list(&Img { px: &px, w, h }, Guess { left: 10.0, right: 790.0, pitch: 35.0 }, 12).expect("dm");
        assert!((f.top - 42.0).abs() <= 3.0, "top {}", f.top);
        assert!((f.pitch - 33.5).abs() <= 1.0, "pitch {}", f.pitch);
        assert!((f.left - 26.0).abs() <= 4.0, "left {}", f.left);
        assert_eq!(f.self_row, Some(11));
    }

    #[test]
    fn finds_scoreboard_in_real_screenshots() {
        let mut ran = 0;
        for &(file, gl, gr, gp, left, ally, enemy, me) in CASES {
            let Some((px, w, h)) = load(file) else { continue };
            ran += 1;
            let f = find(&Img { px: &px, w, h }, Guess { left: gl, right: gr, pitch: gp }).expect(file);
            assert!((f.left - left).abs() <= 4.0, "{file}: left {} want {left}", f.left);
            assert!((f.ally_top - ally).abs() <= 3.0, "{file}: ally {} want {ally}", f.ally_top);
            assert!((f.enemy_top - enemy).abs() <= 3.0, "{file}: enemy {} want {enemy}", f.enemy_top);
            assert_eq!(f.self_row, Some(me), "{file}: your row");
        }
        eprintln!("checked {ran} screenshots");
    }
}

