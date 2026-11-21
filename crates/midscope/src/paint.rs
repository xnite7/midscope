//! Direct2D painting into a layered window. Callers build a list of
//! primitives; `paint` draws them into a DIB and hands it to the window.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{w, Interface, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
    D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES,
    D2D1_ROUNDED_RECT,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory5, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STRETCH_SEMI_CONDENSED, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BLACK, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_MEDIUM,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

use midscope_core::layout::Rect;

// Team colours stay VALORANT's, so what we add reads as part of the game.
pub const RED: u32 = 0xFF4655;
pub const TEAL: u32 = 0x4FD1B4;

// midnite room tokens (couch design system).
pub const ROOM0: u32 = 0x000000;
pub const ROOM1: u32 = 0x0A0809;
pub const ROOM2: u32 = 0x141113;
#[allow(dead_code)] // full token set, kept for parity with couch
pub const ROOM3: u32 = 0x1D181B;
#[allow(dead_code)] // full token set, kept for parity with couch
pub const PLUM: u32 = 0x7A3B5E;
/// Warm white text, never pure white.
pub const TEXT: u32 = 0xF4EAF0;
/// ink-2 / ink-3 / ink-4, pre-blended onto the room.
pub const MUTED: u32 = 0x928C90;
#[allow(dead_code)] // full token set, kept for parity with couch
pub const INK3: u32 = 0x534F52;
pub const INK4: u32 = 0x272527;
/// Quiet control fill (plum-grey 30%).
pub const FILL: u32 = 0x24211F;
pub const LAMP: u32 = 0xFFB25C;
#[allow(dead_code)] // full token set, kept for parity with couch
pub const ON_LAMP: u32 = 0x120A06;
pub const NEON: u32 = 0xE0309A;
#[allow(dead_code)] // full token set, kept for parity with couch
pub const NEON_SOFT: u32 = 0xFF6EC4;
#[allow(dead_code)] // full token set, kept for parity with couch
pub const GOOD: u32 = 0x7FD6A0;
pub const DANGER: u32 = 0xFF5A5A;
/// Old names used by the views.
pub const INK: u32 = ROOM1;
pub const CELL: u32 = ROOM2;
/// Revealed names: lamp, "this is now".
pub const YELLOW: u32 = LAMP;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    /// Player names.
    Name,
    Small,
    SmallBold,
    /// Small, bold, centred (labels on lines, buttons).
    Label,
    /// Section headings and setting names.
    Heading,
    /// Body text in the settings page.
    Body,
    /// The big page title (hero, Black).
    Title,
    /// Section headings (title2, Bold).
    Title2,
    /// Pill button labels (headline, centred).
    Button,
}

impl Font {
    const ALL: [Font; 9] = [Font::Name, Font::Small, Font::SmallBold, Font::Label, Font::Heading, Font::Body, Font::Title, Font::Title2, Font::Button];
    /// Satoshi, heavy at the top and quiet underneath (couch type scale).
    fn spec(self) -> (f32, DWRITE_FONT_WEIGHT, DWRITE_TEXT_ALIGNMENT) {
        let (lead, mid) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER);
        match self {
            Font::Name => (16.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, lead),
            Font::Small => (13.0, DWRITE_FONT_WEIGHT_MEDIUM, lead),
            Font::SmallBold => (13.0, DWRITE_FONT_WEIGHT_BOLD, lead),
            Font::Label => (11.5, DWRITE_FONT_WEIGHT_BOLD, mid),
            Font::Heading => (17.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, lead),
            Font::Body => (15.0, DWRITE_FONT_WEIGHT_MEDIUM, lead),
            Font::Title => (72.0, DWRITE_FONT_WEIGHT_BLACK, lead),
            Font::Title2 => (24.0, DWRITE_FONT_WEIGHT_BOLD, lead),
            Font::Button => (15.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, mid),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Img {
    /// A rank icon by tier (downloaded once into the data folder).
    Rank(u32),
    /// The midnite "m" sticker, built into the exe.
    Logo,
    /// A weapon skin picture (by its url; cached on disk).
    Skin(String),
}

pub enum Prim {
    Fill(Rect, u32, f32),
    Outline(Rect, u32),
    /// Rounded fill: rect, radius, colour, alpha. Everything you press is a pill.
    Round(Rect, f32, u32, f32),
    /// Rounded hairline: rect, radius, colour, width.
    RoundLine(Rect, f32, u32, f32),
    Line(f32, f32, f32, f32, u32, f32),
    Text(String, Font, Rect, u32),
    /// A soft pool of light: centre, radius, colour, strength.
    Glow(f32, f32, f32, u32, f32),
    Image(Img, Rect),
}

/// Back-compat for the views: a rank icon.
#[allow(non_snake_case)]
pub fn Rank(tier: u32, r: Rect) -> Prim {
    Prim::Image(Img::Rank(tier), r)
}

pub fn text(s: impl Into<String>, f: Font, r: Rect, c: u32) -> Prim {
    Prim::Text(s.into(), f, r, c)
}

fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    let c = |s: u32| ((rgb >> s) & 0xFF) as f32 / 255.0 * a; // premultiplied
    D2D1_COLOR_F { r: c(16), g: c(8), b: c(0), a }
}

fn d2r(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h }
}

/// Satoshi ships inside the exe; it's written to the data folder once and
/// loaded as a private font set (nothing gets installed on the system).
const SATOSHI: [(&str, &[u8]); 4] = [
    ("Satoshi-Medium.ttf", include_bytes!("../../../assets/Satoshi-Medium.ttf")),
    ("Satoshi-SemiBold.ttf", include_bytes!("../../../assets/Satoshi-SemiBold.ttf")),
    ("Satoshi-Bold.ttf", include_bytes!("../../../assets/Satoshi-Bold.ttf")),
    ("Satoshi-Black.ttf", include_bytes!("../../../assets/Satoshi-Black.ttf")),
];

fn satoshi(dw: &IDWriteFactory) -> Option<IDWriteFontCollection> {
    let dir = midscope_core::config::data_dir().join("fonts");
    let _ = std::fs::create_dir_all(&dir);
    unsafe {
        let f5: IDWriteFactory5 = dw.cast().ok()?;
        let builder = f5.CreateFontSetBuilder().ok()?;
        for (name, bytes) in SATOSHI {
            let path = dir.join(name);
            if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) != bytes.len() as u64 {
                std::fs::write(&path, bytes).ok()?;
            }
            let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
            let file = f5.CreateFontFileReference(PCWSTR(wide.as_ptr()), None).ok()?;
            builder.AddFontFile(&file).ok()?;
        }
        let set = builder.CreateFontSet().ok()?;
        f5.CreateFontCollectionFromFontSet(&set).ok()?.cast().ok()
    }
}

fn text_format(dw: &IDWriteFactory, fonts: Option<&IDWriteFontCollection>, f: Font, scale: f32) -> Result<IDWriteTextFormat> {
    let (size, weight, align) = f.spec();
    unsafe {
        let t = match fonts {
            Some(c) => dw.CreateTextFormat(w!("Satoshi"), c, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * scale, w!("en-us"))?,
            // Fallback: Bahnschrift ships with Windows 10+.
            None => dw.CreateTextFormat(w!("Bahnschrift"), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_SEMI_CONDENSED, size * scale, w!("en-us"))?,
        };
        t.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        t.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        t.SetTextAlignment(align)?;
        Ok(t)
    }
}

pub struct Painter {
    hwnd: HWND,
    _factory: ID2D1Factory,
    target: ID2D1DCRenderTarget,
    dw: IDWriteFactory,
    collection: Option<IDWriteFontCollection>,
    scale: f32,
    fonts: HashMap<Font, IDWriteTextFormat>,
    images: RefCell<HashMap<Img, Option<ID2D1Bitmap>>>,
    size: (i32, i32),
}

impl Painter {
    pub fn new(hwnd: HWND, size: (i32, i32), scale: f32) -> Result<Painter> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            })?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let collection = satoshi(&dw);
            let mut p = Painter {
                hwnd,
                _factory: factory,
                target,
                dw,
                collection,
                scale: 0.0,
                fonts: HashMap::new(),
                images: RefCell::new(HashMap::new()),
                size,
            };
            p.set_scale(scale);
            Ok(p)
        }
    }

    /// Text follows the game window's UI scale (windowed mode is smaller).
    pub fn set_scale(&mut self, scale: f32) {
        if (scale - self.scale).abs() < 0.01 {
            return;
        }
        let mut fonts = HashMap::new();
        for f in Font::ALL {
            match text_format(&self.dw, self.collection.as_ref(), f, scale) {
                Ok(t) => {
                    fonts.insert(f, t);
                }
                Err(_) => return,
            }
        }
        self.fonts = fonts;
        self.scale = scale;
    }

    /// Loads an image once and keeps it as a GPU bitmap.
    fn image(&self, img: &Img) -> Option<ID2D1Bitmap> {
        if let Some(b) = self.images.borrow().get(img) {
            return b.clone();
        }
        let decoded = match img {
            Img::Rank(tier) => std::fs::read(midscope_core::content::rank_icon_path(*tier))
                .ok()
                .and_then(|b| midscope_core::content::decode_png(&b)),
            Img::Logo => midscope_core::content::decode_png(include_bytes!("../../../assets/logo-160.png")),
            Img::Skin(url) => midscope_core::content::load_skin_art(url),
        };
        // A skin still downloading isn't cached as missing; it's tried again next paint.
        if decoded.is_none() && matches!(img, Img::Skin(_)) {
            return None;
        }
        let bitmap = decoded
            .and_then(|(rgba, w, h)| {
                // RGBA -> premultiplied BGRA.
                let bgra: Vec<u8> = rgba
                    .chunks(4)
                    .flat_map(|p| {
                        let a = p[3] as u32;
                        let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
                        [m(p[2]), m(p[1]), m(p[0]), p[3]]
                    })
                    .collect();
                let props = D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                };
                unsafe {
                    self.target
                        .CreateBitmap(D2D_SIZE_U { width: w as u32, height: h as u32 }, Some(bgra.as_ptr() as _), (w * 4) as u32, &props)
                        .ok()
                }
            });
        self.images.borrow_mut().insert(img.clone(), bitmap.clone());
        bitmap
    }

    pub fn paint(&self, prims: &[Prim]) -> Result<()> {
        let (w, h) = self.size;
        unsafe {
            let screen_dc = GetDC(None);
            let mem = CreateCompatibleDC(screen_dc);
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
            let dib = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(mem, dib);

            let t = &self.target;
            let drawn = (|| -> Result<()> {
                t.BindDC(mem, &RECT { left: 0, top: 0, right: w, bottom: h })?;
                t.BeginDraw();
                t.Clear(Some(&color(0, 0.0)));
                for p in prims {
                    match p {
                        Prim::Fill(r, rgb, a) => t.FillRectangle(&d2r(*r), &t.CreateSolidColorBrush(&color(*rgb, *a), None)?),
                        Prim::Outline(r, rgb) => t.DrawRectangle(&d2r(*r), &t.CreateSolidColorBrush(&color(*rgb, 1.0), None)?, 2.0, None),
                        Prim::Line(x1, y1, x2, y2, rgb, width) => t.DrawLine(
                            D2D_POINT_2F { x: *x1, y: *y1 },
                            D2D_POINT_2F { x: *x2, y: *y2 },
                            &t.CreateSolidColorBrush(&color(*rgb, 1.0), None)?,
                            *width,
                            None,
                        ),
                        Prim::Text(s, f, r, rgb) => {
                            let Some(fmt) = self.fonts.get(f) else { continue };
                            let wide: Vec<u16> = s.encode_utf16().collect();
                            t.DrawText(
                                &wide,
                                fmt,
                                &d2r(*r),
                                &t.CreateSolidColorBrush(&color(*rgb, 1.0), None)?,
                                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                                DWRITE_MEASURING_MODE_NATURAL,
                            )
                        }
                        Prim::Image(img, r) => {
                            if let Some(b) = self.image(img) {
                                t.DrawBitmap(&b, Some(&d2r(*r)), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
                            }
                        }
                        Prim::Round(r, rad, rgb, a) => t.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT { rect: d2r(*r), radiusX: *rad, radiusY: *rad },
                            &t.CreateSolidColorBrush(&color(*rgb, *a), None)?,
                        ),
                        Prim::RoundLine(r, rad, rgb, width) => t.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT { rect: d2r(*r), radiusX: *rad, radiusY: *rad },
                            &t.CreateSolidColorBrush(&color(*rgb, 1.0), None)?,
                            *width,
                            None,
                        ),
                        Prim::Glow(cx, cy, radius, rgb, a) => {
                            // A disc of light fading to nothing: a pool, not a shape.
                            let stops = [
                                D2D1_GRADIENT_STOP { position: 0.0, color: color(*rgb, *a) },
                                D2D1_GRADIENT_STOP { position: 0.45, color: color(*rgb, *a * 0.45) },
                                D2D1_GRADIENT_STOP { position: 1.0, color: color(*rgb, 0.0) },
                            ];
                            let coll = t.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                            let brush = t.CreateRadialGradientBrush(
                                &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                                    center: D2D_POINT_2F { x: *cx, y: *cy },
                                    gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                                    radiusX: *radius,
                                    radiusY: *radius,
                                },
                                None,
                                &coll,
                            )?;
                            t.FillEllipse(
                                &D2D1_ELLIPSE { point: D2D_POINT_2F { x: *cx, y: *cy }, radiusX: *radius, radiusY: *radius },
                                &brush,
                            );
                        }
                    }
                }
                t.EndDraw(None, None)
            })();

            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let shown = UpdateLayeredWindow(
                self.hwnd,
                screen_dc,
                Some(&POINT { x: 0, y: 0 }),
                Some(&SIZE { cx: w, cy: h }),
                mem,
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(dib);
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen_dc);
            drawn?;
            shown
        }
    }
}
