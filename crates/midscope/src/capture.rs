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
