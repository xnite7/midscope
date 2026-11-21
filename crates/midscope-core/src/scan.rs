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

