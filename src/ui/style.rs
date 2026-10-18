//! The translucent purple look, shared by the search and settings views.

use iced::widget::{container, scrollable, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, theme};

pub const PURPLE: Color = Color::from_rgb(0.71, 0.53, 1.0);
pub const WHITE: Color = Color::from_rgb(0.94, 0.93, 0.98);
pub const MUTED: Color = Color::from_rgb(0.57, 0.55, 0.65);
pub const ERROR: Color = Color::from_rgb(1., 0.66, 0.73);

/// Corner radius of the panel. Windows 11 rounds undecorated windows itself
/// with a smaller radius, so match it there instead of drawing over it.
pub const PANEL_RADIUS: f32 = if cfg!(windows) { 8.0 } else { 24.0 };

pub fn theme() -> Theme {
    Theme::custom(
        "Spotlight".to_string(),
        theme::Palette {
            background: Color::from_rgb(0.065, 0.052, 0.095),
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

/// Lowest fill alpha when no native blur is behind the panel. Anything lower
/// lets the windows behind show through and makes the text unreadable.
pub const OPAQUE_FLOOR: f32 = 0.92;

/// The glass panel. With a native backdrop (acrylic or mica on Windows) the
/// fill is lighter so the blur shows through. Without one the fill is nearly
/// opaque, whatever `opacity` asks for.
pub fn panel(opacity: f32, backdrop: bool) -> container::Style {
    let alpha = if backdrop {
        opacity * 0.55
    } else {
        opacity.max(OPAQUE_FLOOR)
    };
    container::Style {
        text_color: Some(WHITE),
        background: Some(Color::from_rgba(0.065, 0.052, 0.095, alpha).into()),
        border: Border {
            color: Color::from_rgba(0.68, 0.53, 0.93, 0.27),
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
        background: Some(Color::from_rgba(0.65, 0.55, 0.8, 0.14).into()),
        ..Default::default()
    }
}

pub fn chip(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(0.52, 0.31, 0.91, 0.16).into()),
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
        background: (active || hovered)
            .then(|| Color::from_rgba(0.49, 0.31, 0.78, if active { 0.24 } else { 0.12 }).into()),
        text_color: WHITE,
        border: Border {
            radius: 11.0.into(),
            width: if active { 1. } else { 0. },
            color: Color::from_rgba(0.62, 0.44, 0.9, 0.28),
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
        selection: Color::from_rgba(0.5, 0.3, 0.8, 0.5),
    }
}

/// Settings fields get a faint outline that brightens on focus.
pub fn field_input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(Color::from_rgba(0.49, 0.31, 0.78, 0.08)),
        border: Border {
            radius: 8.0.into(),
            width: 1.,
            color: Color::from_rgba(0.62, 0.44, 0.9, if focused { 0.5 } else { 0.18 }),
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
            background: Color::from_rgba(0.62, 0.44, 0.9, if active { 0.45 } else { 0.22 }).into(),
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
            background: Color::from_rgba(0.065, 0.052, 0.095, 0.9).into(),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: PURPLE,
        },
    }
}
