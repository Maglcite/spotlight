//! Wayland sessions. The launcher runs as a wlr-layer-shell overlay so the
//! compositor centers it. Compositors such as COSMIC ignore the window
//! position that winit requests on Wayland.

use super::{Message, Spotlight, WIDTH, style};
use iced::window;
use iced_layershell::{
    build_pattern,
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::WlOutput,
        wl_registry::{self, WlRegistry},
    },
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};

/// Share of the output's logical height the panel may use, so the Settings
/// page still fits on smaller screens.
const OUTPUT_FRACTION: f32 = 0.85;
const NAMESPACE: &str = "Spotlight";
const KEEPALIVE_NAMESPACE: &str = "Spotlight-keepalive";

/// Returns the largest surface height for the outputs when this session can
/// use layer-shell surfaces, and `None` otherwise (so winit is used).
pub fn probe() -> Option<f32> {
    std::env::var_os("WAYLAND_DISPLAY")?;
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection).ok()?;
    let qh = queue.handle();
    let list = globals.contents().clone_list();
    if !list.iter().any(|g| g.interface == "zwlr_layer_shell_v1") {
        return None;
    }
    let manager: ZxdgOutputManagerV1 = globals.bind(&qh, 1..=3, ()).ok()?;
    for global in list.iter().filter(|g| g.interface == "wl_output") {
        let output: WlOutput = globals
            .registry()
            .bind(global.name, global.version.min(4), &qh, ());
        manager.get_xdg_output(&output, &qh, ());
    }
    let mut probe = Probe::default();
    queue.roundtrip(&mut probe).ok()?;
    // Use the shortest output so the panel fits on every screen.
    let shortest = probe.heights.into_iter().min()?;
    Some(shortest as f32 * OUTPUT_FRACTION)
}

/// Open the launcher surface at `height` logical pixels, centered by the
/// compositor.
pub fn open(height: f32) -> (window::Id, iced::Task<Message>) {
    Message::layershell_open(layer_settings(height))
}

/// Open a 1x1 transparent background keepalive surface so `iced_layershell`
/// never destroys its `wgpu` compositor, shader pipelines, or EGL display when
/// the main launcher window is closed.
pub fn open_keepalive() -> (window::Id, iced::Task<Message>) {
    Message::layershell_open(NewLayerShellSettings {
        size: Some((1, 1)),
        layer: Layer::Background,
        anchor: Anchor::empty(),
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: OutputOption::Active,
        events_transparent: true,
        namespace: Some(KEEPALIVE_NAMESPACE.into()),
    })
}

/// Run the launcher daemon on layer-shell surfaces. Shares the state, update
/// and view code with the winit daemon in `super`.
pub fn run() -> Result<(), String> {
    build_pattern::daemon(
        Spotlight::boot,
        NAMESPACE,
        Spotlight::update,
        Spotlight::view,
    )
    .subscription(Spotlight::subscription)
    .theme(|_: &Spotlight, _: window::Id| style::theme())
    .style(|_: &Spotlight, theme: &iced::Theme| style::application(theme))
    .settings(Settings {
        antialiasing: true,
        layer_settings: LayerShellSettings {
            // Surfaces are created on demand by `open`, so start with none.
            start_mode: StartMode::Background,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|error| error.to_string())
}

fn layer_settings(height: f32) -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some((WIDTH as u32, height.round() as u32)),
        layer: Layer::Top,
        // No anchors: the compositor centers the surface on the output.
        anchor: Anchor::empty(),
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        output_option: OutputOption::Active,
        events_transparent: false,
        namespace: Some(NAMESPACE.into()),
    }
}

/// Collects the logical height of each output. Only used by [`probe`].
#[derive(Default)]
struct Probe {
    heights: Vec<i32>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgOutputV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_output_v1::Event::LogicalSize { height, .. } = event {
            state.heights.push(height);
        }
    }
}

delegate_noop!(Probe: ignore WlOutput);
delegate_noop!(Probe: ignore ZxdgOutputManagerV1);
