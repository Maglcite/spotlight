//! The settings page and its editable draft. Nothing reaches disk or the
//! keyring until Save, and a failed Save leaves the draft in place.

use super::{MODEL_INPUT, Message, Spotlight, style};
use crate::{autostart, hotkey};
use iced::widget::{Space, button, column, container, row, scrollable, text, text_input, toggler};
use iced::{Alignment, Element, Length};
use spotlight::config::{Config, Provider};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum Edit {
    Provider(Provider),
    Model(String),
    Endpoint(String),
    ApiKey(String),
    Roots(String),
    WebTools(bool),
    LocalTools(bool),
    WebEngine(String),
    Delay(u64),
    Opacity(f32),
    Hotkey(String),
    Autostart(bool),
}

/// Working copy of the settings. `config` holds everything except the raw text
/// fields, which are parsed by [`Draft::to_config`] when saved.
#[derive(Debug)]
pub struct Draft {
    pub config: Config,
    pub key: String,
    pub roots: String,
    pub saving: bool,
    pub error: Option<String>,
}

impl Draft {
    /// The draft starts from the saved config, with "start at login" read from
    /// the platform because that setting can change outside Spotlight.
    pub fn new(config: &Config) -> Self {
        let mut config = config.clone();
        config.autostart = autostart::is_enabled();
        Self {
            roots: join_roots(&config.search_roots),
            config,
            key: String::new(),
            saving: false,
            error: None,
        }
    }

    pub fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Provider(provider) => {
                self.config.provider = provider;
                self.config.base_url = provider.endpoint().into();
                self.config.model = if provider == Provider::OpenCodeGo {
                    Config::default().model
                } else {
                    String::new()
                };
                self.key.clear();
            }
            Edit::Model(value) => self.config.model = value,
            Edit::Endpoint(value) => self.config.base_url = value,
            Edit::ApiKey(value) => self.key = value,
            Edit::Roots(value) => self.roots = value,
            Edit::WebTools(value) => self.config.web_tools = value,
            Edit::LocalTools(value) => self.config.local_tools = value,
            Edit::WebEngine(value) => self.config.web_engine = value,
            Edit::Delay(value) => self.config.debounce_ms = value,
            Edit::Opacity(value) => self.config.opacity = value,
            Edit::Hotkey(value) => self.config.hotkey = value,
            Edit::Autostart(value) => self.config.autostart = value,
        }
    }

    /// The config as it would be saved: trimmed fields and parsed folders.
    pub fn to_config(&self) -> Config {
        let mut config = self.config.clone();
        config.model = config.model.trim().into();
        config.base_url = config.base_url.trim().trim_end_matches('/').into();
        config.hotkey = config.hotkey.trim().into();
        config.search_roots = parse_roots(&self.roots);
        config
    }
}

fn join_roots(roots: &[PathBuf]) -> String {
    roots
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("; ")
}

fn parse_roots(value: &str) -> Vec<PathBuf> {
    value
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenCodeGo => "Go",
        Provider::OpenAI => "OpenAI",
        Provider::Anthropic => "Claude",
        Provider::Gemini => "Gemini",
        Provider::OpenRouter => "OpenRouter",
        Provider::Ollama => "Ollama",
        Provider::Custom => "Custom",
    }
}

pub fn view(app: &Spotlight) -> Element<'_, Message> {
    let draft = &app.draft;
    let config = &draft.config;
    let header = row![
        text("Make it yours").size(25).color(style::WHITE),
        Space::new().width(Length::Fill),
        super::view::pill("Back", false, Message::Back)
    ]
    .align_y(Alignment::Center);

    let providers = Provider::ALL.into_iter().fold(row![].spacing(4), |r, p| {
        r.push(super::view::pill(
            provider_label(p),
            config.provider == p,
            Message::Field(Edit::Provider(p)),
        ))
    });
    let model = text_input("Model ID", &config.model)
        .id(MODEL_INPUT.clone())
        .on_input(|v| Message::Field(Edit::Model(v)))
        .style(style::field_input)
        .padding(10);
    let endpoint = text_input("https://provider.example/v1", &config.base_url)
        .on_input(|v| Message::Field(Edit::Endpoint(v)))
        .style(style::field_input)
        .padding(10);
    let key = text_input("Paste a new key to save in your system keyring", &draft.key)
        .secure(true)
        .on_input(|v| Message::Field(Edit::ApiKey(v)))
        .style(style::field_input)
        .padding(10);
    let roots = text_input("Absolute folders, separated by semicolons", &draft.roots)
        .on_input(|v| Message::Field(Edit::Roots(v)))
        .style(style::field_input)
        .padding(10);
    let tools = row![
        super::view::pill(
            if config.web_tools {
                "✓ Web tools"
            } else {
                "Web tools"
            },
            config.web_tools,
            Message::Field(Edit::WebTools(!config.web_tools)),
        ),
        super::view::pill(
            if config.local_tools {
                "✓ Local file tools"
            } else {
                "Local file tools"
            },
            config.local_tools,
            Message::Field(Edit::LocalTools(!config.local_tools)),
        )
    ]
    .spacing(8);
    let engines =
        ["duckduckgo", "google", "brave"]
            .into_iter()
            .fold(row![].spacing(6), |r, engine| {
                let label = match engine {
                    "google" => "Google",
                    "brave" => "Brave",
                    _ => "DuckDuckGo",
                };
                r.push(super::view::pill(
                    label,
                    config.web_engine == engine,
                    Message::Field(Edit::WebEngine(engine.into())),
                ))
            });
    let delay = row![
        text("Ask after").size(12).color(style::MUTED),
        super::view::pill(
            "400 ms",
            config.debounce_ms == 400,
            Message::Field(Edit::Delay(400))
        ),
        super::view::pill(
            "650 ms",
            config.debounce_ms == 650,
            Message::Field(Edit::Delay(650))
        ),
        super::view::pill(
            "1 second",
            config.debounce_ms == 1000,
            Message::Field(Edit::Delay(1000))
        )
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let opacity = row![
        text("Glass opacity").size(12).color(style::MUTED),
        super::view::pill(
            "75%",
            config.opacity == 0.75,
            Message::Field(Edit::Opacity(0.75))
        ),
        super::view::pill(
            "88%",
            config.opacity == 0.88,
            Message::Field(Edit::Opacity(0.88))
        ),
        super::view::pill(
            "100%",
            config.opacity == 1.0,
            Message::Field(Edit::Opacity(1.0))
        )
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let hotkey_input = text_input(hotkey_placeholder(), &config.hotkey)
        .on_input(|v| Message::Field(Edit::Hotkey(v)))
        .style(style::field_input)
        .padding(10);
    let hotkey_note = if app.hotkey.available() {
        "Press this anywhere to open Spotlight. Takes effect on Save."
    } else {
        "This session does not let apps grab shortcuts. Bind “spotlight --toggle” in your desktop’s keyboard settings."
    };

    let mut form = column![
        text("AI PROVIDER").size(10).color(style::PURPLE),
        providers,
        row![
            column![label("Model"), model].width(Length::Fill),
            column![label("API base URL"), endpoint].width(Length::Fill),
        ]
        .spacing(12),
        label("API key · saved securely, never in your config file"),
        key,
        tools,
        text("Web tools send searches and fetch public pages. Local tools can send text from your search folders to your chosen AI provider. Hidden files and credential files are excluded.")
            .size(11)
            .color(style::MUTED)
            .width(Length::Fill),
        rule(),
        label("Search folders"),
        roots,
        label("Web search engine"),
        engines,
        delay,
        opacity,
        rule(),
        label("Keyboard shortcut"),
        hotkey_input,
        text(hotkey_note).size(11).color(style::MUTED),
        toggler(config.autostart)
            .label("Start at login")
            .text_size(13)
            .on_toggle(|v| Message::Field(Edit::Autostart(v))),
    ]
    .spacing(12)
    .width(Length::Fill);
    if let Some(error) = &draft.error {
        form = form.push(text(error).size(12).color(style::ERROR));
    }

    let save_label = if draft.saving {
        "Saving…"
    } else {
        "Save settings"
    };
    column![
        header,
        text("A small launcher. Your providers. Your preferences.")
            .size(12)
            .color(style::MUTED),
        rule(),
        scrollable(form.padding([0, 12]))
            .style(style::scrollbar)
            .height(Length::Fill),
        row![
            super::view::pill("Refresh index", false, Message::Refresh),
            Space::new().width(Length::Fill),
            button(text(save_label).size(12).color(style::WHITE))
                .padding([7, 12])
                .style(|_, status| style::button(true, status))
                .on_press(if draft.saving {
                    Message::Noop
                } else {
                    Message::Save
                }),
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(16)
    .into()
}

fn hotkey_placeholder() -> &'static str {
    if hotkey::supported() {
        "Super+Space"
    } else {
        "Shortcut"
    }
}

fn label(value: &str) -> Element<'_, Message> {
    text(value).size(11).color(style::MUTED).into()
}

fn rule<'a>() -> Element<'a, Message> {
    container(Space::new().height(1))
        .width(Length::Fill)
        .style(style::rule)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_round_trip_and_drop_blanks() {
        let roots = parse_roots(" /tmp ;; /var/tmp ; ");
        assert_eq!(
            roots,
            vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")]
        );
        assert_eq!(join_roots(&roots), "/tmp; /var/tmp");
    }

    #[test]
    fn switching_provider_resets_endpoint_model_and_key() {
        let mut draft = Draft::new(&Config::default());
        draft.key = "secret".into();
        draft.apply(Edit::Provider(Provider::Ollama));
        assert_eq!(draft.config.base_url, Provider::Ollama.endpoint());
        assert!(draft.config.model.is_empty());
        assert!(draft.key.is_empty());
    }

    #[test]
    fn saved_config_trims_text_fields() {
        let mut draft = Draft::new(&Config::default());
        draft.apply(Edit::Model("  gpt  ".into()));
        draft.apply(Edit::Endpoint(" https://api.openai.com/v1/ ".into()));
        draft.apply(Edit::Hotkey(" Alt+Space ".into()));
        let config = draft.to_config();
        assert_eq!(config.model, "gpt");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.hotkey, "Alt+Space");
    }
}
