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
            .filter(|c| !c.icons.is_empty())
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
            for s in w["skins"].as_array().into_iter().flatten() {
                c.names.insert(id(s), nm(s));
                c.skin_weapon.insert(id(s), wid.clone());
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
