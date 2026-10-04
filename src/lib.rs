//! # Authoring notebooks
//!
//! Compose cells with private state and useful public values, in the spirit of
//! Observable and marimo. A map owns its camera and drawing cache; it publishes
//! a selection that a plot can consume without depending on the map controller.
//! The notebook body connects those interfaces, rather than owning every
//! instrument in one mutable application object.
//!
//! This reactive composition is independent of immediate-mode rendering: card
//! bodies run on repaint, while keyed state survives. Lissajous does not analyze
//! source code or automatically schedule a dependency graph. Use
//! [`NotebookCtx::state`] for lazy retained state, [`state::StateId`] for
//! explicit typed access, and [`dataflow::DerivedState`] for small synchronous
//! derivations keyed by all relevant inputs. [`dataflow::ComputedState`] supplies
//! a native background result slot, not keyed cancellation or latest-request
//! scheduling; asynchronous consumers must preserve input identity and reject
//! stale answers. Keep heavy work off paint and share a resource session or
//! immutable source observations instead of opening one reader per cell.
//!
//! See the [notebook authoring guide and worked map example](
//! https://github.com/erlkonig-ai/lissajous#authoring-notebooks) in the packaged
//! README and examples. Detaching a card changes placement, not ownership.
//! Coordinated access to several handles remains useful when it is the actual
//! operation; keep lock scopes short and avoid reacquiring a cell's own state
//! from inside its write-locked callback.
//!

#![allow(non_snake_case)]

/// Native headless rendering with resident PNG delivery.
#[cfg(not(target_arch = "wasm32"))]
pub mod capture;
/// Card context — the `&mut CardCtx` passed to card closures.
pub mod card_ctx;
/// Card trait and built-in card types (stateful, stateless).
pub mod cards;
/// Background computation with [`ComputedState`](dataflow::ComputedState).
pub mod dataflow;
pub(crate) mod floating;
/// One force-directed layout and one drawing kit, shared by every graph view.
pub mod graph;
#[cfg(not(target_arch = "wasm32"))]
mod headless;
/// Convenient glob import of common types and constants.
pub mod prelude;
/// Notebook-wide search bar — opt-in via [`CardCtx::search`].
pub mod search;
/// Thread-safe state management via [`StateId`](state::StateId) handles.
pub mod state;
/// Tracing telemetry capture (requires `telemetry` feature).
#[cfg(feature = "telemetry")]
pub mod telemetry;
/// Visual themes, RAL colors, and widget style structs.
pub mod themes;
/// Built-in widgets: buttons, fields, sliders, progress bars, and more.
pub mod widgets;

#[cfg(not(target_arch = "wasm32"))]
pub use capture::{CaptureOptions, CaptureResult, CapturedPng};
pub use lissajous_macros::__gorbie_web_export;
pub use lissajous_macros::notebook;

use crate::themes::industrial_dark;
use crate::themes::industrial_fonts;
use crate::themes::industrial_light;
use eframe::egui::{self};
use std::any::TypeId;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use dark_light::Mode;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SourceLocation {
    file: String,
    line: u32,
    column: u32,
}

impl SourceLocation {
    fn from_location(location: &'static std::panic::Location<'static>) -> Self {
        Self {
            file: location.file().to_string(),
            line: location.line(),
            column: location.column(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn format_arg(&self, template: &str) -> String {
        let file = &self.file;
        let line = self.line;
        let column = self.column;
        template
            .replace("{file}", file)
            .replace("{line}", &line.to_string())
            .replace("{column}", &column.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_line_column(&self) -> String {
        let file = &self.file;
        let line = self.line;
        let column = self.column;
        format!("{file}:{line}:{column}")
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum CardIdentityKey {
    Stateless {
        source: Option<SourceLocation>,
        function: TypeId,
    },
    Stateful {
        source: Option<SourceLocation>,
        state: egui::Id,
        function: TypeId,
    },
    Custom,
}

/// Command template for opening source files in an external editor.
///
/// Argument strings may contain `{file}`, `{line}`, and `{column}` placeholders
/// that are expanded when a card's source location is opened.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct EditorCommand {
    program: String,
    args: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl EditorCommand {
    /// Creates a new editor command with the given program name.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Appends an argument to the command. Supports `{file}`, `{line}`, and
    /// `{column}` placeholders.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    fn open(&self, source: &SourceLocation) -> std::io::Result<()> {
        let mut cmd = Command::new(&self.program);
        if self.args.is_empty() {
            cmd.arg(source.file_line_column());
        } else {
            for arg in &self.args {
                cmd.arg(source.format_arg(arg));
            }
        }
        let _child = cmd.spawn()?;
        Ok(())
    }
}

struct CardEntry {
    card: Box<dyn cards::Card + 'static>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    source: Option<SourceLocation>,
    identity: egui::Id,
}

#[derive(Clone, Default)]
struct NotebookState {
    card_detached: Vec<bool>,
    card_placeholder_sizes: Vec<egui::Vec2>,
    card_identities: Vec<Option<egui::Id>>,
}

impl NotebookState {
    fn sync_len(&mut self, len: usize) {
        self.card_detached.resize(len, false);
        self.card_placeholder_sizes.resize(len, egui::Vec2::ZERO);
        self.card_identities.resize(len, None);
    }

    fn ensure_card_identity(&mut self, index: usize, identity: egui::Id) {
        let slot = self
            .card_identities
            .get_mut(index)
            .expect("card_identities synced to cards");
        if slot.map_or(true, |prev| prev != identity) {
            *slot = Some(identity);
            *self
                .card_detached
                .get_mut(index)
                .expect("card_detached synced to cards") = false;
            *self
                .card_placeholder_sizes
                .get_mut(index)
                .expect("card_placeholder_sizes synced to cards") = egui::Vec2::ZERO;
        }
    }
}

/// Configuration for a notebook application.
pub struct NotebookConfig {
    title: String,
    #[cfg(not(target_arch = "wasm32"))]
    editor: Option<EditorCommand>,
    #[cfg(not(target_arch = "wasm32"))]
    headless_capture: Option<HeadlessCaptureConfig>,
    #[cfg(not(target_arch = "wasm32"))]
    headless_settle_timeout: Option<Duration>,
    #[cfg(not(target_arch = "wasm32"))]
    headless_theme: HeadlessTheme,
}

/// Which theme a headless capture renders in.
///
/// Defaults to [`Dark`](HeadlessTheme::Dark) rather than following the desktop,
/// so a render is a pure function of the notebook and this flag. Ambient system
/// state used to leak in here and made captures non-reproducible — the same
/// command produced a light or a dark PNG depending on the operator's machine.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeadlessTheme {
    /// Render light.
    Light,
    /// Render dark. The default.
    #[default]
    Dark,
    /// Follow the operating system's appearance.
    ///
    /// Opt-in only, because it is not reproducible and, on macOS, querying it
    /// goes through System Events — which prompts for automation permission and
    /// has been observed changing the machine's own appearance.
    Auto,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct HeadlessCaptureConfig {
    output_dir: PathBuf,
    pixels_per_point: f32,
    settle_timeout: Duration,
    theme: HeadlessTheme,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct AppIcons {
    light: Arc<egui::IconData>,
    dark: Arc<egui::IconData>,
}

struct NotebookCore {
    config: NotebookConfig,
    body: Box<dyn FnMut(&mut NotebookCtx)>,
    state_store: Arc<state::StateStore>,
    settled: Arc<AtomicBool>,
}

struct Notebook {
    core: NotebookCore,
    #[cfg(not(target_arch = "wasm32"))]
    icons: Option<AppIcons>,
    #[cfg(not(target_arch = "wasm32"))]
    icon_is_dark: Option<bool>,
    #[cfg(feature = "telemetry")]
    #[allow(dead_code)]
    telemetry: Option<telemetry::Telemetry>,
}

/// Frame-scoped notebook builder used to collect cards in immediate mode.
pub struct NotebookCtx {
    state_id: egui::Id,
    cards: Vec<CardEntry>,
    state_store: Arc<state::StateStore>,
    settled: Arc<AtomicBool>,
}

pub use card_ctx::CardCtx;
pub use card_ctx::Grid;
pub use card_ctx::GRID_COLUMNS;
pub use card_ctx::GRID_COL_WIDTH;
pub use card_ctx::GRID_GUTTER;

/// Context-data key for the headless-capture marker.
fn headless_marker_id() -> egui::Id {
    egui::Id::new("gorbie_headless_capture")
}

/// True when `ctx` is driven by the headless capture renderer
/// (`--headless` / [`NotebookConfig::with_headless_capture`]).
///
/// Widgets can consult this to adapt rendering for screenshots —
/// e.g. [`CardCtx::section`] forces sections open in headless mode so
/// captures always show their contents, regardless of the interactive
/// collapsed-by-default setting.
pub fn is_headless(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(headless_marker_id()))
        .unwrap_or(false)
}

/// Mark `ctx` as headless. Called by the capture renderer during setup.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn mark_headless(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(headless_marker_id(), true));
}

/// Context-data key for the wgpu render-target colour format.
fn wgpu_target_format_id() -> egui::Id {
    egui::Id::new("gorbie_wgpu_target_format")
}

/// The wgpu colour-target format the notebook is rendering into, when
/// running on the wgpu backend (which GORBIE selects on native — see
/// [`NotebookConfig::run`]). `None` under glow or before the first
/// frame.
///
/// A custom 3D card (e.g. an `egui_wgpu` paint callback) needs this to
/// build a render pipeline whose fragment-target format matches egui's
/// pass; it isn't otherwise available from inside a callback's
/// `prepare`. The notebook publishes it each frame from the eframe
/// [`Frame`](eframe::Frame).
pub fn wgpu_target_format(ctx: &egui::Context) -> Option<egui_wgpu::wgpu::TextureFormat> {
    ctx.data(|d| d.get_temp(wgpu_target_format_id()))
}

pub(crate) const NOTEBOOK_COLUMN_WIDTH: f32 = 768.0;
#[cfg(not(target_arch = "wasm32"))]
const NOTEBOOK_MIN_HEIGHT: f32 = 360.0;
#[cfg(not(target_arch = "wasm32"))]
const HEADLESS_DEFAULT_PIXELS_PER_POINT: f32 = 2.0;
#[cfg(not(target_arch = "wasm32"))]
const HEADLESS_DEFAULT_SETTLE_TIMEOUT: Duration = Duration::from_millis(2000);

impl Default for NotebookConfig {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl NotebookConfig {
    /// Creates a new configuration with the given application title.
    ///
    /// The editor command is auto-detected from the `GORBIE_EDITOR` environment
    /// variable; use [`with_editor`](Self::with_editor) to override.
    pub fn new(name: impl Into<String>) -> Self {
        let title = name.into();
        Self {
            title,
            #[cfg(not(target_arch = "wasm32"))]
            editor: editor_from_env(),
            #[cfg(not(target_arch = "wasm32"))]
            headless_capture: None,
            #[cfg(not(target_arch = "wasm32"))]
            headless_settle_timeout: None,
            #[cfg(not(target_arch = "wasm32"))]
            headless_theme: HeadlessTheme::default(),
        }
    }

    /// Overrides the editor command used for "open in editor" buttons.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_editor(mut self, editor: EditorCommand) -> Self {
        self.editor = Some(editor);
        self
    }

    /// Enables headless capture mode, rendering each card to a PNG in `output_dir`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_headless_capture(mut self, output_dir: impl Into<PathBuf>) -> Self {
        let settle_timeout = self
            .headless_settle_timeout
            .unwrap_or(HEADLESS_DEFAULT_SETTLE_TIMEOUT);
        self.headless_capture = Some(HeadlessCaptureConfig {
            output_dir: output_dir.into(),
            pixels_per_point: HEADLESS_DEFAULT_PIXELS_PER_POINT,
            settle_timeout,
            theme: self.headless_theme,
        });
        self
    }

    /// Sets which theme a headless capture renders in.
    ///
    /// Defaults to [`HeadlessTheme::Dark`]. Call before
    /// [`with_headless_capture`](Self::with_headless_capture) — the capture
    /// config snapshots this value.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_headless_theme(mut self, theme: HeadlessTheme) -> Self {
        self.headless_theme = theme;
        if let Some(headless) = &mut self.headless_capture {
            headless.theme = theme;
        }
        self
    }

    /// Like [`with_headless_capture`](Self::with_headless_capture), but with a
    /// custom `pixels_per_point` scaling factor for the rendered output.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_headless_capture_scaled(
        mut self,
        output_dir: impl Into<PathBuf>,
        pixels_per_point: f32,
    ) -> Self {
        let pixels_per_point = if pixels_per_point > 0.0 {
            pixels_per_point
        } else {
            HEADLESS_DEFAULT_PIXELS_PER_POINT
        };
        let settle_timeout = self
            .headless_settle_timeout
            .unwrap_or(HEADLESS_DEFAULT_SETTLE_TIMEOUT);
        self.headless_capture = Some(HeadlessCaptureConfig {
            output_dir: output_dir.into(),
            pixels_per_point,
            settle_timeout,
            theme: self.headless_theme,
        });
        self
    }

    /// Sets the settle timeout for headless capture. The renderer waits up to
    /// this duration for the UI to stabilize before capturing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_headless_settle_timeout(mut self, timeout: Duration) -> Self {
        self.headless_settle_timeout = Some(timeout);
        if let Some(headless) = &mut self.headless_capture {
            headless.settle_timeout = timeout;
        }
        self
    }

    /// Render native headless cards into resident PNGs without a window or
    /// filesystem output. `emit` runs synchronously in card/page order; its
    /// first failure stops capture, with no retry of accepted output. Rendering
    /// and settling are shared with file captures, not a separate renderer.
    ///
    /// `options` controls scale and settling independently of file-capture
    /// settings. The theme follows [`with_headless_theme`](Self::with_headless_theme)
    /// and defaults to dark. A card's raw tiles may be resident while it is
    /// encoded; this is not a GPU memory quota or an overall rendering deadline.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn capture(
        self,
        options: CaptureOptions,
        body: impl FnMut(&mut NotebookCtx) + 'static,
        mut emit: impl FnMut(CapturedPng) -> CaptureResult<()>,
    ) -> CaptureResult<()> {
        options.validate()?;
        let theme = self.headless_theme;
        headless::capture(
            NotebookCore::new(self, Box::new(body)),
            options,
            theme,
            &mut emit,
        )
    }

    fn state_id(&self) -> egui::Id {
        egui::Id::new(("gorbie_notebook_state", self.title.as_str()))
    }

    /// Launches the notebook application.
    ///
    /// On native, the `body` closure is called once per frame in an interactive
    /// window (or headless capture mode). On wasm, the notebook renders into a
    /// `<canvas id="gorbie_canvas">` element.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(self, body: impl FnMut(&mut NotebookCtx) + 'static) -> eframe::Result {
        let config = self;
        if let Some(headless) = config.headless_capture.clone() {
            return headless::run_headless(NotebookCore::new(config, Box::new(body)), headless)
                .map_err(eframe::Error::AppCreation);
        }

        let window_title = if config.title.is_empty() {
            "GORBIE".to_owned()
        } else {
            config.title.clone()
        };

        let icons = load_app_icons();
        let mut native_options = eframe::NativeOptions::default();
        // Render live on the wgpu backend (Metal/Vulkan/D3D12), not glow.
        // Unifies the live renderer with the headless capture path and
        // cubecl's GPU compute — one GPU API across the stack — and is
        // what custom 3D paint callbacks (the globe widget) require:
        // `Frame::wgpu_render_state()` is `None` under the glow backend.
        native_options.renderer = eframe::Renderer::Wgpu;
        native_options.persist_window = true;
        native_options.viewport = native_options
            .viewport
            .with_inner_size(egui::vec2(1200.0, 800.0))
            .with_min_inner_size(egui::vec2(NOTEBOOK_COLUMN_WIDTH, NOTEBOOK_MIN_HEIGHT));

        if let Some(icons) = icons.as_ref() {
            let icon = match dark_light::detect() {
                Ok(Mode::Light) => icons.light.clone(),
                Ok(Mode::Dark) => icons.dark.clone(),
                Ok(Mode::Unspecified) | Err(_) => icons.dark.clone(),
            };
            native_options.viewport = native_options.viewport.with_icon(icon);
        }

        let body = Box::new(body);
        eframe::run_native(
            &window_title,
            native_options,
            Box::new(|cc| {
                let ctx = cc.egui_ctx.clone();
                ctrlc::set_handler(move || ctx.send_viewport_cmd(egui::ViewportCommand::Close))
                    .expect("failed to set exit signal handler");

                cc.egui_ctx.set_fonts(industrial_fonts());

                cc.egui_ctx
                    .set_style_of(egui::Theme::Light, industrial_light());
                cc.egui_ctx
                    .set_style_of(egui::Theme::Dark, industrial_dark());

                #[cfg(feature = "telemetry")]
                let telemetry_title = config.title.clone();
                Ok(Box::new(Notebook {
                    core: NotebookCore::new(config, body),
                    icons,
                    icon_is_dark: None,
                    #[cfg(feature = "telemetry")]
                    telemetry: telemetry::Telemetry::install_global_from_env(&telemetry_title),
                }))
            }),
        )
    }

    /// Launches the notebook in the browser.
    ///
    /// Looks for a `<canvas id="gorbie_canvas">` element and starts the eframe
    /// web runner on it.
    #[cfg(target_arch = "wasm32")]
    pub fn run(
        self,
        body: impl FnMut(&mut NotebookCtx) + 'static,
    ) -> Result<(), wasm_bindgen::JsValue> {
        use wasm_bindgen::JsCast;

        eframe::WebLogger::init(log::LevelFilter::Debug).ok();

        let canvas_id = "gorbie_canvas";
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id(canvas_id)
            .unwrap_or_else(|| panic!("no <canvas id=\"{canvas_id}\"> found"))
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("element is not a canvas");

        let runner = eframe::WebRunner::new();
        let body = Box::new(body);
        let config = self;

        wasm_bindgen_futures::spawn_local(async move {
            runner
                .start(
                    canvas,
                    eframe::WebOptions::default(),
                    Box::new(|cc| {
                        cc.egui_ctx.set_fonts(industrial_fonts());
                        cc.egui_ctx
                            .set_style_of(egui::Theme::Light, industrial_light());
                        cc.egui_ctx
                            .set_style_of(egui::Theme::Dark, industrial_dark());

                        Ok(Box::new(Notebook {
                            core: NotebookCore::new(config, body),
                            #[cfg(feature = "telemetry")]
                            telemetry: None,
                        }))
                    }),
                )
                .await
                .expect("eframe start failed");
        });
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_app_icons() -> Option<AppIcons> {
    let light =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_light.png")).ok()?;
    let dark = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_dark.png")).ok()?;
    Some(AppIcons {
        light: Arc::new(light),
        dark: Arc::new(dark),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn editor_from_env() -> Option<EditorCommand> {
    let gorbie_editor = std::env::var("GORBIE_EDITOR")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if gorbie_editor.is_none() {
        log_missing_editor_hint();
        return None;
    }
    let editor = gorbie_editor?;

    let mut parts = editor.split_whitespace();
    let program = parts.next()?.to_string();
    let args = parts.map(str::to_string);
    let mut command = EditorCommand::new(program);
    for arg in args {
        command = command.arg(arg);
    }
    Some(command)
}

#[cfg(not(target_arch = "wasm32"))]
fn log_missing_editor_hint() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        log::info!(
            "GORBIE_EDITOR is not set. Set it to an editor command with placeholders {{file}} {{line}} {{column}} to enable open-in-editor. Example: GORBIE_EDITOR='code -g {{file}}:{{line}}:{{column}}'."
        );
    });
}

impl state::StateAccess for NotebookCtx {
    fn store(&self) -> &state::StateStore {
        &self.state_store
    }
}

impl NotebookCtx {
    fn new(
        config: &NotebookConfig,
        state_store: Arc<state::StateStore>,
        settled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state_id: config.state_id(),
            cards: Vec::new(),
            state_store,
            settled,
        }
    }

    /// Signal that notebook content is fully loaded and ready for capture.
    /// In headless mode this triggers immediate capture instead of waiting
    /// for the settle timeout. In interactive mode this is a no-op.
    pub fn settled(&self) {
        self.settled.store(true, Ordering::Relaxed);
    }

    /// Adds a stateless card whose content is drawn by `function` each frame.
    #[track_caller]
    pub fn view<F>(&mut self, function: F)
    where
        F: for<'a, 'b> FnMut(&'a mut CardCtx<'b>) + 'static,
    {
        let source = SourceLocation::from_location(std::panic::Location::caller());
        let identity = self.card_identity(CardIdentityKey::Stateless {
            source: Some(source.clone()),
            function: TypeId::of::<F>(),
        });
        let card = cards::StatelessCard::new(function);
        self.push_with_source(Box::new(card), Some(source), identity);
    }

    /// Adds a stateful card, constructing its value only when the key is absent.
    ///
    /// `init` runs only while the state key is absent. Once it returns,
    /// subsequent frames rebuild the card around the existing value without
    /// invoking the initializer. If initialization panics, no state is
    /// installed and a later call may retry.
    /// Initialization is synchronous, not a dependency computation. Read
    /// other state in the notebook body or draw callback, not in `init`:
    /// insertion holds the state store's write lock while invoking it.
    ///
    /// Returns a [`StateId`](state::StateId) handle for reading/writing the
    /// state from other cards.
    /// Prefer exposing useful values or narrow interfaces over passing a whole
    /// application controller; see the [crate-level authoring guide](crate).
    #[track_caller]
    pub fn state<K, T, I, F>(&mut self, key: &K, init: I, function: F) -> state::StateId<T>
    where
        K: std::hash::Hash + ?Sized,
        T: Send + Sync + 'static,
        I: FnOnce() -> T,
        F: for<'a, 'b> FnMut(&'a mut CardCtx<'b>, &mut T) + 'static,
    {
        let source = SourceLocation::from_location(std::panic::Location::caller());
        let state_id = self.state_id_for(key);
        let identity = self.card_identity(CardIdentityKey::Stateful {
            source: Some(source.clone()),
            state: state_id,
            function: TypeId::of::<F>(),
        });
        let state = state::StateId::new(state_id);
        let handle = state;
        self.state_store.get_or_insert_with(state, init);
        let card = cards::StatefulCard::new(state, function);
        self.push_with_source(Box::new(card), Some(source), identity);
        handle
    }

    /// Adds a pre-built [`Card`](cards::Card) trait object to the notebook.
    pub fn push(&mut self, card: Box<dyn cards::Card>) {
        let identity = self.card_identity(CardIdentityKey::Custom);
        self.push_with_source(card, None, identity);
    }

    pub(crate) fn state_id_for<K: std::hash::Hash + ?Sized>(&self, key: &K) -> egui::Id {
        self.state_id.with(("state", key))
    }

    fn card_identity(&self, key: CardIdentityKey) -> egui::Id {
        self.state_id.with(("card", key))
    }

    fn push_with_source(
        &mut self,
        card: Box<dyn cards::Card>,
        source: Option<SourceLocation>,
        identity: egui::Id,
    ) {
        self.cards.push(CardEntry {
            card,
            source,
            identity,
        });
    }
}

impl NotebookCore {
    fn new(config: NotebookConfig, body: Box<dyn FnMut(&mut NotebookCtx)>) -> Self {
        Self {
            config,
            body,
            state_store: Arc::new(state::StateStore::default()),
            settled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn has_settled(&self) -> bool {
        self.settled.swap(false, Ordering::Relaxed)
    }

    fn build_notebook(&mut self) -> NotebookCtx {
        let mut notebook =
            NotebookCtx::new(&self.config, self.state_store.clone(), self.settled.clone());
        (self.body)(&mut notebook);
        notebook
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn draw_card(
        &self,
        ctx: &egui::Context,
        notebook: &mut NotebookCtx,
        index: usize,
        card_width: f32,
    ) -> Option<f32> {
        if index >= notebook.cards.len() {
            return None;
        }

        let store = notebook.state_store.clone();
        let mut measured_height: Option<f32> = None;
        let panel_fill = ctx.global_style().visuals.window_fill;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(panel_fill))
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(card_width, 0.0));
                if let Some(entry) = notebook.cards.get_mut(index) {
                    let card: &mut dyn cards::Card = entry.card.as_mut();
                    let rect = draw_card_body(ui, card_width, card, store.as_ref(), None);
                    measured_height = Some(rect.height());
                }
            });

        measured_height
    }
}

impl Notebook {
    #[cfg(not(target_arch = "wasm32"))]
    fn update_app_icon(&mut self, ctx: &egui::Context) {
        let Some(icons) = self.icons.as_ref() else {
            return;
        };
        let is_dark = matches!(ctx.theme(), egui::Theme::Dark);
        if self.icon_is_dark == Some(is_dark) {
            return;
        }

        let icon = if is_dark {
            icons.dark.clone()
        } else {
            icons.light.clone()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(icon)));
        self.icon_is_dark = Some(is_dark);
    }
}

impl eframe::App for Notebook {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Publish the wgpu colour-target format so custom 3D cards can
        // build pipelines that match egui's pass (see
        // `wgpu_target_format`). Present only on the wgpu backend.
        if let Some(rs) = frame.wgpu_render_state() {
            let fmt = rs.target_format;
            ctx.data_mut(|d| d.insert_temp(wgpu_target_format_id(), fmt));
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.update_app_icon(&ctx);
        ctx.global_style_mut(|style| {
            style.visuals.clip_rect_margin = 0.0;
        });

        #[cfg(feature = "telemetry")]
        let _telemetry_frame = tracing::info_span!("frame").entered();

        let mut notebook = {
            #[cfg(feature = "telemetry")]
            let _build_span = tracing::info_span!("build_notebook").entered();
            self.core.build_notebook()
        };
        let config = &self.core.config;

        let state_id = config.state_id();
        let mut runtime = ctx.data_mut(|data| {
            let slot = data.get_temp_mut_or_insert_with(state_id, NotebookState::default);
            std::mem::take(slot)
        });

        // eframe 0.34's `fn ui` receives a bare `&mut Ui` with no margin
        // or background color (see the trait doc). Wrap in a central-panel
        // Frame so the notebook draws on the theme's panel fill instead
        // of transparent/black.
        egui::Frame::central_panel(&ctx.global_style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                // Disable drag-to-scroll on the main notebook scroller. egui's
                // hit-test panics (`hit_test.rs:365`) when a big drag-sensing
                // ScrollArea coexists with nearby click-sensing widgets —
                // which is every interactive card in a notebook. Users still
                // scroll via scroll bars + mouse wheel; touch-style drag-pan
                // is lost.
                .scroll_source(egui::scroll_area::ScrollSource {
                    scroll_bar: true,
                    drag: false,
                    mouse_wheel: true,
                })
                .show_viewport(ui, |ui, viewport| {
                    let rect = ui.max_rect();
                    let clip_rect = ui.clip_rect();
                    let scroll_y = viewport.min.y;

                    // Publish scroll info so floating cards can do anchor switching.
                    floating::store_scroll_info(
                        ui.ctx(),
                        floating::NotebookScrollInfo {
                            scroll_y,
                            viewport_top: clip_rect.min.y,
                            clip_rect,
                        },
                    );

                    let column_width = NOTEBOOK_COLUMN_WIDTH;
                    let left_margin_width = 0.0;
                    let card_width = column_width;

                    let left_margin_paint = egui::Rect::from_min_max(
                        egui::pos2(rect.min.x, clip_rect.min.y),
                        egui::pos2(rect.min.x + left_margin_width, clip_rect.max.y),
                    );
                    let left_margin = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.min.x + left_margin_width, rect.max.y),
                    );
                    let column_rect = egui::Rect::from_min_max(
                        egui::pos2(left_margin.max.x, rect.min.y),
                        egui::pos2(left_margin.max.x + column_width, rect.max.y),
                    );
                    let right_margin_paint = egui::Rect::from_min_max(
                        egui::pos2(column_rect.max.x, clip_rect.min.y),
                        egui::pos2(rect.max.x, clip_rect.max.y),
                    );
                    let right_margin = egui::Rect::from_min_max(
                        egui::pos2(column_rect.max.x, rect.min.y),
                        rect.max,
                    );

                    paint_reseau_grid(ui, left_margin_paint, scroll_y);
                    paint_reseau_grid(ui, right_margin_paint, scroll_y);

                    ui.scope_builder(egui::UiBuilder::new().max_rect(column_rect), |ui| {
                        // Keep column/background fills from painting into the margins when
                        // a too-wide widget forces a larger layout rect.
                        let restore_clip_rect = ui.clip_rect();
                        let column_clip_rect = egui::Rect::from_min_max(
                            egui::pos2(column_rect.min.x, restore_clip_rect.min.y),
                            egui::pos2(column_rect.max.x, restore_clip_rect.max.y),
                        );
                        ui.set_clip_rect(column_clip_rect);

                        // Min-size in y is bounded by the viewport, not by
                        // `max_rect` — `ui.max_rect()` inside ScrollArea
                        // reflects the previous frame's content size, so
                        // forcing the column to that height plus float
                        // overflow grows the scroll content by `float_overflow`
                        // every frame (linear "infinite scroll" once any
                        // content-anchored float exists).
                        ui.set_min_size(egui::vec2(column_rect.width(), clip_rect.height()));
                        ui.set_max_width(column_rect.width());

                        let fill = ui.visuals().window_fill;

                        let card_gap = ui.visuals().widgets.noninteractive.bg_stroke.width.max(1.0);
                        let card_gap_i8 = card_gap.round().clamp(0.0, i8::MAX as f32) as i8;
                        let column_inner_margin = egui::Margin {
                            left: 0,
                            right: 0,
                            top: 12,
                            bottom: card_gap_i8,
                        };
                        let column_frame = egui::Frame::new()
                            .fill(fill)
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(0.0)
                            .inner_margin(column_inner_margin)
                            .show(ui, |ui| {
                                // Theme switch is part of the page header (above the first card).
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    if !config.title.is_empty() {
                                        let header_title =
                                            egui::RichText::new(config.title.to_uppercase())
                                                .monospace()
                                                .strong();
                                        ui.add(egui::Label::new(header_title).truncate());
                                    }

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.add_space(16.0);
                                            let mut preference =
                                                ui.ctx().options(|opt| opt.theme_preference);
                                            if ui
                                                .add(
                                                    widgets::ChoiceToggle::new(&mut preference)
                                                        .choice(egui::ThemePreference::System, "◐")
                                                        .choice(egui::ThemePreference::Dark, "●")
                                                        .choice(egui::ThemePreference::Light, "○"),
                                                )
                                                .changed()
                                            {
                                                ui.ctx().set_theme(preference);
                                            }
                                        },
                                    );
                                });

                                ui.add_space(12.0);

                                let default_item_spacing = ui.style().spacing.item_spacing;
                                ui.style_mut().spacing.item_spacing.y = 0.0;

                                // Separator between header and first card.
                                {
                                    let (gap_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(card_width, card_gap),
                                        egui::Sense::hover(),
                                    );
                                    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
                                    ui.painter().rect_filled(gap_rect, 0.0, stroke.color);
                                }

                                runtime.sync_len(notebook.cards.len());
                                let store = notebook.state_store.clone();
                                let cards_len = notebook.cards.len();
                                for (i, entry) in notebook.cards.iter_mut().enumerate() {
                                    let card_identity = entry.identity;
                                    runtime.ensure_card_identity(i, card_identity);
                                    let card_detached = runtime
                                        .card_detached
                                        .get_mut(i)
                                        .expect("card_detached synced to cards");
                                    let card_placeholder_size = runtime
                                        .card_placeholder_sizes
                                        .get_mut(i)
                                        .expect("card_placeholder_sizes synced to cards");
                                    ui.push_id((i, card_identity), |ui| {
                                        let card_left = column_rect.min.x;
                                        let card: &mut dyn cards::Card = entry.card.as_mut();
                                        let card_rect = if *card_detached {
                                            let placeholder_height =
                                                crate::card_ctx::GRID_ROW_MODULE;
                                            let placeholder_width = card_width;
                                            let (rect, resp) = ui.allocate_exact_size(
                                                egui::vec2(placeholder_width, placeholder_height),
                                                egui::Sense::click(),
                                            );
                                            let fill = ui.visuals().window_fill;
                                            let outline =
                                                ui.visuals().widgets.noninteractive.bg_stroke.color;
                                            ui.painter().rect_filled(rect, 0.0, fill);
                                            paint_hatching(
                                                &ui.painter().with_clip_rect(rect),
                                                rect,
                                                outline,
                                            );
                                            show_postit_tooltip(ui, &resp, "Dock card");
                                            if resp.clicked() {
                                                *card_detached = false;
                                            }

                                            if *card_detached {
                                                let initial_screen_pos = egui::pos2(
                                                    right_margin.min.x + 12.0,
                                                    rect.top(),
                                                );
                                                let detached_id = ui.id().with("detached_card");
                                                let float_resp = floating::show_floating_card(
                                                    ui.ctx(),
                                                    detached_id,
                                                    initial_screen_pos,
                                                    card_width,
                                                    card_placeholder_size.y,
                                                    store.as_ref(),
                                                    "Dock card",
                                                    &mut |ctx| {
                                                        #[cfg(feature = "telemetry")]
                                                        let _detached_span =
                                                            tracing::info_span!("detached_draw")
                                                                .entered();
                                                        card.draw(ctx);
                                                    },
                                                );
                                                if float_resp.handle_clicked {
                                                    *card_detached = false;
                                                }
                                            }
                                            rect
                                        } else {
                                            let clip_rect = ui.clip_rect();
                                            let card_clip_rect = egui::Rect::from_min_max(
                                                egui::pos2(column_rect.min.x, clip_rect.min.y),
                                                egui::pos2(column_rect.max.x, clip_rect.max.y),
                                            );
                                            #[cfg(feature = "telemetry")]
                                            let _card_span = {
                                                let source = entry
                                                    .source
                                                    .as_ref()
                                                    .map(|s| s.file_line_column())
                                                    .unwrap_or_default();
                                                tracing::info_span!(
                                                    "card",
                                                    source = source.as_str()
                                                )
                                                .entered()
                                            };
                                            let inner_rect = draw_card_body(
                                                ui,
                                                card_width,
                                                card,
                                                store.as_ref(),
                                                Some(card_clip_rect),
                                            );
                                            *card_placeholder_size =
                                                egui::vec2(card_width, inner_rect.height());
                                            egui::Rect::from_min_size(
                                                egui::pos2(card_left, inner_rect.min.y),
                                                egui::vec2(card_width, inner_rect.height()),
                                            )
                                        };
                                        if i + 1 < cards_len {
                                            let separator_top = card_rect.bottom().ceil();
                                            let cursor_top = ui.cursor().top();
                                            if separator_top > cursor_top {
                                                ui.add_space(separator_top - cursor_top);
                                            }
                                            let (gap_rect, _) = ui.allocate_exact_size(
                                                egui::vec2(card_width, card_gap),
                                                egui::Sense::hover(),
                                            );
                                            let stroke =
                                                ui.visuals().widgets.noninteractive.bg_stroke;
                                            ui.painter().rect_filled(gap_rect, 0.0, stroke.color);
                                        }

                                        let show_detach_button = !*card_detached;
                                        #[cfg(not(target_arch = "wasm32"))]
                                        let show_open_button = show_detach_button
                                            && entry.source.is_some()
                                            && config.editor.is_some();
                                        #[cfg(target_arch = "wasm32")]
                                        let show_open_button = false;
                                        if show_detach_button {
                                            let tab_size = egui::vec2(
                                                20.0,
                                                2.0 * crate::card_ctx::GRID_ROW_MODULE,
                                            );
                                            let tab_pull = 4.0;
                                            let base_tab_gap = 4.0;
                                            let base_top_offset = 8.0;
                                            let min_top_offset = 0.0;
                                            let min_visible = tab_size.y * 0.4;
                                            let tab_fill = crate::themes::GorbieButtonStyle::from(
                                                ui.style().as_ref(),
                                            )
                                            .fill;

                                            let tab_count = 1 + usize::from(show_open_button);
                                            let available = card_rect.height().max(0.0);
                                            let mut top_offset = base_top_offset;
                                            let mut gap = base_tab_gap;
                                            let required = top_offset
                                                + tab_size.y * tab_count as f32
                                                + gap * (tab_count.saturating_sub(1) as f32);

                                            if required > available {
                                                let extra = required - available;
                                                let max_top_reduce =
                                                    (top_offset - min_top_offset).max(0.0);
                                                let top_reduce = extra.min(max_top_reduce);
                                                top_offset -= top_reduce;
                                                let remaining = extra - top_reduce;

                                                if remaining > 0.0 && tab_count > 1 {
                                                    let min_gap = -(tab_size.y - min_visible);
                                                    let max_gap_reduce = (gap - min_gap).max(0.0);
                                                    let gap_reduce = remaining.min(max_gap_reduce);
                                                    gap -= gap_reduce;
                                                }
                                            }

                                            let tab_x = card_rect.right().round();
                                            let top_y = (card_rect.top() + top_offset).round();
                                            let detach_pos = egui::pos2(tab_x, top_y);
                                            #[allow(unused_variables)]
                                            let open_pos = show_open_button.then(|| {
                                                egui::pos2(
                                                    tab_x,
                                                    (top_y + tab_size.y + gap).round(),
                                                )
                                            });

                                            ui.push_id((i, card_identity), |ui| {
                                                #[cfg(not(target_arch = "wasm32"))]
                                                if let Some(open_pos) = open_pos {
                                                    let open_id = ui.id().with("open_button");
                                                    let open_area = egui::Area::new(open_id)
                                                        .order(egui::Order::Middle)
                                                        .fixed_pos(open_pos)
                                                        .movable(false)
                                                        .constrain_to(egui::Rect::EVERYTHING);
                                                    let open_resp =
                                                        open_area.show(ui.ctx(), |ui| {
                                                            let (rect, resp) = ui
                                                                .allocate_exact_size(
                                                                    egui::vec2(
                                                                        tab_size.x + tab_pull,
                                                                        tab_size.y,
                                                                    ),
                                                                    egui::Sense::click(),
                                                                );
                                                            let tab_rect =
                                                                egui::Rect::from_min_size(
                                                                    rect.min, tab_size,
                                                                );
                                                            paint_card_tab_button(
                                                                ui, &resp, tab_rect, "<>",
                                                                tab_fill, tab_pull,
                                                            );

                                                            if let Some(source) =
                                                                entry.source.as_ref()
                                                            {
                                                                let file = &source.file;
                                                                let line = source.line;
                                                                let tooltip = format!(
                                                                    "Open in editor\n{file}:{line}"
                                                                );
                                                                show_postit_tooltip(
                                                                    ui, &resp, &tooltip,
                                                                );
                                                            } else {
                                                                show_postit_tooltip(
                                                                    ui,
                                                                    &resp,
                                                                    "Open in editor",
                                                                );
                                                            }
                                                            resp
                                                        });

                                                    #[cfg(not(target_arch = "wasm32"))]
                                                    if open_resp.inner.clicked() {
                                                        if let (Some(source), Some(editor)) = (
                                                            entry.source.as_ref(),
                                                            config.editor.as_ref(),
                                                        ) {
                                                            if let Err(err) = editor.open(source) {
                                                                log::warn!(
                                                                    "failed to open editor: {err}"
                                                                );
                                                            }
                                                        }
                                                    }
                                                }

                                                let detach_id = ui.id().with("detach_button");
                                                let detach_area = egui::Area::new(detach_id)
                                                    .order(egui::Order::Middle)
                                                    .fixed_pos(detach_pos)
                                                    .movable(false)
                                                    .constrain_to(egui::Rect::EVERYTHING);
                                                let detach_resp =
                                                    detach_area.show(ui.ctx(), |ui| {
                                                        let (rect, resp) = ui.allocate_exact_size(
                                                            egui::vec2(
                                                                tab_size.x + tab_pull,
                                                                tab_size.y,
                                                            ),
                                                            egui::Sense::click(),
                                                        );
                                                        let tab_rect = egui::Rect::from_min_size(
                                                            rect.min, tab_size,
                                                        );
                                                        paint_card_tab_button(
                                                            ui, &resp, tab_rect, "[]", tab_fill,
                                                            tab_pull,
                                                        );

                                                        let tooltip = if *card_detached {
                                                            "Dock card"
                                                        } else {
                                                            "Detach card"
                                                        };
                                                        show_postit_tooltip(ui, &resp, tooltip);
                                                        resp
                                                    });

                                                if detach_resp.inner.clicked() {
                                                    *card_detached = !*card_detached;
                                                }
                                            });
                                        }
                                    });
                                }

                                ui.style_mut().spacing.item_spacing = default_item_spacing;
                            });
                        ui.set_clip_rect(restore_clip_rect);
                        let frame_rect = column_frame.response.rect;
                        let frame_rect = egui::Rect::from_min_max(
                            egui::pos2(column_rect.min.x, frame_rect.min.y),
                            egui::pos2(column_rect.max.x, frame_rect.max.y),
                        );
                        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
                        ui.painter()
                            .rect_stroke(frame_rect, 0.0, stroke, egui::StrokeKind::Inside);

                        // Extend scroll content to include content-anchored
                        // floating cards. The comparison is in *heights*,
                        // not absolute positions — `frame_rect.bottom()`
                        // is in screen coords (drifts with scroll), while
                        // `float_bottom` (= fstate.pos.y + card_h) is in
                        // content coords (fixed). Subtracting the two
                        // mixes coord systems and grows linearly with
                        // scroll position → runaway scroll content.
                        // Compare heights instead: the column's inline
                        // content occupies content_y = [0, frame_h]; the
                        // float ends at content_y = float_bottom; we
                        // extend by the gap.
                        let inline_height = frame_rect.height();
                        let float_bottom = floating::max_float_content_bottom(ui.ctx());
                        if float_bottom > inline_height {
                            ui.allocate_space(egui::vec2(0.0, float_bottom - inline_height));
                        }
                    });
                });
        });

        // Notebook-wide search bar — opt-in via `CardCtx::current_search`.
        // Only renders if at least one widget requested search within
        // the last couple of frames.
        search::render_bar(&ctx);

        ctx.data_mut(|data| {
            data.insert_temp(state_id, runtime);
        });
    }
}

fn draw_card_body(
    ui: &mut egui::Ui,
    card_width: f32,
    card: &mut dyn cards::Card,
    store: &state::StateStore,
    clip_rect: Option<egui::Rect>,
) -> egui::Rect {
    // Clamp *all* painting (including the frame fill) to the column rect.
    // Without this, a too-wide widget can expand the frame and paint into the
    // notebook margins, covering the dot grid.
    let restore_clip = clip_rect.map(|rect| {
        let restore = ui.clip_rect();
        ui.set_clip_rect(rect);
        restore
    });

    let inner = egui::Frame::group(ui.style())
        .stroke(egui::Stroke::NONE)
        .corner_radius(0.0)
        .inner_margin(egui::Margin::ZERO)
        .show(ui, |ui| {
            ui.reset_style();
            ui.set_width(card_width);
            let mut ctx = CardCtx::new(ui, store);
            card.draw(&mut ctx);
        });

    if let Some(restore) = restore_clip {
        ui.set_clip_rect(restore);
    }
    inner.response.rect
}

/// Réseau fiducials in the page margin.
///
/// An Apollo Hasselblad carried a glass réseau plate in front of the film,
/// etched with a sparse grid of crosses. Every frame came back marked, so the
/// photograph could be measured and the measurement trusted. A dot field says
/// "graph paper — you may draw here"; a réseau says "this has been measured",
/// which is the truer thing for a notebook whose whole argument is receipts.
///
/// Sparser than the dot grid it replaces, and on the layout's column pitch
/// (`GRID_COL_WIDTH + GRID_GUTTER`) rather than a pitch of its own, so the
/// workspace shares the page's rhythm. Note the pitch is shared but the phase
/// is not: the lattice is anchored to a global multiple of `pitch_x` plus a
/// half-pitch, not to the column origin, so a mark is not guaranteed to sit on
/// a column boundary. Rhythm, not registration.
fn paint_reseau_grid(ui: &egui::Ui, rect: egui::Rect, scroll_y: f32) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }

    let painter = ui.painter_at(rect);

    // Two column pitches across, twelve modules down — half the density of the
    // first pass, so the marks read as landmarks rather than texture. The arm is
    // half a row module, making each cross exactly one module wide.
    let pitch_x = 2.0 * (crate::card_ctx::GRID_COL_WIDTH + crate::card_ctx::GRID_GUTTER);
    let pitch_y = crate::card_ctx::GRID_ROW_MODULE * 12.0;
    let arm = crate::card_ctx::GRID_ROW_MODULE / 2.0;

    let background = ui.visuals().window_fill;
    let outline = ui.visuals().widgets.noninteractive.bg_stroke.color;
    // A cross carries more ink than a dot, so it sits further back than the
    // 0.35 the dot field used or it stops being quiet.
    let color = crate::themes::blend(background, outline, 0.28);
    let stroke = egui::Stroke::new(1.0, color);

    let start_x = (rect.left() / pitch_x).floor() * pitch_x + pitch_x / 2.0;
    let start_y = rect.top() - scroll_y.rem_euclid(pitch_y) + pitch_y / 2.0;

    let mut y = start_y;
    while y < rect.bottom() + arm {
        let mut x = start_x;
        while x < rect.right() + arm {
            painter.line_segment([egui::pos2(x - arm, y), egui::pos2(x + arm, y)], stroke);
            painter.line_segment([egui::pos2(x, y - arm), egui::pos2(x, y + arm)], stroke);
            x += pitch_x;
        }
        y += pitch_y;
    }
}

fn paint_hatching(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let spacing = 12.0;
    let stroke = egui::Stroke::new(1.0, color);

    let h = rect.height();
    let mut x = rect.left() - h;
    while x < rect.right() + h {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x + h, rect.bottom())],
            stroke,
        );
        x += spacing;
    }
}

pub(crate) fn show_postit_tooltip(ui: &egui::Ui, response: &egui::Response, text: &str) {
    let outline = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let shadow_color = crate::themes::ral(9004);
    let shadow = egui::epaint::Shadow {
        offset: [4, 4],
        blur: 0,
        spread: 0,
        color: shadow_color,
    };

    let frame = egui::Frame::new()
        .fill(crate::themes::ral(1003))
        .stroke(egui::Stroke::new(1.0, outline))
        .shadow(shadow)
        .corner_radius(0.0)
        .inner_margin(egui::Margin::same(10));

    let mut tooltip = egui::containers::Tooltip::for_enabled(response);
    tooltip.popup = tooltip.popup.frame(frame);
    tooltip.show(|ui| {
        ui.set_max_width(ui.spacing().tooltip_width);
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .monospace()
                    .color(crate::themes::ral(9011)),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
    });
}

fn paint_card_tab_button(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    label: &str,
    fill: egui::Color32,
    pull_out: f32,
) {
    let outline = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let stroke = egui::Stroke::new(1.0, outline);
    let rounding = egui::CornerRadius {
        nw: 0,
        ne: 4,
        sw: 0,
        se: 4,
    };
    let rect = if response.hovered() || response.has_focus() {
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x + pull_out, rect.max.y))
    } else {
        rect
    };

    ui.painter().rect_filled(rect, rounding, fill);
    ui.painter()
        .rect_stroke(rect, rounding, stroke, egui::StrokeKind::Inside);
    let text_color = if response.enabled() {
        crate::themes::ral(9011)
    } else {
        crate::themes::blend(crate::themes::ral(9011), fill, 0.55)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::monospace(10.0),
        text_color,
    );
}

// notebook initialization is handled by the #[notebook] attribute macro.

#[cfg(test)]
mod notebook_state_tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn state_initializes_once_across_notebook_rebuilds() {
        let mut core = NotebookCore::new(NotebookConfig::new("lazy-state-test"), Box::new(|_| {}));
        let initializations = Cell::new(0);

        let first = {
            let mut notebook = core.build_notebook();
            notebook.state(
                &"shared",
                || {
                    initializations.set(initializations.get() + 1);
                    41_u32
                },
                |_, _| {},
            )
        };
        let (second, value) = {
            let mut notebook = core.build_notebook();
            let state = notebook.state(
                &"shared",
                || {
                    initializations.set(initializations.get() + 1);
                    99_u32
                },
                |_, _| {},
            );
            let value = *state.read(&notebook);
            (state, value)
        };

        assert_eq!(first, second);
        assert_eq!(value, 41);
        assert_eq!(initializations.get(), 1);
    }

    #[test]
    fn state_retains_a_non_copy_resource_from_a_consuming_initializer() {
        struct Resource { text: String }
        let mut core = NotebookCore::new(NotebookConfig::new("resource-state-test"), Box::new(|_| {}));
        let initializations = Cell::new(0);
        let mut previous = None;
        for label in ["first", "must not replace"] {
            let owned = Box::new(label.to_owned());
            let count = &initializations;
            let mut notebook = core.build_notebook();
            let state = notebook.state("resource", move || {
                count.set(count.get() + 1);
                Resource { text: *owned }
            }, |_, _| {});
            if let Some(previous) = previous { assert_eq!(state.id(), previous); }
            else { state.read_mut(&notebook).text.push_str(" retained"); }
            previous = Some(state.id());
            assert_eq!(state.read(&notebook).text, "first retained");
        }
        assert_eq!(initializations.get(), 1);
    }

    #[test]
    fn state_keys_are_independent_and_type_mismatch_does_not_initialize() {
        let mut core = NotebookCore::new(NotebookConfig::new("key-state-test"), Box::new(|_| {}));
        let mut notebook = core.build_notebook();
        let left = notebook.state("left", || 41_u32, |_, _| {});
        let right = notebook.state("right", || String::from("other type"), |_, _| {});
        assert_ne!(left.id(), right.id());
        assert_eq!(*left.read(&notebook), 41);
        assert_eq!(right.read(&notebook).as_str(), "other type");
        let initialized = Cell::new(false);
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            notebook.state("left", || { initialized.set(true); String::new() }, |_, _| {});
        }));
        assert!(failure.is_err());
        assert!(!initialized.get());
        assert_eq!(*left.read(&notebook), 41);
    }

    #[test]
    fn state_preserves_caller_location_and_card_identity_across_rebuilds() {
        fn draw(_: &mut CardCtx<'_>, _: &mut String) {}
        fn declare(notebook: &mut NotebookCtx, init: impl FnOnce() -> String) -> (state::StateId<String>, u32) {
            let line = line!() + 1;
            let state = notebook.state("stable", init, draw);
            (state, line)
        }
        let mut core = NotebookCore::new(NotebookConfig::new("identity-state-test"), Box::new(|_| {}));
        let (first, source, identity) = {
            let mut notebook = core.build_notebook();
            let (state, line) = declare(&mut notebook, || String::from("retained"));
            let source = notebook.cards[0].source.clone().unwrap();
            assert_eq!(source.file, file!());
            assert_eq!(source.line, line);
            (state, source, notebook.cards[0].identity)
        };
        let mut notebook = core.build_notebook();
        let (second, _) = declare(&mut notebook, || panic!("existing key must not initialize"));
        assert_eq!(first, second);
        assert_eq!(notebook.cards[0].source.as_ref(), Some(&source));
        assert_eq!(notebook.cards[0].identity, identity);
        assert_eq!(second.read(&notebook).as_str(), "retained");
    }

    #[test]
    fn stateful_card_helper_is_lazy_and_forwards_the_call_site() {
        let mut core = NotebookCore::new(NotebookConfig::new("helper-state-test"), Box::new(|_| {}));
        let mut notebook = core.build_notebook();
        let line = line!() + 1;
        let first = cards::stateful_card(&mut notebook, "shared", || String::from("first"), |_, _| {});
        assert_eq!(notebook.cards[0].source.as_ref().unwrap().line, line);
        let second = cards::stateful_card(&mut notebook, "shared", || panic!("not lazy"), |_, _: &mut String| {});
        assert_eq!(first, second);
        assert_eq!(first.read(&notebook).as_str(), "first");
    }
}
