//! Names for agent, weapon, skin and buddy IDs, from valorant-api.com,
//! cached on disk per game version so later starts need no download.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Default, Serialize, Deserialize)]
pub struct Content {
    pub version: String,
    /// lowercase uuid -> display name (agents, weapons, skins, buddy levels)
    pub names: HashMap<String, String>,
    /// skin uuid -> weapon uuid
    pub skin_weapon: HashMap<String, String>,
    /// weapon name -> silhouette mask (MASK_W x MASK_H, 0/1), to tell guns apart
    /// in the scoreboard's loadout column.
    #[serde(default)]
    pub gun_masks: HashMap<String, Vec<u8>>,
    /// skin or chroma uuid -> picture url (a chroma's render wins over its skin's).
    #[serde(default)]
    pub skin_art: HashMap<String, String>,
    /// agent name -> 32x32 RGBA portrait, for matching rows to players.
    /// Kept raw so it can be composited over each row's own background.
    #[serde(default)]
    pub icons: HashMap<String, Vec<u8>>,
}

/// Decodes a PNG into RGBA bytes plus its size.
pub fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, usize, usize)> {
    let mut dec = png::Decoder::new(bytes);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, w, h))
}

pub const ICON_SIDE: usize = 32;

/// Gun silhouettes are compared at this size (after trimming to their shape).
pub const MASK_W: usize = 48;
pub const MASK_H: usize = 14;

fn fetch_png(url: &str) -> Option<(Vec<u8>, usize, usize)> {
    let mut bytes = Vec::new();
    ureq::get(url).call().ok()?.into_reader().read_to_end(&mut bytes).ok()?;
    decode_png(&bytes)
}

/// Trims a shape to its bounding box and samples it to MASK_W x MASK_H.
pub fn shape_mask(w: usize, h: usize, on: impl Fn(usize, usize) -> bool) -> Option<Vec<u8>> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if on(x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX || x1 - x0 < 8 {
        return None;
    }
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut m = vec![0u8; MASK_W * MASK_H];
    for my in 0..MASK_H {
        for mx in 0..MASK_W {
            // Majority of the cell.
            let (ax, bx) = (x0 + mx * bw / MASK_W, x0 + ((mx + 1) * bw / MASK_W).max(mx * bw / MASK_W + 1));
            let (ay, by) = (y0 + my * bh / MASK_H, y0 + ((my + 1) * bh / MASK_H).max(my * bh / MASK_H + 1));
            let (mut n, mut hit) = (0, 0);
            for y in ay..by.min(y1 + 1) {
                for x in ax..bx.min(x1 + 1) {
                    n += 1;
                    hit += on(x, y) as usize;
                }
            }
            m[my * MASK_W + mx] = (hit * 2 >= n.max(1)) as u8;
        }
    }
    Some(m)
}

fn alpha_mask(px: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    shape_mask(w, h, |x, y| px[(y * w + x) * 4 + 3] > 128)
}

/// Overlap of two masks (intersection over union), 0..1.
pub fn mask_match(a: &[u8], b: &[u8]) -> f32 {
    let (mut i, mut u) = (0, 0);
    for (x, y) in a.iter().zip(b) {
        i += (*x & *y) as u32;
        u += (*x | *y) as u32;
    }
    i as f32 / u.max(1) as f32
}

/// Which gun a silhouette is, and how sure (IoU).
pub fn which_gun<'a>(c: &'a Content, mask: &[u8]) -> Option<(&'a str, f32)> {
    c.gun_masks.iter().map(|(n, m)| (n.as_str(), mask_match(m, mask))).max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Skin pictures are cached on disk as raw RGBA (trimmed to the gun).
pub fn skin_art_path(url: &str) -> PathBuf {
    let key: String = url.split('/').rev().nth(1).unwrap_or("skin").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    crate::config::data_dir().join("skins").join(format!("{key}.rgba"))
}

/// Downloads skin pictures that aren't cached yet (all at once).
pub fn ensure_skin_art(urls: &[String]) {
    let todo: Vec<&String> = urls.iter().filter(|u| !skin_art_path(u).exists()).collect();
    if todo.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(crate::config::data_dir().join("skins"));
    std::thread::scope(|s| {
        for url in todo {
            s.spawn(move || {
                let Some((px, w, h)) = fetch_png(url) else { return };
                // Trim the transparent margin so the picture lines up with the silhouette.
                let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
                for y in 0..h {
                    for x in 0..w {
                        if px[(y * w + x) * 4 + 3] > 24 {
                            x0 = x0.min(x);
                            y0 = y0.min(y);
                            x1 = x1.max(x);
                            y1 = y1.max(y);
                        }
                    }
                }
                if x0 >= x1 || y0 >= y1 {
                    return;
                }
                let (tw, th) = (x1 - x0 + 1, y1 - y0 + 1);
                let mut out = Vec::with_capacity(8 + tw * th * 4);
                out.extend_from_slice(&(tw as u32).to_le_bytes());
                out.extend_from_slice(&(th as u32).to_le_bytes());
                for y in y0..=y1 {
                    out.extend_from_slice(&px[(y * w + x0) * 4..(y * w + x1 + 1) * 4]);
                }
                // Written aside then renamed, so a half-written file is never read.
                let path = skin_art_path(url);
                let tmp = path.with_extension("part");
                if std::fs::write(&tmp, out).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            });
        }
    });
}

/// Reads a cached skin picture: (RGBA, width, height).
pub fn load_skin_art(url: &str) -> Option<(Vec<u8>, usize, usize)> {
    let b = std::fs::read(skin_art_path(url)).ok()?;
    let w = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    let h = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
    (b.len() == 8 + w * h * 4).then(|| (b[8..].to_vec(), w, h))
}

/// Width and height of a cached skin picture (header only, remembered).
pub fn skin_art_size(url: &str) -> Option<(usize, usize)> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static SIZES: Mutex<Option<HashMap<String, (usize, usize)>>> = Mutex::new(None);
    let mut g = SIZES.lock().ok()?;
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(s) = map.get(url) {
        return Some(*s);
    }
    use std::io::Read;
    let mut hdr = [0u8; 8];
    std::fs::File::open(skin_art_path(url)).ok()?.read_exact(&mut hdr).ok()?;
    let w = u32::from_le_bytes(hdr[0..4].try_into().ok()?) as usize;
    let h = u32::from_le_bytes(hdr[4..8].try_into().ok()?) as usize;
    map.insert(url.to_owned(), (w, h));
    Some((w, h))
}

/// Box-filters RGBA down to ICON_SIDE x ICON_SIDE.
fn shrink(px: &[u8], w: usize, h: usize) -> Vec<u8> {
    let n = ICON_SIDE;
    let mut out = vec![0u8; n * n * 4];
    for ty in 0..n {
        for tx in 0..n {
            let (x0, x1) = (tx * w / n, ((tx + 1) * w / n).max(tx * w / n + 1));
            let (y0, y1) = (ty * h / n, ((ty + 1) * h / n).max(ty * h / n + 1));
            let mut acc = [0u32; 4];
            let mut cnt = 0;
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let i = (y * w + x) * 4;
                    (0..4).for_each(|c| acc[c] += px[i + c] as u32);
                    cnt += 1;
                }
            }
            let o = (ty * n + tx) * 4;
            (0..4).for_each(|c| out[o + c] = (acc[c] / cnt.max(1)) as u8);
        }
    }
    out
}

/// Official rank icons, one PNG per tier, kept on disk.
pub fn rank_icon_path(tier: u32) -> PathBuf {
    crate::config::data_dir().join("ranks").join(format!("{tier}.png"))
}

/// Downloads the current act's rank icons if they aren't on disk yet.
pub fn ensure_rank_icons() {
    if rank_icon_path(27).exists() {
        return;
    }
    let Ok(resp) = ureq::get("https://valorant-api.com/v1/competitivetiers").call() else { return };
    let Ok(v) = resp.into_json::<Value>() else { return };
    let Some(latest) = v["data"].as_array().and_then(|a| a.last()) else { return };
    let _ = std::fs::create_dir_all(crate::config::data_dir().join("ranks"));
    for t in latest["tiers"].as_array().into_iter().flatten() {
        let (Some(tier), Some(url)) = (t["tier"].as_u64(), t["largeIcon"].as_str()) else { continue };
        let mut bytes = Vec::new();
        if let Ok(r) = ureq::get(url).call() {
            if r.into_reader().read_to_end(&mut bytes).is_ok() {
                let _ = std::fs::write(rank_icon_path(tier as u32), &bytes);
            }
        }
    }
}

impl Content {
    /// Thumbnail of an agent's portrait composited over `bg` (RGB).
    pub fn icon_thumb(&self, agent: &str, bg: [f32; 3]) -> Option<crate::iconmatch::Thumb> {
        let px = self.icons.get(agent)?;
        Some(crate::iconmatch::thumb(px, ICON_SIDE, ICON_SIDE, ICON_SIDE * 4, false, bg))
    }
}

fn cache_path() -> PathBuf {
    crate::config::data_dir().join("content.json")
}

impl Content {
    pub fn name(&self, id: &str) -> Option<&str> {
        self.names.get(&id.to_ascii_lowercase()).map(String::as_str)
    }

    /// Loads the cache, refreshing it when the game version changed.
    pub fn load(version: &str) -> Content {
        if let Some(c) = std::fs::read(cache_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Content>(&b).ok())
            .filter(|c| version.is_empty() || c.version == version)
            .filter(|c| !c.icons.is_empty() && !c.gun_masks.is_empty())
        {
            return c;
        }
        match Content::download(version) {
            Ok(c) => {
                let _ = std::fs::create_dir_all(crate::config::data_dir());
                let _ = std::fs::write(cache_path(), serde_json::to_vec(&c).unwrap_or_default());
                c
            }
            Err(_) => Content::default(),
        }
    }

    fn download(version: &str) -> Result<Content, ureq::Error> {
        let get = |p: &str| -> Result<Value, ureq::Error> {
            Ok(ureq::get(&format!("https://valorant-api.com/v1/{p}")).call()?.into_json()?)
        };
        let mut c = Content { version: version.to_owned(), ..Default::default() };
        let id = |v: &Value| v["uuid"].as_str().unwrap_or_default().to_ascii_lowercase();
        let nm = |v: &Value| v["displayName"].as_str().unwrap_or_default().to_owned();

        for a in get("agents?isPlayableCharacter=true")?["data"].as_array().into_iter().flatten() {
            c.names.insert(id(a), nm(a));
            let thumb = a["displayIconSmall"].as_str().and_then(|url| {
                let mut bytes = Vec::new();
                ureq::get(url).call().ok()?.into_reader().read_to_end(&mut bytes).ok()?;
                let (px, w, h) = decode_png(&bytes)?;
                Some(shrink(&px, w, h))
            });
            if let Some(t) = thumb {
                c.icons.insert(nm(a), t);
            }
        }
        for w in get("weapons")?["data"].as_array().into_iter().flatten() {
            let wid = id(w);
            c.names.insert(wid.clone(), nm(w));
            // The white silhouette the scoreboard shows for this gun.
            if let Some((px, iw, ih)) = w["killStreamIcon"].as_str().and_then(fetch_png) {
                if let Some(m) = alpha_mask(&px, iw, ih) {
                    c.gun_masks.insert(nm(w), m);
                }
            }
            for s in w["skins"].as_array().into_iter().flatten() {
                c.names.insert(id(s), nm(s));
                c.skin_weapon.insert(id(s), wid.clone());
                if let Some(url) = s["displayIcon"].as_str() {
                    c.skin_art.insert(id(s), url.to_owned());
                }
                for ch in s["chromas"].as_array().into_iter().flatten() {
                    if let Some(url) = ch["fullRender"].as_str().or(ch["displayIcon"].as_str()) {
                        c.skin_art.insert(id(ch), url.to_owned());
                    }
                }
            }
        }
        for b in get("buddies")?["data"].as_array().into_iter().flatten() {
            for l in b["levels"].as_array().into_iter().flatten() {
                c.names.insert(id(l), nm(b));
            }
        }
        Ok(c)
    }
}
