//! Result icons: themed application icons on Linux, plus small built-in
//! symbolic glyphs used everywhere else (and on Windows, where the core does
//! not extract shortcut icons).

use iced::widget::{image, svg};
use std::collections::HashMap;

/// Only Linux resolves theme icons, so other platforms never build these.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Debug)]
pub enum Icon {
    Svg(svg::Handle),
    Raster(image::Handle),
}

/// Resolved application icons, keyed by the desktop entry's `Icon=` value.
pub type IconMap = HashMap<String, Icon>;

const SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/></svg>"#;
const CHAT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linejoin="round"><path d="M4 5h16v11H9l-5 4z"/><path d="M12 8v5M9.5 10.5h5" stroke-linecap="round"/></svg>"#;
const APP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"><rect x="3.5" y="3.5" width="7" height="7" rx="2"/><rect x="13.5" y="3.5" width="7" height="7" rx="2"/><rect x="3.5" y="13.5" width="7" height="7" rx="2"/><rect x="13.5" y="13.5" width="7" height="7" rx="2"/></svg>"#;
const FILE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4M9 12h6M9 16h6" stroke-linecap="round"/></svg>"#;
const WEB: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8"><circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c-3 3.2-3 13.8 0 17M12 3.5c3 3.2 3 13.8 0 17"/></svg>"#;

/// Built-in symbolic glyphs. Tint them with an `svg` style.
#[derive(Clone, Copy, Debug)]
pub enum Glyph {
    Search,
    Chat,
    App,
    File,
    Web,
}

impl Glyph {
    pub fn handle(self) -> svg::Handle {
        // Handles hash their bytes, so recreating them per frame still hits
        // the renderer's cache.
        svg::Handle::from_memory(
            match self {
                Self::Search => SEARCH,
                Self::Chat => CHAT,
                Self::App => APP,
                Self::File => FILE,
                Self::Web => WEB,
            }
            .as_bytes(),
        )
    }
}

/// Resolve icon names (or absolute paths) for the given apps. Theme lookups
/// touch the filesystem, so call this off the UI thread.
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
        let icon = match path.extension().and_then(|e| e.to_str()) {
            Some("svg" | "svgz") => Icon::Svg(svg::Handle::from_path(path)),
            Some("png") => Icon::Raster(image::Handle::from_path(path)),
            // XPM and other legacy formats fall back to the built-in glyph.
            _ => continue,
        };
        icons.insert(name.to_owned(), icon);
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
