//! Result icons: themed application icons on Linux, plus small built-in
//! symbolic glyphs used everywhere else (and on Windows, where the core does
//! not extract shortcut icons).
//!
//! All SVGs and PNGs are pre-rasterized to 56x56 RGBA buffers in RAM off the
//! UI thread (`image::Handle::from_rgba`) so opening a window never triggers
//! `iced_wgpu`'s synchronous `fontdb.load_system_fonts()` scan or XML parsing
//! on the UI thread.

use iced::Color;
use iced::widget::image;
use resvg::{tiny_skia, usvg};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const RASTER_PX: u32 = 56;

/// Pre-rasterized RGBA icon handle.
#[derive(Clone, Debug)]
pub enum Icon {
    Raster(image::Handle),
}

/// Resolved application icons, keyed by the desktop entry's `Icon=` value.
pub type IconMap = HashMap<String, Icon>;

const SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/></svg>"#;
const CHAT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linejoin="round"><path d="M4 5h16v11H9l-5 4z"/><path d="M12 8v5M9.5 10.5h5" stroke-linecap="round"/></svg>"#;
const APP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"><rect x="3.5" y="3.5" width="7" height="7" rx="2"/><rect x="13.5" y="3.5" width="7" height="7" rx="2"/><rect x="3.5" y="13.5" width="7" height="7" rx="2"/><rect x="13.5" y="13.5" width="7" height="7" rx="2"/></svg>"#;
const FILE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4M9 12h6M9 16h6" stroke-linecap="round"/></svg>"#;
const WEB: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8"><circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c-3 3.2-3 13.8 0 17M12 3.5c3 3.2 3 13.8 0 17"/></svg>"#;

/// Built-in symbolic glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    Search,
    Chat,
    App,
    File,
    Web,
}

static GLYPH_CACHE: LazyLock<Mutex<HashMap<(Glyph, [u8; 4]), image::Handle>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl Glyph {
    pub fn handle(self, color: Color) -> image::Handle {
        let rgba8 = color.into_rgba8();
        let mut cache = GLYPH_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(handle) = cache.get(&(self, rgba8)) {
            return handle.clone();
        }
        let svg_str = match self {
            Self::Search => SEARCH,
            Self::Chat => CHAT,
            Self::App => APP,
            Self::File => FILE,
            Self::Web => WEB,
        };
        let handle = rasterize_svg_bytes(svg_str.as_bytes(), RASTER_PX, Some(rgba8))
            .unwrap_or_else(|| image::Handle::from_rgba(1, 1, vec![0, 0, 0, 0]));
        cache.insert((self, rgba8), handle.clone());
        handle
    }
}

fn rasterize_svg_bytes(
    bytes: &[u8],
    target_px: u32,
    tint: Option<[u8; 4]>,
) -> Option<image::Handle> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(target_px, target_px)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let scale = (target_px as f32 / w).min(target_px as f32 / h);
    let tx = (target_px as f32 - w * scale) * 0.5;
    let ty = (target_px as f32 - h * scale) * 0.5;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }))
    .ok()?;
    let mut rgba = pixmap.take();
    if let Some([r, g, b, _]) = tint {
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            if a > 0 {
                // Premultiplied -> straight RGBA for `image::Handle::from_rgba`
                px[0] = r;
                px[1] = g;
                px[2] = b;
            }
        }
    } else {
        // Convert tiny_skia premultiplied RGBA to straight RGBA expected by iced
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3] as u16;
            if a > 0 && a < 255 {
                px[0] = ((px[0] as u16 * 255 + a / 2) / a).min(255) as u8;
                px[1] = ((px[1] as u16 * 255 + a / 2) / a).min(255) as u8;
                px[2] = ((px[2] as u16 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(image::Handle::from_rgba(target_px, target_px, rgba))
}

#[cfg(target_os = "linux")]
fn load_raster_path(path: &std::path::Path) -> Option<image::Handle> {
    let bytes = std::fs::read(path).ok()?;
    let decoded = ::image::load_from_memory(&bytes).ok()?;
    let rgba = if decoded.width() > RASTER_PX || decoded.height() > RASTER_PX {
        decoded
            .resize(
                RASTER_PX,
                RASTER_PX,
                ::image::imageops::FilterType::Triangle,
            )
            .to_rgba8()
    } else {
        decoded.to_rgba8()
    };
    let (w, h) = rgba.dimensions();
    Some(image::Handle::from_rgba(w, h, rgba.into_raw()))
}

/// Resolve icon names (or absolute paths) for the given apps. Theme lookups
/// and rasterization touch the filesystem, so call this off the UI thread.
#[cfg(target_os = "linux")]
pub fn resolve<'a>(names: impl IntoIterator<Item = &'a str>) -> IconMap {
    let theme = icon_theme();
    let mut icons = IconMap::new();
    for name in names {
        if icons.contains_key(name) {
            continue;
        }
        let path = if std::path::Path::new(name).is_absolute() {
            Some(std::path::PathBuf::from(name))
        } else {
            freedesktop_icons::lookup(name)
                .with_size(48)
                .with_theme(&theme)
                .with_cache()
                .find()
        };
        let Some(path) = path.filter(|p| p.is_file()) else {
            continue;
        };
        let handle = match path.extension().and_then(|e| e.to_str()) {
            Some("svg" | "svgz") => std::fs::read(&path)
                .ok()
                .and_then(|bytes| rasterize_svg_bytes(&bytes, RASTER_PX, None)),
            Some("png") => load_raster_path(&path),
            // XPM and other legacy formats fall back to the built-in glyph.
            _ => continue,
        };
        if let Some(handle) = handle {
            icons.insert(name.to_owned(), Icon::Raster(handle));
        }
    }
    icons
}

#[cfg(not(target_os = "linux"))]
pub fn resolve<'a>(_names: impl IntoIterator<Item = &'a str>) -> IconMap {
    IconMap::new()
}

/// The user's icon theme: COSMIC's setting first, then GTK's, then hicolor.
#[cfg(target_os = "linux")]
fn icon_theme() -> String {
    let cosmic = dirs::config_dir()
        .map(|dir| dir.join("cosmic/com.system76.CosmicTk/v1/icon_theme"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty());
    cosmic
        .or_else(freedesktop_icons::default_theme_gtk)
        .unwrap_or_else(|| "hicolor".into())
}
