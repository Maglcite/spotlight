//! The glass panel: the search list, the AI answer, and the pill buttons that
//! both pages share.

use super::{
    INPUT, Message, Page, RESULTS, Spotlight, VISIBLE_RESULTS,
    icons::{Glyph, Icon},
    question_prompt, settings, style,
};
use iced::widget::{
    Space, button, column, container, image, row, scrollable, svg, svg as svg_style, text,
    text_input,
};
use iced::{Alignment, Element, Length};
use spotlight::search::{ResultKind, SearchResult};

const ICON_SIZE: f32 = 28.0;

pub fn render(app: &Spotlight) -> Element<'_, Message> {
    let content = match app.page {
        Page::Search => search(app),
        Page::Settings => settings::view(app),
    };
    container(content)
        .padding([20, 24])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| style::panel(app.config.opacity, app.backdrop))
        .into()
}

/// A small text button. `active` gives it the purple selected look.
pub fn pill<'a>(label: &'a str, active: bool, message: Message) -> Element<'a, Message> {
    button(
        text(label)
            .size(12)
            .color(if active { style::PURPLE } else { style::MUTED }),
    )
    .padding([7, 12])
    .style(move |_, status| style::button(active, status))
    .on_press(message)
    .into()
}

fn search(app: &Spotlight) -> Element<'_, Message> {
    let ai_mode = question_prompt(&app.query).is_some();
    let input = text_input("Search anything…", &app.query)
        .id(INPUT.clone())
        .size(24)
        .padding([16, 0])
        .on_input(Message::Input)
        .on_submit(Message::Submit)
        .style(style::search_input);
    let search_row = row![
        glyph_sized(
            if ai_mode { Glyph::Chat } else { Glyph::Search },
            25.0,
            style::PURPLE
        ),
        input,
        container(
            text(if ai_mode { "ASK AI" } else { "SEARCH" })
                .size(10)
                .color(style::PURPLE)
        )
        .padding([5, 8])
        .style(style::chip)
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    let top = row![
        text("S P O T L I G H T").size(11).color(style::PURPLE),
        Space::new().width(Length::Fill),
        pill("Settings", false, Message::OpenSettings),
        pill("esc", false, Message::Escape)
    ]
    .align_y(Alignment::Center)
    .spacing(4);

    let mut content = column![top, search_row].spacing(7);
    content = content.push(rule());
    if ai_mode {
        content = content.push(answer(app));
    } else {
        content = content.push(results(app));
    }
    content = content.push(rule());
    let footer = if ai_mode {
        row![
            text("Read-only tools").size(11).color(style::MUTED),
            Space::new().width(Length::Fill),
            text(if app.answer.running {
                "esc to cancel & close"
            } else {
                "Edit your question to ask again"
            })
            .size(11)
            .color(style::MUTED)
        ]
    } else {
        row![
            text("↑ ↓ navigate    ↵ open").size(11).color(style::MUTED),
            Space::new().width(Length::Fill),
            text("End with  ?  to ask AI").size(11).color(style::PURPLE)
        ]
    };
    let footer = match (&app.error, ai_mode) {
        (Some(error), false) => {
            column![text(error).size(12).color(style::PURPLE), footer].spacing(6)
        }
        _ => column![footer],
    };
    content.push(footer.padding([10, 0])).into()
}

fn results(app: &Spotlight) -> Element<'_, Message> {
    let header = row![
        text(if app.query.is_empty() {
            "APPLICATIONS"
        } else {
            "BEST MATCHES"
        })
        .size(10)
        .color(style::MUTED),
        Space::new().width(Length::Fill),
        text(if app.indexing {
            "Indexing your files…".to_string()
        } else {
            format!("{} apps · {} files", app.apps.len(), app.files.len())
        })
        .size(10)
        .color(style::MUTED)
    ]
    .padding([9, 0]);

    let mut items = column![].spacing(4);
    for (i, result) in app.results.iter().take(VISIBLE_RESULTS).enumerate() {
        items = items.push(result_row(app, result, i == app.selected, i));
    }
    if app.results.is_empty() {
        items = items.push(
            container(
                column![
                    text(if app.indexing {
                        "Getting everything ready"
                    } else {
                        "No matches yet"
                    })
                    .size(22)
                    .color(style::WHITE),
                    text("Try an app name, a filename, or a web search.")
                        .size(13)
                        .color(style::MUTED)
                ]
                .spacing(12),
            )
            .padding([30, 12]),
        );
    }
    column![
        header,
        scrollable(items)
            .id(RESULTS.clone())
            .style(style::scrollbar)
            .height(Length::Fill)
    ]
    .into()
}

fn result_row<'a>(
    app: &'a Spotlight,
    result: &'a SearchResult,
    selected: bool,
    index: usize,
) -> Element<'a, Message> {
    let category = match result.kind {
        ResultKind::App { .. } => "APP",
        ResultKind::File { .. } => "FILE",
        ResultKind::Web { .. } => "WEB",
    };
    let entry = row![
        container(result_icon(app, result)).center_x(44),
        column![
            text(&result.title).size(15).color(style::WHITE),
            text(shorten(&result.subtitle, 74))
                .size(11)
                .color(style::MUTED)
        ]
        .spacing(3)
        .width(Length::Fill),
        text(category).size(9).color(if selected {
            style::PURPLE
        } else {
            style::MUTED
        }),
        text(if selected { "↵" } else { " " })
            .size(17)
            .color(style::PURPLE)
    ]
    .align_y(Alignment::Center)
    .spacing(10);
    button(entry)
        .width(Length::Fill)
        .padding([10, 10])
        .on_press(Message::Activate(index))
        .style(move |_, status| style::button(selected, status))
        .into()
}

/// Themed application icon when one was resolved, otherwise a built-in glyph.
fn result_icon<'a>(app: &'a Spotlight, result: &'a SearchResult) -> Element<'a, Message> {
    let resolved = result.icon.as_ref().and_then(|name| app.icons.get(name));
    match resolved {
        Some(Icon::Svg(handle)) => svg(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        Some(Icon::Raster(handle)) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        None => {
            let glyph = match result.kind {
                ResultKind::App { .. } => Glyph::App,
                ResultKind::File { .. } => Glyph::File,
                ResultKind::Web { .. } => Glyph::Web,
            };
            glyph_sized(glyph, ICON_SIZE, style::MUTED)
        }
    }
}

fn answer(app: &Spotlight) -> Element<'_, Message> {
    let state = &app.answer;
    let model = row![
        text("✦  AI ANSWER").size(11).color(style::PURPLE),
        Space::new().width(Length::Fill),
        text(format!("{} · {}", app.config.provider, app.config.model))
            .size(11)
            .color(style::MUTED)
    ]
    .align_y(Alignment::Center);
    let mut body = column![model].spacing(18);
    if state.text.is_empty() {
        let title = if state.running {
            "Thinking…"
        } else if state.error.is_some() {
            "Couldn’t finish this answer"
        } else {
            "Your answer will appear here"
        };
        body = body.push(text(title).size(23).color(style::WHITE));
        body = body.push(text(&state.status).size(13).color(style::MUTED));
    } else {
        body = body.push(
            text(&state.text)
                .size(16)
                .color(style::WHITE)
                .width(Length::Fill),
        );
        if state.running {
            body = body.push(text(&state.status).size(11).color(style::PURPLE));
        }
    }
    if let Some(error) = &state.error {
        body = body.push(text(error).size(13).color(style::ERROR));
        body = body.push(
            row![
                pill("Retry", true, Message::Retry),
                pill("AI settings", false, Message::OpenSettings)
            ]
            .spacing(8),
        );
    }
    if !state.text.is_empty() {
        let elapsed = state
            .elapsed
            .map(|d| format!("{:.1}s", d.as_secs_f32()))
            .unwrap_or_default();
        body = body.push(
            row![
                pill(
                    if state.copied {
                        "Copied"
                    } else {
                        "Copy answer"
                    },
                    true,
                    Message::Copy
                ),
                text(elapsed).size(11).color(style::MUTED)
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        );
    }
    scrollable(body.padding([16, 0]))
        .style(style::scrollbar)
        .height(Length::Fill)
        .into()
}

fn glyph_sized<'a>(glyph: Glyph, size: f32, color: iced::Color) -> Element<'a, Message> {
    svg(glyph.handle())
        .width(size)
        .height(size)
        .style(move |_, _| svg_style::Style { color: Some(color) })
        .into()
}

fn rule<'a>() -> Element<'a, Message> {
    container(Space::new().height(1))
        .width(Length::Fill)
        .style(style::rule)
        .into()
}

/// Cut `value` to `max` characters, ending with an ellipsis when it is longer.
fn shorten(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.into()
    } else {
        value
            .chars()
            .take(max - 1)
            .chain(std::iter::once('…'))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_subtitles_are_cut_with_an_ellipsis() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(shorten("ééé", 3), "ééé");
    }
}
