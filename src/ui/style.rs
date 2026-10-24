//! The translucent purple look, shared by the search and settings views and
//! matched to purpwm's compositor theme.

use iced::widget::{container, scrollable, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, theme};

/// purpwm `BAR_ACCENT_RGBA` / `COLOR_BORDER_GLOW`.
pub const PURPLE: Color = Color::from_rgb(0.753, 0.518, 0.988);
/// purpwm `BAR_TEXT_FG_RGBA`.
pub const WHITE: Color = Color::from_rgb(0.953, 0.910, 1.0);
/// purpwm `BAR_TEXT_DIM_RGBA`.
pub const MUTED: Color = Color::from_rgb(0.545, 0.478, 0.659);
/// purpwm `COLOR_BG_ROOT`.
pub const ROOT: Color = Color::from_rgb(0.067, 0.031, 0.122);
/// purpwm `BAR_BG_RGBA`.
pub const PANEL: Color = Color::from_rgb(0.090, 0.043, 0.161);
/// purpwm `BAR_LAYOUT_BG_RGBA`.
pub const PILL: Color = Color::from_rgb(0.165, 0.078, 0.278);
/// purpwm `BAR_TAG_ACTIVE_RGBA`.
pub const ACTIVE: Color = Color::from_rgb(0.486, 0.227, 0.929);
/// purpwm `BAR_ACCENT_LINE_RGBA`.
pub const SELECTION: Color = Color::from_rgb(0.576, 0.200, 0.918);
/// purpwm `COLOR_BORDER_ACTIVE`.
pub const BORDER_ACTIVE: Color = Color::from_rgb(0.659, 0.333, 0.969);

pub const ERROR: Color = Color::from_rgb(1., 0.66, 0.73);

/// The bundled font used by both purpwm's compositor and Spotlight, so the
/// launcher always matches the session typography even without system fonts.
pub const BUNDLED_FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSansMono-Regular.ttf");
pub const DEFAULT_FONT: Font = Font::with_name("Noto Sans Mono");

/// Corner radius of the panel. Windows 11 rounds undecorated windows itself,
/// so match it there; on Wayland purpwm frames the whole layer surface, so a
/// small radius keeps the panel inside that rectangular frame.
pub const PANEL_RADIUS: f32 = 8.0;

pub fn theme() -> Theme {
    Theme::custom(
        "Spotlight".to_string(),
        theme::Palette {
            background: ROOT,
            text: WHITE,
            primary: PURPLE,
            success: Color::from_rgb(0.55, 0.85, 0.65),
            warning: Color::from_rgb(1., 0.8, 0.45),
            danger: ERROR,
        },
    )
}

/// The window itself stays transparent; only the panel is painted.
pub fn application(_theme: &Theme) -> theme::Style {
    theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: WHITE,
    }
}

/// Lowest fill alpha when no native blur is behind the panel. This matches
/// purpwm's status bar opacity while keeping overlay text readable.
pub const OPAQUE_FLOOR: f32 = 0.82;

/// The glass panel. With a native backdrop (acrylic or mica on Windows) the
/// fill is lighter so the blur shows through. Without one the configured
/// opacity applies down to [`OPAQUE_FLOOR`].
pub fn panel(opacity: f32, backdrop: bool) -> container::Style {
    let alpha = if backdrop {
        opacity * 0.55
    } else {
        opacity.max(OPAQUE_FLOOR)
    };
    container::Style {
        text_color: Some(WHITE),
        background: Some(Color::from_rgba(PANEL.r, PANEL.g, PANEL.b, alpha).into()),
        border: Border {
            color: Color::from_rgba(BORDER_ACTIVE.r, BORDER_ACTIVE.g, BORDER_ACTIVE.b, 0.35),
            width: 1.,
            radius: PANEL_RADIUS.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0., 0., 0., 0.5),
            offset: Vector::new(0., 14.),
            blur_radius: 40.,
        },
        ..Default::default()
    }
}

pub fn rule(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(PURPLE.r, PURPLE.g, PURPLE.b, 0.14).into()),
        ..Default::default()
    }
}

pub fn chip(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(ACTIVE.r, ACTIVE.g, ACTIVE.b, 0.16).into()),
        border: Border {
            radius: 5.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn button(active: bool, status: iced::widget::button::Status) -> iced::widget::button::Style {
    let hovered = matches!(
        status,
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed
    );
    iced::widget::button::Style {
        background: (active || hovered).then(|| {
            Color::from_rgba(
                ACTIVE.r,
                ACTIVE.g,
                ACTIVE.b,
                if active { 0.24 } else { 0.12 },
            )
            .into()
        }),
        text_color: WHITE,
        border: Border {
            radius: 11.0.into(),
            width: if active { 1. } else { 0. },
            color: Color::from_rgba(PURPLE.r, PURPLE.g, PURPLE.b, 0.28),
        },
        ..Default::default()
    }
}

/// Borderless input for the main search field.
pub fn search_input(_theme: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        icon: PURPLE,
        placeholder: MUTED,
        value: WHITE,
        selection: Color::from_rgba(SELECTION.r, SELECTION.g, SELECTION.b, 0.5),
    }
}

/// Settings fields get a faint outline that brightens on focus.
pub fn field_input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(Color::from_rgba(PILL.r, PILL.g, PILL.b, 0.35)),
        border: Border {
            radius: 8.0.into(),
            width: 1.,
            color: Color::from_rgba(
                PURPLE.r,
                PURPLE.g,
                PURPLE.b,
                if focused { 0.5 } else { 0.18 },
            ),
        },
        ..search_input(_theme, status)
    }
}

/// A thin purple scroller without a visible rail.
pub fn scrollbar(_theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let active = !matches!(status, scrollable::Status::Active { .. });
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Color::from_rgba(
                PURPLE.r,
                PURPLE.g,
                PURPLE.b,
                if active { 0.45 } else { 0.22 },
            )
            .into(),
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Color::from_rgba(PANEL.r, PANEL.g, PANEL.b, 0.9).into(),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: PURPLE,
        },
    }
}
