use crate::{
    ai,
    config::{Config, Provider},
    search::{self, ResultKind, SearchResult},
};
use clap::Parser;
use cosmic::app::{Core, CosmicFlags, Settings, Task};
use cosmic::dbus_activation::Details;
use cosmic::iced::keyboard::{Key, key::Named};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::SctkLayerSurfaceSettings;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, destroy_layer_surface,
};
use cosmic::iced::runtime::core::event::{PlatformSpecific, wayland};
use cosmic::iced::runtime::core::layout::Limits;
use cosmic::iced::runtime::platform_specific::wayland::CornerRadius;
use cosmic::iced::widget::{Space, button, column, container, row, scrollable, text};
use cosmic::iced::{self, Alignment, Border, Color, Length, Shadow, Subscription, Vector, window};
use cosmic::surface::action::{LiveSettings, app_layer_shell};
use cosmic::widget::{icon, text_input};
use cosmic::{Application, Element};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const PURPLE: Color = Color::from_rgb(0.71, 0.53, 1.0);
const WHITE: Color = Color::from_rgb(0.94, 0.93, 0.98);
const MUTED: Color = Color::from_rgb(0.57, 0.55, 0.65);
static RESULTS: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("spotlight-results"));
static INPUT: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("spotlight-search"));
static MODEL_INPUT: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("spotlight-model"));

#[derive(Parser, Clone, Debug)]
#[command(
    version,
    about = "A translucent launcher and read-only AI assistant for COSMIC"
)]
pub struct Args {
    #[arg(long, help = "Start in the background without opening the launcher")]
    daemon: bool,
    #[arg(long, help = "Toggle the launcher (the default action)")]
    toggle: bool,
    #[arg(long, help = "Open AI and search settings")]
    settings: bool,
    #[arg(long, help = "Open the launcher with a search query")]
    query: Option<String>,
    #[arg(skip)]
    action: Option<Command>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    Toggle,
    Settings,
    Query(String),
    Daemon,
}
impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self).unwrap())
    }
}
impl CosmicFlags for Args {
    type SubCommand = Command;
    type Args = Vec<String>;
    fn action(&self) -> Option<&Command> {
        self.action.as_ref()
    }
}

pub fn run() -> iced::Result {
    let mut args = Args::parse();
    args.action = Some(if args.settings {
        Command::Settings
    } else if let Some(q) = &args.query {
        Command::Query(q.clone())
    } else if args.daemon {
        Command::Daemon
    } else {
        Command::Toggle
    });
    cosmic::app::run_single_instance::<Spotlight>(
        Settings::default()
            .theme(cosmic::Theme::dark())
            .antialiasing(true)
            .default_text_size(15.0)
            .no_main_window(true)
            .exit_on_close(false),
        args,
    )
}

#[derive(Clone, Debug)]
pub enum Message {
    Input(String),
    SearchReady(u64, Vec<SearchResult>),
    AppsReady(u64, Arc<Vec<SearchResult>>),
    FilesReady(u64, Arc<Vec<PathBuf>>),
    Submit(u64),
    Activate(u64, usize),
    Activated(u64, Result<(), String>),
    Move(i32),
    Hide,
    FocusSurface(window::Id),
    HideSurface(window::Id),
    Focus,
    Tick,
    Settings,
    Back,
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
    Save,
    Saved(Result<Config, String>),
    Refresh,
    Copy,
    Retry,
    Noop,
}

pub struct Spotlight {
    core: Core,
    window: window::Id,
    visible: bool,
    query: String,
    results: Vec<SearchResult>,
    selected: usize,
    apps: Arc<Vec<SearchResult>>,
    files: Arc<Vec<PathBuf>>,
    indexing: bool,
    indexed_at: Instant,
    config: Config,
    draft: Config,
    settings: bool,
    key_input: String,
    roots_input: String,
    saving: bool,
    generation: u64,
    index_generation: u64,
    submitted: Option<String>,
    answer: String,
    status: String,
    error: Option<String>,
    answer_cancelled: bool,
    running: bool,
    cancel: CancellationToken,
    receiver: Option<mpsc::UnboundedReceiver<ai::Event>>,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    copied: bool,
}

impl Spotlight {
    fn show(&mut self) -> Task<Message> {
        if self.visible {
            return self.focus_current();
        }
        self.visible = true;
        self.window = window::Id::unique();
        cosmic::surface::surface_task(app_layer_shell(
            |_app: &Self| LiveSettings {
                padding: None,
                corners: Some(CornerRadius {
                    top_left: 24,
                    top_right: 24,
                    bottom_left: 24,
                    bottom_right: 24,
                }),
                blur: Some(true),
            },
            |app: &mut Self| SctkLayerSurfaceSettings {
                id: app.window,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                anchor: Anchor::empty(),
                namespace: "spotlight".into(),
                exclusive_zone: -1,
                size: Some((Some(740), Some(620))),
                size_limits: Limits::NONE
                    .min_width(360.)
                    .max_width(740.)
                    .min_height(300.)
                    .max_height(620.),
                ..Default::default()
            },
            None,
        ))
    }

    fn focus_current(&self) -> Task<Message> {
        if self.settings {
            text_input::focus(MODEL_INPUT.clone())
        } else {
            text_input::focus(INPUT.clone())
        }
    }

    fn cancel_ai(&mut self) {
        self.cancel.cancel();
        self.running = false;
        self.receiver = None;
    }

    fn hide(&mut self) -> Task<Message> {
        self.cancel_ai();
        self.generation += 1;
        self.visible = false;
        self.key_input.clear();
        destroy_layer_surface(self.window)
    }

    fn index(&mut self) -> Task<Message> {
        self.indexing = true;
        self.index_generation += 1;
        self.generation += 1;
        self.results.clear();
        self.selected = 0;
        let request = self.index_generation;
        let roots = self.config.search_roots.clone();
        let apps = Task::perform(
            async move {
                tokio::task::spawn_blocking(search::discover_apps)
                    .await
                    .unwrap_or_default()
            },
            move |apps| cosmic::Action::App(Message::AppsReady(request, Arc::new(apps))),
        );
        let files = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || search::build_file_index(&roots))
                    .await
                    .unwrap_or_default()
            },
            move |files| cosmic::Action::App(Message::FilesReady(request, Arc::new(files))),
        );
        Task::batch([apps, files])
    }

    fn query_task(&self, generation: u64, include_files: bool) -> Task<Message> {
        let query = self.query.clone();
        let apps = self.apps.clone();
        let files = if include_files {
            self.files.clone()
        } else {
            Arc::default()
        };
        let engine = self.config.web_engine.clone();
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(60)).await;
                tokio::task::spawn_blocking(move || search::query(&query, &apps, &files, &engine))
                    .await
                    .unwrap_or_default()
            },
            move |results| cosmic::Action::App(Message::SearchReady(generation, results)),
        )
    }

    fn changed(&mut self, value: String) -> Task<Message> {
        if self.query == value {
            return Task::none();
        }
        self.cancel_ai();
        self.query = value;
        self.generation += 1;
        self.selected = 0;
        self.submitted = None;
        self.answer.clear();
        self.error = None;
        self.elapsed = None;
        self.copied = false;
        let generation = self.generation;
        if question_prompt(&self.query).is_some() {
            self.results.clear();
            self.status = "Ready to ask — keep typing to edit".into();
            let delay = self.config.debounce_ms;
            return Task::perform(
                async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    generation
                },
                |g| cosmic::Action::App(Message::Submit(g)),
            );
        }
        self.status.clear();
        self.results.clear();
        self.query_task(generation, !self.indexing)
    }

    fn submit(&mut self) -> Task<Message> {
        let Some(prompt) = question_prompt(&self.query) else {
            return Task::none();
        };
        if self.submitted.as_deref() == Some(&prompt) {
            return Task::none();
        }
        self.cancel_ai();
        self.submitted = Some(prompt.clone());
        self.answer.clear();
        self.error = None;
        self.elapsed = None;
        self.started = Some(Instant::now());
        self.running = true;
        self.status = "Connecting to your model…".into();
        self.cancel = CancellationToken::new();
        let cancel = self.cancel.clone();
        let config = self.config.clone();
        let files = self.files.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        self.receiver = Some(rx);
        Task::perform(
            async move {
                ai::run(prompt, config, files, tx, cancel).await;
            },
            |_| cosmic::Action::App(Message::Noop),
        )
    }

    fn pill<'a>(&self, label: &'a str, active: bool, message: Message) -> Element<'a, Message> {
        button(
            text(label)
                .size(12)
                .class(if active { PURPLE } else { MUTED }),
        )
        .padding([7, 12])
        .class(cosmic::theme::iced::Button::Custom(Box::new(
            move |_, status| button_style(active, status),
        )))
        .on_press(message)
        .into()
    }

    fn search_view(&self) -> Element<'_, Message> {
        let ai_mode = question_prompt(&self.query).is_some();
        let search = text_input::text_input("Search anything…", &self.query)
            .id(INPUT.clone())
            .size(24)
            .padding([16, 0])
            .on_input(Message::Input)
            .on_paste(Message::Input)
            .on_submit(|_| Message::Submit(u64::MAX))
            .style(search_style());
        let search_row = row![
            icon::from_name(if ai_mode {
                "chat-message-new-symbolic"
            } else {
                "system-search-symbolic"
            })
            .size(25),
            search,
            container(
                text(if ai_mode { "ASK AI" } else { "SEARCH" })
                    .size(10)
                    .class(PURPLE)
            )
            .padding([5, 8])
            .class(cosmic::theme::Container::Custom(Box::new(|_| chip_style())))
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let top = row![
            text("S P O T L I G H T").size(11).class(PURPLE),
            Space::new().width(Length::Fill),
            self.pill("Settings", false, Message::Settings),
            self.pill("esc", false, Message::Hide)
        ]
        .align_y(Alignment::Center)
        .spacing(4);
        let mut content = column![top, search_row].spacing(7);
        content = content.push(rule());
        if ai_mode {
            let model = row![
                text("✦  AI ANSWER").size(11).class(PURPLE),
                Space::new().width(Length::Fill),
                text(format!("{} · {}", self.config.provider, self.config.model))
                    .size(11)
                    .class(MUTED)
            ]
            .align_y(Alignment::Center);
            let mut body = column![model].spacing(18);
            if self.answer.is_empty() {
                body = body.push(
                    text(if self.running {
                        "Thinking…"
                    } else if self.error.is_some() {
                        "Couldn’t finish this answer"
                    } else {
                        "Your answer will appear here"
                    })
                    .size(23)
                    .class(WHITE),
                );
                body = body.push(text(&self.status).size(13).class(MUTED));
            } else {
                body = body.push(text(&self.answer).size(16).class(WHITE).width(Length::Fill));
                if self.running {
                    body = body.push(text(&self.status).size(11).class(PURPLE));
                }
            }
            if let Some(error) = &self.error {
                body = body.push(text(error).size(13).class(Color::from_rgb(1., 0.66, 0.73)));
                body = body.push(
                    row![
                        self.pill("Retry", true, Message::Retry),
                        self.pill("AI settings", false, Message::Settings)
                    ]
                    .spacing(8),
                );
            }
            if !self.answer.is_empty() {
                let elapsed = self
                    .elapsed
                    .map(|d| format!("{:.1}s", d.as_secs_f32()))
                    .unwrap_or_default();
                body = body.push(
                    row![
                        self.pill(
                            if self.copied { "Copied" } else { "Copy answer" },
                            true,
                            Message::Copy
                        ),
                        text(elapsed).size(11).class(MUTED)
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center),
                );
            }
            content = content.push(scrollable(body.padding([16, 0])).height(Length::Fill));
        } else {
            content = content.push(
                row![
                    text(if self.query.is_empty() {
                        "APPLICATIONS"
                    } else {
                        "BEST MATCHES"
                    })
                    .size(10)
                    .class(MUTED),
                    Space::new().width(Length::Fill),
                    text(if self.indexing {
                        "Indexing your files…".into()
                    } else {
                        format!("{} apps · {} files", self.apps.len(), self.files.len())
                    })
                    .size(10)
                    .class(MUTED)
                ]
                .padding([9, 0]),
            );
            let mut items = column![].spacing(4);
            for (i, result) in self.results.iter().take(12).enumerate() {
                let selected = i == self.selected;
                let (fallback, category) = match &result.kind {
                    ResultKind::App { .. } => ("application-x-executable-symbolic", "APP"),
                    ResultKind::File { .. } => ("text-x-generic-symbolic", "FILE"),
                    ResultKind::Web { .. } => ("web-browser-symbolic", "WEB"),
                };
                let result_icon =
                    icon::from_name(result.icon.as_deref().unwrap_or(fallback)).size(28);
                let title = text(&result.title).size(15).class(WHITE);
                let subtitle = text(shorten(&result.subtitle, 74)).size(11).class(MUTED);
                let entry = row![
                    container(result_icon).center_x(44),
                    column![title, subtitle].spacing(3).width(Length::Fill),
                    text(category)
                        .size(9)
                        .class(if selected { PURPLE } else { MUTED }),
                    text(if selected { "↵" } else { " " })
                        .size(17)
                        .class(PURPLE)
                ]
                .align_y(Alignment::Center)
                .spacing(10);
                items = items.push(
                    button(entry)
                        .width(Length::Fill)
                        .padding([10, 10])
                        .on_press(Message::Activate(self.generation, i))
                        .class(cosmic::theme::iced::Button::Custom(Box::new(
                            move |_, status| button_style(selected, status),
                        ))),
                );
            }
            if self.results.is_empty() {
                items = items.push(
                    container(
                        column![
                            text(if self.indexing {
                                "Getting everything ready"
                            } else {
                                "No matches yet"
                            })
                            .size(22)
                            .class(WHITE),
                            text("Try an app name, a filename, or a web search.")
                                .size(13)
                                .class(MUTED)
                        ]
                        .spacing(12),
                    )
                    .padding([30, 12]),
                );
            }
            content = content.push(scrollable(items).id(RESULTS.clone()).height(Length::Fill));
            if let Some(error) = &self.error {
                content = content.push(text(error).size(12).class(PURPLE));
            }
        }
        content = content.push(rule());
        let footer = if ai_mode {
            row![
                text("Read-only tools").size(11).class(MUTED),
                Space::new().width(Length::Fill),
                text(if self.running {
                    "esc to cancel & close"
                } else {
                    "Edit your question to ask again"
                })
                .size(11)
                .class(MUTED)
            ]
        } else {
            row![
                text("↑ ↓ navigate    ↵ open").size(11).class(MUTED),
                Space::new().width(Length::Fill),
                text("End with  ?  to ask AI").size(11).class(PURPLE)
            ]
        };
        content.push(footer.padding([10, 0])).into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let header = row![
            text("Make it yours").size(25).class(WHITE),
            Space::new().width(Length::Fill),
            self.pill("Back", false, Message::Back)
        ]
        .align_y(Alignment::Center);
        let providers = Provider::ALL.into_iter().fold(row![].spacing(4), |r, p| {
            r.push(self.pill(
                match p {
                    Provider::OpenCodeGo => "Go",
                    Provider::OpenAI => "OpenAI",
                    Provider::Anthropic => "Claude",
                    Provider::Gemini => "Gemini",
                    Provider::OpenRouter => "OpenRouter",
                    Provider::Ollama => "Ollama",
                    Provider::Custom => "Custom",
                },
                self.draft.provider == p,
                Message::Provider(p),
            ))
        });
        let model = text_input::text_input("Model ID", &self.draft.model)
            .id(MODEL_INPUT.clone())
            .on_input(Message::Model)
            .style(search_style())
            .padding(10);
        let endpoint = text_input::text_input("https://provider.example/v1", &self.draft.base_url)
            .on_input(Message::Endpoint)
            .style(search_style())
            .padding(10);
        let key = text_input::secure_input(
            "Paste a new key to save in your desktop keyring",
            &self.key_input,
            Some(Message::Noop),
            true,
        )
        .on_input(Message::ApiKey)
        .style(search_style())
        .padding(10);
        let roots = text_input::text_input(
            "Absolute folders, separated by semicolons",
            &self.roots_input,
        )
        .on_input(Message::Roots)
        .style(search_style())
        .padding(10);
        let tools = row![
            self.pill(
                if self.draft.web_tools {
                    "✓ Web tools"
                } else {
                    "Web tools"
                },
                self.draft.web_tools,
                Message::WebTools(!self.draft.web_tools)
            ),
            self.pill(
                if self.draft.local_tools {
                    "✓ Local file tools"
                } else {
                    "Local file tools"
                },
                self.draft.local_tools,
                Message::LocalTools(!self.draft.local_tools)
            )
        ]
        .spacing(8);
        let engine = row![
            self.pill(
                "DuckDuckGo",
                self.draft.web_engine == "duckduckgo",
                Message::WebEngine("duckduckgo".into())
            ),
            self.pill(
                "Google",
                self.draft.web_engine == "google",
                Message::WebEngine("google".into())
            ),
            self.pill(
                "Brave",
                self.draft.web_engine == "brave",
                Message::WebEngine("brave".into())
            )
        ]
        .spacing(6);
        let delay = row![
            text("Ask after").size(12).class(MUTED),
            self.pill("400 ms", self.draft.debounce_ms == 400, Message::Delay(400)),
            self.pill("650 ms", self.draft.debounce_ms == 650, Message::Delay(650)),
            self.pill(
                "1 second",
                self.draft.debounce_ms == 1000,
                Message::Delay(1000)
            )
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let opacity = row![
            text("Glass opacity").size(12).class(MUTED),
            self.pill("75%", self.draft.opacity == 0.75, Message::Opacity(0.75)),
            self.pill("88%", self.draft.opacity == 0.88, Message::Opacity(0.88)),
            self.pill("100%", self.draft.opacity == 1., Message::Opacity(1.))
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let mut form = column![text("AI PROVIDER").size(10).class(PURPLE), providers, row![column![label("Model"), model].width(Length::Fill), column![label("API base URL"), endpoint].width(Length::Fill)].spacing(12), label("API key · saved securely, never in your config file"), key, tools, text("Web tools send searches and fetch public pages. Local tools can send text from your search folders to your chosen AI provider. Hidden files and credential files are excluded.").size(11).class(MUTED).width(Length::Fill), rule(), label("Search folders"), roots, label("Web search engine"), engine, delay, opacity].spacing(12).width(Length::Fill);
        if let Some(error) = &self.error {
            form = form.push(text(error).size(12).class(Color::from_rgb(1., 0.66, 0.73)));
        }
        column![
            header,
            text("A small launcher. Your providers. Your preferences.")
                .size(12)
                .class(MUTED),
            rule(),
            scrollable(form).height(Length::Fill),
            row![
                self.pill("Refresh index", false, Message::Refresh),
                Space::new().width(Length::Fill),
                self.pill(
                    if self.saving {
                        "Saving…"
                    } else {
                        "Save settings"
                    },
                    true,
                    if self.saving {
                        Message::Noop
                    } else {
                        Message::Save
                    }
                )
            ]
            .align_y(Alignment::Center)
        ]
        .spacing(16)
        .into()
    }
}

impl Application for Spotlight {
    type Executor = cosmic::executor::Default;
    type Flags = Args;
    type Message = Message;
    const APP_ID: &'static str = crate::config::APP_ID;
    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }
    fn init(mut core: Core, flags: Args) -> (Self, Task<Message>) {
        core.set_app_type(cosmic::core::AppType::System);
        core.set_keyboard_nav(false);
        let loaded = Config::load();
        let error = loaded.as_ref().err().cloned();
        let config = loaded.unwrap_or_default();
        let roots_input = config
            .search_roots
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("; ");
        let mut app = Self {
            core,
            window: window::Id::unique(),
            visible: false,
            query: String::new(),
            results: vec![],
            selected: 0,
            apps: Arc::default(),
            files: Arc::default(),
            indexing: true,
            indexed_at: Instant::now(),
            draft: config.clone(),
            config,
            settings: flags.settings,
            key_input: String::new(),
            roots_input,
            saving: false,
            generation: 0,
            index_generation: 0,
            submitted: None,
            answer: String::new(),
            status: String::new(),
            error,
            answer_cancelled: false,
            running: false,
            cancel: CancellationToken::new(),
            receiver: None,
            started: None,
            elapsed: None,
            copied: false,
        };
        let index = app.index();
        let show = if flags.daemon {
            Task::none()
        } else {
            app.show()
        };
        let query = if let Some(q) = flags.query {
            app.changed(q)
        } else {
            Task::none()
        };
        (app, Task::batch([index, show, query]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Input(value) => return self.changed(value),
            Message::SearchReady(g, results)
                if is_current(g, self.generation) && question_prompt(&self.query).is_none() =>
            {
                self.results = results;
                self.selected = self.selected.min(self.results.len().saturating_sub(1));
            }
            Message::AppsReady(request, apps) if is_current(request, self.index_generation) => {
                self.apps = apps;
                if question_prompt(&self.query).is_none() {
                    return self.query_task(self.generation, !self.indexing);
                }
            }
            Message::FilesReady(request, files) if is_current(request, self.index_generation) => {
                self.files = files;
                self.indexing = false;
                self.indexed_at = Instant::now();
                if question_prompt(&self.query).is_none() {
                    return self.query_task(self.generation, true);
                }
            }
            Message::Submit(g)
                if self.visible && !self.settings && (g == self.generation || g == u64::MAX) =>
            {
                if question_prompt(&self.query).is_some() {
                    return self.submit();
                }
                return self.update(Message::Activate(self.generation, self.selected));
            }
            Message::Activate(g, i)
                if g == self.generation
                    && self.visible
                    && !self.settings
                    && question_prompt(&self.query).is_none() =>
            {
                if let Some(result) = self.results.get(i).cloned() {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || search::activate(&result))
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()))
                        },
                        move |r| cosmic::Action::App(Message::Activated(g, r)),
                    );
                }
            }
            Message::Activated(g, Ok(())) if g == self.generation && self.visible => {
                return self.hide();
            }
            Message::Activated(g, Err(error)) if g == self.generation && self.visible => {
                self.error = Some(error)
            }
            Message::Move(direction) if !self.settings && !self.results.is_empty() => {
                self.selected = (self.selected as i32 + direction)
                    .rem_euclid(self.results.len() as i32) as usize;
                let y = self.selected as f32 / self.results.len().saturating_sub(1).max(1) as f32;
                return iced::widget::operation::snap_to(
                    RESULTS.clone(),
                    iced::widget::scrollable::RelativeOffset { x: 0., y },
                );
            }
            Message::Hide => return self.hide(),
            Message::Focus if self.visible => return self.focus_current(),
            Message::FocusSurface(surface) if self.visible && surface == self.window => {
                return self.focus_current();
            }
            Message::HideSurface(surface) if self.visible && surface == self.window => {
                return self.hide();
            }
            Message::Settings => {
                if self.running {
                    self.cancel_ai();
                    self.submitted = None;
                    self.answer.clear();
                    self.elapsed = None;
                    self.answer_cancelled = true;
                }
                self.generation += 1;
                self.settings = true;
                self.draft = self.config.clone();
                self.key_input.clear();
                self.error = None;
                self.roots_input = self
                    .draft
                    .search_roots
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("; ");
                return self.focus_current();
            }
            Message::Back => {
                self.settings = false;
                self.key_input.clear();
                self.error = if self.answer_cancelled {
                    Some("Answer canceled. Retry to ask again.".into())
                } else {
                    None
                };
                self.answer_cancelled = false;
                return self.focus_current();
            }
            Message::Provider(provider) => {
                self.draft.provider = provider;
                self.draft.base_url = provider.endpoint().into();
                self.draft.model = if provider == Provider::OpenCodeGo {
                    "deepseek-v4.1-flash".into()
                } else {
                    String::new()
                };
                self.key_input.clear();
            }
            Message::Model(value) => self.draft.model = value,
            Message::Endpoint(value) => self.draft.base_url = value,
            Message::ApiKey(value) => self.key_input = value,
            Message::Roots(value) => self.roots_input = value,
            Message::WebTools(value) => self.draft.web_tools = value,
            Message::LocalTools(value) => self.draft.local_tools = value,
            Message::WebEngine(value) => self.draft.web_engine = value,
            Message::Delay(value) => self.draft.debounce_ms = value,
            Message::Opacity(value) => self.draft.opacity = value,
            Message::Save if !self.saving => {
                let mut config = self.draft.clone();
                config.model = config.model.trim().into();
                config.base_url = config.base_url.trim().trim_end_matches('/').into();
                config.search_roots = self
                    .roots_input
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect();
                if let Err(error) = config.validate() {
                    self.error = Some(error);
                    return Task::none();
                }
                let key = std::mem::take(&mut self.key_input);
                self.saving = true;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            config.store_key(&key)?;
                            config.save()?;
                            Ok(config)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| cosmic::Action::App(Message::Saved(r)),
                );
            }
            Message::Saved(result) => {
                self.saving = false;
                match result {
                    Ok(config) => {
                        self.config = config;
                        self.settings = false;
                        self.error = None;
                        self.submitted = None;
                        self.answer_cancelled = false;
                        let index = self.index();
                        let focus = self.focus_current();
                        return Task::batch([index, focus]);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Message::Refresh if !self.indexing => return self.index(),
            Message::Retry => {
                self.submitted = None;
                return self.submit();
            }
            Message::Copy => {
                self.copied = true;
                return iced::clipboard::write(self.answer.clone());
            }
            Message::Tick => {
                let events: Vec<_> = if let Some(rx) = &mut self.receiver {
                    std::iter::from_fn(|| rx.try_recv().ok())
                        .take(256)
                        .collect()
                } else {
                    vec![]
                };
                for event in events {
                    match event {
                        ai::Event::Status(s) => self.status = s,
                        ai::Event::Delta(s) => self.answer.push_str(&s),
                        ai::Event::Done => {
                            self.running = false;
                            self.elapsed = self.started.map(|s| s.elapsed());
                            self.receiver = None;
                        }
                        ai::Event::Error(s) => {
                            self.running = false;
                            self.error = Some(s);
                            self.status.clear();
                            self.elapsed = self.started.map(|s| s.elapsed());
                            self.receiver = None;
                        }
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn dbus_activation(&mut self, message: cosmic::dbus_activation::Message) -> Task<Message> {
        let command = match message.msg {
            Details::ActivateAction { action, .. } => {
                serde_json::from_str(&action).unwrap_or(Command::Toggle)
            }
            Details::Activate => Command::Toggle,
            _ => return Task::none(),
        };
        match command {
            Command::Daemon => Task::none(),
            Command::Toggle if self.visible => self.hide(),
            Command::Toggle => {
                self.settings = false;
                self.query.clear();
                self.results = search::query("", &self.apps, &self.files, &self.config.web_engine);
                self.selected = 0;
                self.answer.clear();
                self.submitted = None;
                self.error = None;
                let show = self.show();
                if self.indexed_at.elapsed() > Duration::from_secs(300) && !self.indexing {
                    Task::batch([show, self.index()])
                } else {
                    show
                }
            }
            Command::Settings => {
                let show = self.show();
                Task::batch([show, self.update(Message::Settings)])
            }
            Command::Query(query) => {
                self.settings = false;
                let show = self.show();
                Task::batch([show, self.changed(query)])
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        self.view_window(self.window)
    }
    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let content = if self.settings {
            self.settings_view()
        } else {
            self.search_view()
        };
        let opacity = self.config.opacity;
        container(content)
            .padding([20, 24])
            .width(Length::Fill)
            .height(Length::Fill)
            .class(cosmic::theme::Container::Custom(Box::new(move |_| {
                container::Style {
                    text_color: Some(WHITE),
                    background: Some(Color::from_rgba(0.065, 0.052, 0.095, opacity).into()),
                    border: Border {
                        color: Color::from_rgba(0.68, 0.53, 0.93, 0.27),
                        width: 1.,
                        radius: 24.0.into(),
                    },
                    shadow: Shadow {
                        color: Color::from_rgba(0., 0., 0., 0.5),
                        offset: Vector::new(0., 14.),
                        blur_radius: 40.,
                    },
                    ..Default::default()
                }
            })))
            .into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: WHITE,
            icon_color: PURPLE,
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys =
            iced::event::listen_raw(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: Key::Named(Named::Escape),
                    ..
                }) => Some(Message::Hide),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: Key::Named(Named::ArrowDown),
                    ..
                }) => Some(Message::Move(1)),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: Key::Named(Named::ArrowUp),
                    ..
                }) => Some(Message::Move(-1)),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: Key::Character(c),
                    modifiers,
                    ..
                }) if modifiers.control() && c == "," => Some(Message::Settings),
                iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Layer(wayland::LayerEvent::Focused, _, surface),
                )) => Some(Message::FocusSurface(surface)),
                iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Layer(wayland::LayerEvent::Unfocused, _, surface),
                )) => Some(Message::HideSurface(surface)),
                iced::Event::Window(window::Event::Opened { .. }) => Some(Message::Focus),
                _ => None,
            });
        if self.running {
            Subscription::batch([
                keys,
                iced::time::every(Duration::from_millis(32)).map(|_| Message::Tick),
            ])
        } else {
            keys
        }
    }
}

fn question_prompt(query: &str) -> Option<String> {
    let query = query.trim();
    query
        .strip_suffix('?')
        .filter(|s| !s.trim().is_empty())
        .map(|_| query.to_string())
}
fn is_current(request: u64, current: u64) -> bool {
    request == current
}
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
fn label(value: &str) -> Element<'_, Message> {
    text(value).size(11).class(MUTED).into()
}
fn rule<'a>() -> Element<'a, Message> {
    container(Space::new().height(1))
        .width(Length::Fill)
        .class(cosmic::theme::Container::Custom(Box::new(|_| {
            container::Style {
                background: Some(Color::from_rgba(0.65, 0.55, 0.8, 0.14).into()),
                ..Default::default()
            }
        })))
        .into()
}
fn chip_style() -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(0.52, 0.31, 0.91, 0.16).into()),
        border: Border {
            radius: 5.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn button_style(active: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: if active || hovered {
            Some(Color::from_rgba(0.49, 0.31, 0.78, if active { 0.24 } else { 0.12 }).into())
        } else {
            None
        },
        text_color: WHITE,
        border: Border {
            radius: 11.0.into(),
            width: if active { 1. } else { 0. },
            color: Color::from_rgba(0.62, 0.44, 0.9, 0.28),
        },
        ..Default::default()
    }
}
fn search_style() -> cosmic::theme::TextInput {
    fn appearance(_: &cosmic::Theme) -> text_input::Appearance {
        text_input::Appearance {
            background: Color::TRANSPARENT.into(),
            border_radius: 8.0.into(),
            border_offset: None,
            border_width: 0.,
            border_color: Color::TRANSPARENT,
            label_color: MUTED,
            placeholder_color: MUTED,
            selected_text_color: WHITE,
            icon_color: Some(PURPLE),
            text_color: Some(WHITE),
            selected_fill: Color::from_rgba(0.5, 0.3, 0.8, 0.5),
        }
    }
    cosmic::theme::TextInput::Custom {
        active: Box::new(appearance),
        error: Box::new(appearance),
        hovered: Box::new(appearance),
        focused: Box::new(appearance),
        disabled: Box::new(appearance),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn question_suffix_requires_a_real_prompt() {
        assert_eq!(
            question_prompt("what time is it?  "),
            Some("what time is it?".into())
        );
        for query in ["", "?", "   ?", "app", "why? actually"] {
            assert!(question_prompt(query).is_none());
        }
    }

    #[test]
    fn older_async_results_are_rejected_after_a_new_generation_starts() {
        assert!(is_current(4, 4));
        assert!(!is_current(3, 4));
    }
}
