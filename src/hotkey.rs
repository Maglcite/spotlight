//! The global launcher shortcut. Spotlight registers it itself on Windows and
//! X11; Wayland compositors own global shortcuts, so there the desktop runs
//! `spotlight --toggle` instead.

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use iced::futures::Stream;

/// Whether this session lets applications grab a global shortcut.
pub fn supported() -> bool {
    if cfg!(windows) {
        return true;
    }
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    cfg!(target_os = "linux") && set("DISPLAY") && !set("WAYLAND_DISPLAY")
}

/// Parse a shortcut such as `Alt+Space` or `Ctrl+Shift+K`. `Win` and
/// `Windows` are accepted as spellings of `Super`.
pub fn parse(spec: &str) -> Result<HotKey, String> {
    let normalized = spec
        .split('+')
        .map(|part| {
            let part = part.trim();
            if part.eq_ignore_ascii_case("win") || part.eq_ignore_ascii_case("windows") {
                "Super"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("+");
    normalized
        .parse::<HotKey>()
        .map_err(|_| format!("“{}” is not a shortcut like Alt+Space.", spec.trim()))
}

/// Owns the OS registration. On Windows the manager must be created on the
/// event-loop thread, because the hotkey messages are delivered there.
pub struct Hotkey {
    manager: Option<GlobalHotKeyManager>,
    current: Option<HotKey>,
}

impl Hotkey {
    pub fn new() -> Self {
        let manager = if supported() {
            GlobalHotKeyManager::new()
                .map_err(|error| tracing::warn!("global shortcuts are unavailable: {error}"))
                .ok()
        } else {
            None
        };
        Self {
            manager,
            current: None,
        }
    }

    pub fn available(&self) -> bool {
        self.manager.is_some()
    }

    /// Replace the registered shortcut. On failure the previous one stays.
    pub fn register(&mut self, spec: &str) -> Result<(), String> {
        let hotkey = parse(spec)?;
        let Some(manager) = &self.manager else {
            return Ok(());
        };
        if self.current == Some(hotkey) {
            return Ok(());
        }
        if let Some(previous) = self.current.take() {
            let _ = manager.unregister(previous);
        }
        match manager.register(hotkey) {
            Ok(()) => {
                self.current = Some(hotkey);
                Ok(())
            }
            Err(error) => {
                if let Some(previous) = self.current {
                    let _ = manager.register(previous);
                }
                Err(format!(
                    "Could not register {}: {error}. Another app may already use it.",
                    spec.trim()
                ))
            }
        }
    }
}

/// Shortcut presses, for a subscription. The handler runs on whichever thread
/// the platform delivers events on and only forwards them.
pub fn presses() -> impl Stream<Item = ()> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state() == HotKeyState::Pressed {
            let _ = tx.send(());
        }
    }));
    iced::futures::stream::unfold(
        rx,
        |mut rx| async move { rx.recv().await.map(|()| ((), rx)) },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    #[test]
    fn shortcuts_parse_with_platform_spellings() {
        assert_eq!(
            parse("Alt+Space").unwrap(),
            HotKey::new(Some(Modifiers::ALT), Code::Space)
        );
        assert_eq!(
            parse(" win + space ").unwrap(),
            parse("Super+Space").unwrap()
        );
        assert_eq!(
            parse("Ctrl+Shift+K").unwrap(),
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyK)
        );
        for spec in ["", "Alt+", "Space+Alt+K", "Hyper+Banana"] {
            assert!(parse(spec).is_err(), "{spec}");
        }
        assert!(parse(spotlight::config::default_hotkey()).is_ok());
    }
}
