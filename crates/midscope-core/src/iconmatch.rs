//! The scoreboard sorts rows by score, which no API exposes. To know which
//! row is which player we look at the agent portrait in each row (a tiny
//! screen capture, the same thing OBS does) and match it to the agent icons.
//! Each team has unique agents, so the agent identifies the player.

/// Thumbnail side. 8x8 RGB is plenty to tell 25+ agent portraits apart.
pub const SIDE: usize = 8;

/// A normalised colour thumbnail (zero mean, unit length) for correlation.
pub type Thumb = Vec<f32>;

/// Builds a thumbnail from 4-byte pixels. `bgr` selects BGRA vs RGBA order.
/// Transparent pixels are composited over `bg` (RGB).
pub fn thumb(px: &[u8], width: usize, height: usize, stride: usize, bgr: bool, bg: [f32; 3]) -> Thumb {
    let mut out = vec![0f32; SIDE * SIDE * 3];
    let mut n = vec![0f32; SIDE * SIDE];
    for y in 0..height {
        let ty = y * SIDE / height.max(1);
        for x in 0..width {
            let tx = x * SIDE / width.max(1);
            let i = y * stride + x * 4;
            let Some(p) = px.get(i..i + 4) else { continue };
            let (r, g, b) = if bgr { (p[2], p[1], p[0]) } else { (p[0], p[1], p[2]) };
            let a = p[3] as f32 / 255.0;
            let cell = ty * SIDE + tx;
            for (c, v) in [r, g, b].into_iter().enumerate() {
                out[cell * 3 + c] += v as f32 * a + bg[c] * (1.0 - a);
            }
            n[cell] += 1.0;
        }
    }
    for (cell, count) in n.iter().enumerate() {
        for c in 0..3 {
            out[cell * 3 + c] /= count.max(1.0);
        }
    }
    normalise(out)
}

fn normalise(mut v: Vec<f32>) -> Thumb {
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    v.iter_mut().for_each(|x| *x -= mean);
    let len = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= len);
    v
}

/// Correlation in -1..=1; higher is more alike.
pub fn similarity(a: &Thumb, b: &Thumb) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Assigns each captured row to a candidate (index into `candidates`),
/// best pairs first, each candidate used once. Rows that match nothing
/// well enough stay `None`, and the overlay leaves those rows alone.
pub fn assign(rows: &[Thumb], candidates: &[Option<&Thumb>], min: f32) -> Vec<Option<usize>> {
    let scores: Vec<Vec<Option<f32>>> =
        rows.iter().map(|row| candidates.iter().map(|c| c.map(|t| similarity(row, t))).collect()).collect();
    assign_scores(&scores, candidates.len(), min)
}

/// Best overall assignment: tries every pairing (a team is at most 5, so
/// 120 combinations) and keeps the one with the highest total score, so one
/// strong match can't push two others into each other's rows.
/// `None` in the matrix means that candidate can't be on that row.
pub fn assign_scores(scores: &[Vec<Option<f32>>], candidates: usize, min: f32) -> Vec<Option<usize>> {
    let rows = scores.len();
    if rows > 6 || candidates > 6 {
        return assign_greedy(scores, candidates, min);
    }
    let mut best: (f32, Vec<Option<usize>>) = (f32::MIN, vec![None; rows]);
    let mut cur = vec![None; rows];
    let mut used = vec![false; candidates];
    fn go(r: usize, total: f32, scores: &[Vec<Option<f32>>], min: f32, cur: &mut Vec<Option<usize>>, used: &mut Vec<bool>, best: &mut (f32, Vec<Option<usize>>)) {
        if r == scores.len() {
            if total > best.0 {
                *best = (total, cur.clone());
            }
            return;
        }
        // Leave this row empty (costs a little, so it's only chosen when nothing fits).
        cur[r] = None;
        go(r + 1, total - 0.5, scores, min, cur, used, best);
        for c in 0..used.len() {
            if let Some(sc) = scores[r].get(c).copied().flatten() {
                if !used[c] && sc >= min {
                    used[c] = true;
                    cur[r] = Some(c);
                    go(r + 1, total + sc, scores, min, cur, used, best);
                    used[c] = false;
                }
            }
        }
        cur[r] = None;
    }
    go(0, 0.0, scores, min, &mut cur, &mut used, &mut best);
    best.1
}

fn assign_greedy(scores: &[Vec<Option<f32>>], candidates: usize, min: f32) -> Vec<Option<usize>> {
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (r, row) in scores.iter().enumerate() {
        for (c, s) in row.iter().enumerate() {
            if let Some(s) = s {
                pairs.push((*s, r, c));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = vec![None; scores.len()];
    let mut used = vec![false; candidates];
    for (s, r, c) in pairs {
        if s >= min && out[r].is_none() && !used[c] {
            out[r] = Some(c);
            used[c] = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                v.extend_from_slice(&[r, g, b, 255]);
            }
        }
        v
    }

    #[test]
    fn matches_rows_to_icons_regardless_of_order() {
        let red_left = solid(32, 32, |x, _| if x < 16 { [220, 40, 40] } else { [20, 20, 20] });
        let blue_top = solid(32, 32, |_, y| if y < 16 { [40, 40, 220] } else { [20, 20, 20] });
        let icons = [thumb(&red_left, 32, 32, 128, false, [0.0; 3]), thumb(&blue_top, 32, 32, 128, false, [0.0; 3])];
        // Rows captured at a different size and in the other order, as BGRA.
        let to_bgra = |v: &[u8]| v.chunks(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect::<Vec<_>>();
        let a = solid(40, 40, |_, y| if y < 20 { [40, 40, 220] } else { [20, 20, 20] });
        let b = solid(40, 40, |x, _| if x < 20 { [220, 40, 40] } else { [20, 20, 20] });
        let rows = [thumb(&to_bgra(&a), 40, 40, 160, true, [0.0; 3]), thumb(&to_bgra(&b), 40, 40, 160, true, [0.0; 3])];
        let got = assign(&rows, &[Some(&icons[0]), Some(&icons[1])], 0.5);
        assert_eq!(got, vec![Some(1), Some(0)]);
    }

    /// Greedy would give row 0 to its single best (c0) and force row 1 onto
    /// a poor fit; the best total puts row 0 on c1 instead.
    #[test]
    fn best_total_beats_greedy() {
        let s = vec![vec![Some(0.90), Some(0.85)], vec![Some(0.88), Some(0.10)]];
        assert_eq!(assign_scores(&s, 2, 0.0), vec![Some(1), Some(0)]);
    }
}
