//! Copies small screen regions (agent portraits on the scoreboard).
//! Plain GDI BitBlt from the desktop, like any screenshot tool; works with
//! VALORANT in windowed fullscreen. Nothing touches the game process.

use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

/// Returns BGRA pixels of the region (stride = w * 4), or None on failure.
pub fn grab(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let out = match CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(dib) => {
                let old = SelectObject(mem, dib);
                let ok = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY).is_ok();
                let px = if ok {
                    let mut v = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
                    // GDI leaves alpha at 0; the matcher treats pixels as opaque.
                    v.chunks_mut(4).for_each(|p| p[3] = 255);
                    Some(v)
                } else {
                    None
                };
                SelectObject(mem, old);
                let _ = DeleteObject(dib);
                px
            }
            Err(_) => None,
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        out
    }
}

/// One capture of a screen region, cut up in memory afterwards. Each screen
/// grab costs a few milliseconds, so a read takes one grab, not twenty.
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Frame {
    pub fn grab(x: i32, y: i32, w: i32, h: i32) -> Option<Frame> {
        let px = grab(x, y, w, h)?;
        Some(Frame { x, y, w: w as usize, h: h as usize, px })
    }

    /// BGRA pixels of a sub-region (stride = w * 4), if it lies inside the frame.
    pub fn crop(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
        let (cx, cy) = (x - self.x, y - self.y);
        if cx < 0 || cy < 0 || w <= 0 || h <= 0 || (cx + w) as usize > self.w || (cy + h) as usize > self.h {
            return None;
        }
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h as usize {
            let start = ((cy as usize + row) * self.w + cx as usize) * 4;
            out.extend_from_slice(&self.px[start..start + w as usize * 4]);
        }
        Some(out)
    }

    /// RGB at a screen point.
    pub fn rgb(&self, x: f32, y: f32) -> Option<[u8; 3]> {
        let p = self.crop(x as i32, y as i32, 1, 1)?;
        Some([p[2], p[1], p[0]])
    }
}
