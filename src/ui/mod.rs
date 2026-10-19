//! The launcher. One iced daemon owns the application index, the AI session and
//! the settings draft; its window exists only while the launcher is shown.
//! Behaviour (keys, debounced questions, stale-result guards, hide on focus
//! loss) follows the earlier COSMIC layer-surface UI.

pub mod icons;
pub mod settings;
pub mod style;
pub mod view;
#[cfg(target_os = "linux")]
mod wayland;

#[cfg(windows)]
use crate::platform;
use crate::{
    autostart,
    hotkey::{self, Hotkey},
};
use iced::{
    Element, Size, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    widget::{Id, operation, scrollable},
    window,
};
use icons::IconMap;
use settings::{Draft, Edit};
use spotlight::{
    ai,
    config::Config,
    ipc, launch,
    search::{self, SearchResult},
};
use std::{
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_util::sync::CancellationToken;

/// Logical size of the panel. The window is not resizable.
pub const WIDTH: f32 = 740.0;
pub const HEIGHT: f32 = 620.0;
/// The settings form is taller than the search panel and scrolls within the
/// space between its header and the Save footer.
pub const SETTINGS_HEIGHT: f32 = 1000.0;
/// Results shown at once; the list scrolls to the selection beyond these.
pub const VISIBLE_RESULTS: usize = 12;
/// Short pause after each keystroke so fast typing does not search per key.
const SEARCH_DELAY: Duration = Duration::from_millis(60);
/// Rebuild the app and file index this often, and when shown after this long.
const INDEX_MAX_AGE: Duration = Duration::from_secs(300);
const NOTICE_CANCELED: &str = "Answer canceled. Retry to ask again.";

static INPUT: LazyLock<Id> = LazyLock::new(|| Id::new("spotlight-search"));
pub static RESULTS: LazyLock<Id> = LazyLock::new(|| Id::new("spotlight-results"));
pub static MODEL_INPUT: LazyLock<Id> = LazyLock::new(|| Id::new("spotlight-model"));

// Handed from `main` to the iced boot function and the IPC subscription. iced
// boot and subscription builders are plain functions, so they read these once.
static INITIAL: Mutex<Option<ipc::Command>> = Mutex::new(None);
static IPC: Mutex<Option<UnboundedReceiver<ipc::Command>>> = Mutex::new(None);

/// How the launcher's surfaces are created. Chosen once in [`run`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum Backend {
    /// Ordinary winit windows (Windows, macOS, Linux on X11 or non-layer-shell Wayland).
    Winit,
    /// wlr-layer-shell overlay surfaces. Wayland ignores requested window
    /// positions, but a layer surface with no anchors is centered by the
    /// compositor. `max_height` is the largest surface height for this output.
    #[cfg(target_os = "linux")]
    LayerShell { max_height: f32 },
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

fn backend() -> Backend {
    BACKEND.get().copied().unwrap_or(Backend::Winit)
}

/// Start the daemon. `initial` is the command the launching invocation asked
/// for (`None` for `--daemon`, which starts hidden).
pub fn run(
    initial: Option<ipc::Command>,
    commands: UnboundedReceiver<ipc::Command>,
) -> Result<(), String> {
    *lock(&INITIAL) = initial;
    *lock(&IPC) = Some(commands);
    #[cfg(target_os = "linux")]
    if let Some(max_height) = wayland::probe() {
        let _ = BACKEND.set(Backend::LayerShell { max_height });
        return wayland::run();
    }
    iced::daemon(Spotlight::boot, Spotlight::update, Spotlight::view)
        .title(|_: &Spotlight, _: window::Id| "Spotlight".to_string())
        .theme(|_: &Spotlight, _: window::Id| style::theme())
        .style(|_: &Spotlight, theme: &iced::Theme| style::application(theme))
        .subscription(Spotlight::subscription)
        .antialiasing(true)
        .run()
        .map_err(|error| error.to_string())
}

fn lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Search,
    Settings,
}

// On Linux the layer-shell backend turns some variants into compositor
// requests (see `Backend::LayerShell`), so this adds the macro's variants there.
#[cfg_attr(target_os = "linux", iced_layershell::to_layer_message(multi))]
#[derive(Clone, Debug)]
pub enum Message {
    Command(ipc::Command),
    HotkeyPressed,
    WindowOpened(window::Id),
    WindowPrepared(bool),
    CloseRequested(window::Id),
    Unfocused(window::Id),
    Escape,
    OpenSettings,
    Back,
    Move(i32),
    Input(String),
    Submit,
    AutoAsk(u64),
    Activate(usize),
    Activated(u64, Result<(), String>),
    AppsReady(u64, Arc<Vec<SearchResult>>),
    FilesReady(u64, Arc<Vec<PathBuf>>),
    IconsReady(Arc<IconMap>),
    SearchReady(u64, Vec<SearchResult>),
    Ai(u64, ai::Event),
    Retry,
    Copy,
    Refresh,
    Field(Edit),
    Save,
    Saved(Result<Config, String>),
    Noop,
}

/// The answer to the current question, streamed from the AI harness.
#[derive(Default)]
struct Answer {
    prompt: Option<String>,
    text: String,
    status: String,
    error: Option<String>,
    running: bool,
    copied: bool,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    cancel: Option<CancellationToken>,
    handle: Option<iced::task::Handle>,
}

pub struct Spotlight {
    window: Option<window::Id>,
    backdrop: bool,
    page: Page,
    config: Config,
    draft: Draft,
    hotkey: Hotkey,
    query: String,
    results: Vec<SearchResult>,
    selected: usize,
    apps: Arc<Vec<SearchResult>>,
    files: Arc<Vec<PathBuf>>,
    icons: Arc<IconMap>,
    indexing: bool,
    index_request: u64,
    indexed_at: Instant,
    /// Bumped whenever the visible state changes; late results carry the old value.
    generation: u64,
    answer: Answer,
    answer_canceled: bool,
    error: Option<String>,
}

impl Spotlight {
    fn boot() -> (Self, Task<Message>) {
        let (config, load_error) = match Config::load() {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(error)),
        };
        let mut hotkey = Hotkey::new();
        let hotkey_error = hotkey.register(&config.hotkey).err();
        let mut app = Self {
            window: None,
            backdrop: false,
            page: Page::Search,
            draft: Draft::new(&config),
            config,
            hotkey,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            apps: Arc::default(),
            files: Arc::default(),
            icons: Arc::default(),
            indexing: false,
            index_request: 0,
            indexed_at: Instant::now(),
            generation: 0,
            answer: Answer::default(),
            answer_canceled: false,
            error: load_error.or(hotkey_error),
        };
        let mut tasks = vec![app.index()];
        if let Some(command) = lock(&INITIAL).take() {
            tasks.push(Task::done(Message::Command(command)));
        }
        (app, Task::batch(tasks))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(ipc_commands).map(Message::Command),
            Subscription::run(hotkey::presses).map(|()| Message::HotkeyPressed),
            event::listen_with(keyboard_event),
            window::close_requests().map(Message::CloseRequested),
            iced::time::every(INDEX_MAX_AGE).map(|_| Message::Refresh),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Command(command) => self.command(command),
            Message::HotkeyPressed => self.toggle(),
            Message::WindowOpened(id) if self.window == Some(id) => {
                Task::batch([self.prepare(id), self.focus()])
            }
            Message::WindowOpened(_) => Task::none(),
            Message::WindowPrepared(backdrop) => {
                self.backdrop = backdrop;
                Task::none()
            }
            Message::CloseRequested(id) if self.window == Some(id) => self.hide(),
            Message::CloseRequested(id) => window::close(id),
            Message::Unfocused(id) if self.window == Some(id) => self.hide(),
            Message::Unfocused(_) => Task::none(),
            Message::Escape if self.page == Page::Settings => self.back(),
            Message::Escape => self.hide(),
            Message::OpenSettings => {
                let resize = self.resize_for(Page::Settings);
                Task::batch([resize, self.open_settings()])
            }
            Message::Back => self.back(),
            Message::Move(direction) => {
                if self.page == Page::Search && !self.results.is_empty() {
                    self.selected = step(self.selected, self.results.len(), direction);
                    return operation::snap_to(
                        RESULTS.clone(),
                        scrollable::RelativeOffset {
                            x: None,
                            y: Some(scroll_fraction(self.selected, self.results.len())),
                        },
                    );
                }
                Task::none()
            }
            Message::Input(value) => self.input(value),
            Message::Submit => match question_prompt(&self.query) {
                Some(prompt) if self.answer.prompt.as_deref() != Some(&prompt) => self.ask(prompt),
                Some(_) => Task::none(),
                None => self.activate(self.selected),
            },
            Message::AutoAsk(generation) if generation == self.generation => {
                match question_prompt(&self.query) {
                    Some(prompt) if self.answer.prompt.as_deref() != Some(&prompt) => {
                        self.ask(prompt)
                    }
                    _ => Task::none(),
                }
            }
            Message::AutoAsk(_) => Task::none(),
            Message::Activate(index) => self.activate(index),
            Message::Activated(generation, Ok(())) if generation == self.generation => self.hide(),
            Message::Activated(generation, Err(error)) if generation == self.generation => {
                self.error = Some(error);
                Task::none()
            }
            Message::Activated(_, _) => Task::none(),
            Message::AppsReady(request, apps) if request == self.index_request => {
                let names: Vec<String> = apps.iter().filter_map(|a| a.icon.clone()).collect();
                self.apps = apps;
                Task::batch([
                    Task::perform(
                        blocking(move || icons::resolve(names.iter().map(String::as_str))),
                        |map| Message::IconsReady(Arc::new(map)),
                    ),
                    self.search_if_plain(),
                ])
            }
            Message::FilesReady(request, files) if request == self.index_request => {
                self.files = files;
                self.indexing = false;
                self.indexed_at = Instant::now();
                self.search_if_plain()
            }
            Message::AppsReady(..) | Message::FilesReady(..) => Task::none(),
            Message::IconsReady(map) => {
                self.icons = map;
                Task::none()
            }
            Message::SearchReady(generation, results)
                if generation == self.generation && question_prompt(&self.query).is_none() =>
            {
                self.results = results;
                self.selected = self.selected.min(self.results.len().saturating_sub(1));
                Task::none()
            }
            Message::SearchReady(..) => Task::none(),
            Message::Ai(generation, event) if generation == self.generation => {
                self.answer_event(event);
                Task::none()
            }
            Message::Ai(..) => Task::none(),
            Message::Retry => match question_prompt(&self.query) {
                Some(prompt) => self.ask(prompt),
                None => Task::none(),
            },
            Message::Copy => {
                self.answer.copied = true;
                iced::clipboard::write(self.answer.text.clone())
            }
            Message::Refresh if !self.indexing => self.index(),
            Message::Refresh => Task::none(),
            Message::Field(edit) => {
                self.draft.apply(edit);
                Task::none()
            }
            Message::Save if !self.draft.saving => self.save(),
            Message::Save => Task::none(),
            Message::Saved(Ok(config)) => {
                self.draft.saving = false;
                self.config = config;
                self.page = Page::Search;
                self.draft = Draft::new(&self.config);
                self.error = None;
                self.answer_canceled = false;
                Task::batch([self.index(), self.focus()])
            }
            Message::Saved(Err(error)) => {
                self.draft.saving = false;
                self.draft.error = Some(error);
                // The keyring or disk write failed, so keep the last saved shortcut.
                if let Err(error) = self.hotkey.register(&self.config.hotkey) {
                    tracing::warn!("{error}");
                }
                Task::none()
            }
            Message::Noop => Task::none(),
            // Layer-shell requests are converted before `update` sees them, so
            // these variants never arrive here on Linux.
            #[cfg(target_os = "linux")]
            _ => Task::none(),
        }
    }

    fn view(&self, _id: window::Id) -> Element<'_, Message> {
        view::render(self)
    }

    fn command(&mut self, command: ipc::Command) -> Task<Message> {
        match command {
            ipc::Command::Toggle => self.toggle(),
            ipc::Command::Show => self.reveal(Page::Search),
            ipc::Command::Hide => self.hide(),
            ipc::Command::Settings => {
                let resize = self.resize_for(Page::Settings);
                let reveal = self.reveal(Page::Settings);
                Task::batch([resize, reveal, self.open_settings()])
            }
            ipc::Command::Query(query) => {
                let reveal = self.reveal(Page::Search);
                Task::batch([reveal, self.input(query)])
            }
            ipc::Command::Quit => iced::exit(),
        }
    }

    fn toggle(&mut self) -> Task<Message> {
        if self.window.is_some() {
            self.hide()
        } else {
            self.reveal(Page::Search)
        }
    }

    /// Show the launcher on `page`. When it is already visible this only refocuses it.
    fn reveal(&mut self, page: Page) -> Task<Message> {
        if self.window.is_some() {
            return self.focus();
        }
        self.page = page;
        self.query.clear();
        self.generation += 1;
        self.results = search::query("", &self.apps, &self.files, &self.config.web_engine);
        self.selected = 0;
        self.answer = Answer::default();
        self.answer_canceled = false;
        self.error = None;
        let (id, open) = open_window(page_height(page));
        self.window = Some(id);
        let mut tasks = vec![open];
        if self.indexed_at.elapsed() > INDEX_MAX_AGE && !self.indexing {
            tasks.push(self.index());
        }
        Task::batch(tasks)
    }

    fn hide(&mut self) -> Task<Message> {
        self.stop_answer();
        self.generation += 1;
        self.draft.key.clear();
        match self.window.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    fn focus(&self) -> Task<Message> {
        match self.page {
            Page::Search => operation::focus(INPUT.clone()),
            Page::Settings => operation::focus(MODEL_INPUT.clone()),
        }
    }

    fn index(&mut self) -> Task<Message> {
        self.indexing = true;
        self.index_request += 1;
        let request = self.index_request;
        let roots = self.config.search_roots.clone();
        let apps = Task::perform(blocking(search::discover_apps), move |apps| {
            Message::AppsReady(request, Arc::new(apps))
        });
        let files = Task::perform(
            blocking(move || search::build_file_index(&roots)),
            move |files| Message::FilesReady(request, Arc::new(files)),
        );
        Task::batch([apps, files])
    }

    fn search(&self) -> Task<Message> {
        let query = self.query.clone();
        let apps = self.apps.clone();
        let files = self.files.clone();
        let engine = self.config.web_engine.clone();
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::time::sleep(SEARCH_DELAY).await;
                blocking(move || search::query(&query, &apps, &files, &engine)).await
            },
            move |results| Message::SearchReady(generation, results),
        )
    }

    fn search_if_plain(&self) -> Task<Message> {
        if question_prompt(&self.query).is_none() {
            self.search()
        } else {
            Task::none()
        }
    }

    fn input(&mut self, value: String) -> Task<Message> {
        if self.query == value {
            return Task::none();
        }
        self.stop_answer();
        self.query = value;
        self.generation += 1;
        self.selected = 0;
        self.answer = Answer::default();
        self.answer_canceled = false;
        self.error = None;
        self.results.clear();
        if question_prompt(&self.query).is_some() {
            self.answer.status = "Ready to ask — keep typing to edit".into();
            let generation = self.generation;
            let delay = Duration::from_millis(self.config.debounce_ms);
            Task::perform(
                async move {
                    tokio::time::sleep(delay).await;
                    generation
                },
                Message::AutoAsk,
            )
        } else {
            self.search()
        }
    }

    fn activate(&self, index: usize) -> Task<Message> {
        if self.page != Page::Search || question_prompt(&self.query).is_some() {
            return Task::none();
        }
        let Some(result) = self.results.get(index).cloned() else {
            return Task::none();
        };
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || launch::activate(&result))
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()))
            },
            move |result| Message::Activated(generation, result),
        )
    }

    fn ask(&mut self, prompt: String) -> Task<Message> {
        self.stop_answer();
        let cancel = CancellationToken::new();
        self.answer = Answer {
            prompt: Some(prompt.clone()),
            status: "Connecting to your model…".into(),
            running: true,
            started: Some(Instant::now()),
            cancel: Some(cancel.clone()),
            ..Answer::default()
        };
        let (tx, rx) = unbounded_channel();
        let generation = self.generation;
        let run = Task::future(ai::run(
            prompt,
            self.config.clone(),
            self.files.clone(),
            tx,
            cancel,
        ))
        .discard();
        let events = Task::stream(ai_events(rx)).map(move |event| Message::Ai(generation, event));
        // One handle stops both the harness future and the event stream.
        let (task, handle) = Task::batch([run, events]).abortable();
        self.answer.handle = Some(handle);
        task
    }

    /// Cancel the running answer, if any. Its events stop arriving once aborted.
    fn stop_answer(&mut self) {
        if let Some(cancel) = self.answer.cancel.take() {
            cancel.cancel();
        }
        if let Some(handle) = self.answer.handle.take() {
            handle.abort();
        }
        self.answer.running = false;
    }

    fn answer_event(&mut self, event: ai::Event) {
        match event {
            ai::Event::Status(status) => self.answer.status = status,
            ai::Event::Delta(part) => self.answer.text.push_str(&part),
            ai::Event::Done => self.finish_answer(None),
            ai::Event::Error(error) => self.finish_answer(Some(error)),
        }
    }

    fn finish_answer(&mut self, error: Option<String>) {
        self.answer.running = false;
        self.answer.elapsed = self.answer.started.map(|start| start.elapsed());
        self.answer.cancel = None;
        self.answer.handle = None;
        if error.is_some() {
            self.answer.status.clear();
        }
        self.answer.error = error;
    }

    fn open_settings(&mut self) -> Task<Message> {
        if self.answer.running {
            self.stop_answer();
            self.answer.prompt = None;
            self.answer.text.clear();
            self.answer.elapsed = None;
            self.answer_canceled = true;
        }
        self.generation += 1;
        self.page = Page::Settings;
        self.draft = Draft::new(&self.config);
        self.error = None;
        self.focus()
    }

    fn back(&mut self) -> Task<Message> {
        let resize = self.resize_for(Page::Search);
        self.page = Page::Search;
        self.draft.key.clear();
        self.error = self.answer_canceled.then(|| NOTICE_CANCELED.to_string());
        self.answer_canceled = false;
        Task::batch([resize, self.focus()])
    }

    /// Give the open window the height of `page`. Windows cannot be resized in
    /// place on Wayland, so the window is replaced by one of the new size. A
    /// window that is not open yet is sized by `reveal`.
    fn resize_for(&mut self, page: Page) -> Task<Message> {
        let Some(old) = self.window else {
            return Task::none();
        };
        if self.page == page {
            return Task::none();
        }
        let (id, open) = open_window(page_height(page));
        self.window = Some(id);
        Task::batch([window::close(old), open])
    }

    /// A shown winit window gets its native backdrop. Layer surfaces have none.
    fn prepare(&self, id: window::Id) -> Task<Message> {
        match backend() {
            Backend::Winit => window::run(id, prepare_window).map(Message::WindowPrepared),
            #[cfg(target_os = "linux")]
            Backend::LayerShell { .. } => Task::none(),
        }
    }

    fn save(&mut self) -> Task<Message> {
        let config = self.draft.to_config();
        let checked = config
            .validate()
            .and_then(|()| hotkey::parse(&config.hotkey).map(|_| ()));
        if let Err(error) = checked {
            self.draft.error = Some(error);
            return Task::none();
        }
        if let Err(error) = self.hotkey.register(&config.hotkey) {
            self.draft.error = Some(error);
            return Task::none();
        }
        let key = std::mem::take(&mut self.draft.key);
        self.draft.saving = true;
        self.draft.error = None;
        Task::perform(
            blocking_result(move || {
                config.store_key(&key)?;
                config.save()?;
                autostart::set(config.autostart)?;
                Ok(config)
            }),
            Message::Saved,
        )
    }

    /// State without a window or a hotkey registration, for unit tests.
    #[cfg(test)]
    fn test_state() -> Self {
        Self {
            window: None,
            backdrop: false,
            page: Page::Search,
            config: Config::default(),
            draft: Draft::new(&Config::default()),
            hotkey: Hotkey::new(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            apps: Arc::default(),
            files: Arc::default(),
            icons: Arc::default(),
            indexing: false,
            index_request: 0,
            indexed_at: Instant::now(),
            generation: 0,
            answer: Answer::default(),
            answer_canceled: false,
            error: None,
        }
    }
}

/// Run blocking work off the UI thread. A panicking worker yields the default.
async fn blocking<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Default + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.unwrap_or_default()
}

/// Like [`blocking`] for fallible work; join failures become the error text.
async fn blocking_result<T, F>(work: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(error.to_string()))
}

fn ai_events(
    rx: UnboundedReceiver<ai::Event>,
) -> impl iced::futures::Stream<Item = ai::Event> + Send + 'static {
    iced::futures::stream::unfold(rx, |mut rx| async move {
        let event = rx.recv().await?;
        Some((event, rx))
    })
}

fn ipc_commands() -> impl iced::futures::Stream<Item = ipc::Command> + Send + 'static {
    iced::futures::stream::unfold(lock(&IPC).take(), |rx| async move {
        let mut rx = rx?;
        let command = rx.recv().await?;
        Some((command, Some(rx)))
    })
}

fn keyboard_event(event: iced::Event, _: event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => match key {
            Key::Named(Named::Escape) => Some(Message::Escape),
            Key::Named(Named::ArrowDown) => Some(Message::Move(1)),
            Key::Named(Named::ArrowUp) => Some(Message::Move(-1)),
            Key::Character(c) if modifiers.control() && c == "," => Some(Message::OpenSettings),
            _ => None,
        },
        iced::Event::Window(window::Event::Unfocused) => Some(Message::Unfocused(id)),
        // Layer surfaces have no open task to report their id, so use the event.
        iced::Event::Window(window::Event::Opened { .. })
            if !matches!(backend(), Backend::Winit) =>
        {
            Some(Message::WindowOpened(id))
        }
        _ => None,
    }
}

fn page_height(page: Page) -> f32 {
    match page {
        Page::Search => HEIGHT,
        Page::Settings => SETTINGS_HEIGHT,
    }
}

/// Create the surface for the launcher at `height` logical pixels. Its id is
/// known straight away; winit reports it as `WindowOpened` once shown, and
/// layer surfaces send the `Opened` window event (see `keyboard_event`).
fn open_window(height: f32) -> (window::Id, Task<Message>) {
    match backend() {
        Backend::Winit => {
            let (id, open) = window::open(window_settings(height));
            (id, open.map(Message::WindowOpened))
        }
        #[cfg(target_os = "linux")]
        Backend::LayerShell { max_height } => wayland::open(height.min(max_height)),
    }
}

fn window_settings(height: f32) -> window::Settings {
    window::Settings {
        size: Size::new(WIDTH, height),
        position: window::Position::Centered,
        resizable: false,
        decorations: false,
        transparent: true,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        platform_specific: platform_specific(),
        ..Default::default()
    }
}

#[cfg(windows)]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        skip_taskbar: true,
        ..Default::default()
    }
}

#[cfg(not(windows))]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        application_id: spotlight::config::APP_ID.into(),
        ..Default::default()
    }
}

/// Native backdrop and foreground handling, run on the event-loop thread.
fn prepare_window(window: &dyn window::Window) -> bool {
    #[cfg(windows)]
    {
        platform::windows::prepare(window)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}

/// A query that ends in `?` asks the AI instead of searching.
pub fn question_prompt(query: &str) -> Option<String> {
    let query = query.trim();
    query
        .strip_suffix('?')
        .filter(|s| !s.trim().is_empty())
        .map(|_| query.to_string())
}

/// Move the selection by one step, wrapping at both ends.
pub fn step(selected: usize, len: usize, direction: i32) -> usize {
    (selected as i64 + i64::from(direction)).rem_euclid(len as i64) as usize
}

/// Vertical scroll position (0 to 1) that keeps the selection in view.
pub fn scroll_fraction(selected: usize, len: usize) -> f32 {
    selected as f32 / len.saturating_sub(1).max(1) as f32
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
    fn selection_wraps_in_both_directions() {
        assert_eq!(step(0, 3, -1), 2);
        assert_eq!(step(2, 3, 1), 0);
        assert_eq!(step(1, 3, 1), 2);
        assert_eq!(step(0, 1, 1), 0);
    }

    #[test]
    fn scroll_reaches_both_ends_of_the_list() {
        assert_eq!(scroll_fraction(0, 1), 0.0);
        assert_eq!(scroll_fraction(0, 5), 0.0);
        assert_eq!(scroll_fraction(4, 5), 1.0);
        assert_eq!(scroll_fraction(2, 5), 0.5);
    }

    #[test]
    fn stale_answers_are_ignored_after_a_new_generation() {
        let mut app = Spotlight::test_state();
        app.generation = 2;
        app.answer.running = true;
        let _ = app.update(Message::Ai(1, ai::Event::Delta("old".into())));
        assert!(app.answer.text.is_empty());
        let _ = app.update(Message::Ai(2, ai::Event::Delta("new".into())));
        assert_eq!(app.answer.text, "new");
    }

    #[test]
    fn stale_search_results_are_dropped() {
        let mut app = Spotlight::test_state();
        app.generation = 5;
        app.query = "fire".into();
        let _ = app.update(Message::SearchReady(4, Vec::new()));
        assert!(app.results.is_empty());
    }
}
