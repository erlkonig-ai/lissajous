use std::hash::Hash;
use std::ops::{Deref, DerefMut};

use eframe::egui;

use crate::state;

/// Number of columns in the notebook grid.
pub const GRID_COLUMNS: u32 = 12;

/// Gutter width between grid columns, in pixels.
pub const GRID_GUTTER: f32 = 12.0;

/// Vertical module height for the modular grid, in pixels.
///
/// Cell heights are rounded up to the next multiple of this value, so row
/// tops always land on a module-aligned baseline (relative to the grid
/// origin). Matches the horizontal gutter so the grid is truly modular
/// (square gutters).
pub const GRID_ROW_MODULE: f32 = 12.0;

/// Edge padding of the grid, in pixels.
///
/// The grid has one gutter-width of padding on each side, so the content
/// area is `768 - 2 * GRID_EDGE_PAD = 744px`.
pub const GRID_EDGE_PAD: f32 = GRID_GUTTER;

/// Width of a single grid column, in pixels.
///
/// Derived from the notebook column width (768px), edge padding, grid
/// column count, and gutter:
/// `(768 - 2 * 12 - (GRID_COLUMNS - 1) * GRID_GUTTER) / GRID_COLUMNS = 51`.
pub const GRID_COL_WIDTH: f32 = 51.0;

/// Pixel width of a span of `n` grid columns (including internal gutters).
pub const fn span_width(n: u32) -> f32 {
    n as f32 * GRID_COL_WIDTH + n.saturating_sub(1) as f32 * GRID_GUTTER
}

/// Pixel height of `n` vertical grid modules.
///
/// The vertical twin of [`span_width`]. Use it instead of a hand-picked pixel
/// height: a row pinned to `20.0` was sized against one body size and starts
/// clipping the moment the type scale moves, whereas a module count tracks the
/// grid the page is built on.
pub const fn module_height(n: u32) -> f32 {
    n as f32 * GRID_ROW_MODULE
}

/// Context-data key for the notebook-wide section default set by
/// [`set_default_section_open`].
fn section_default_open_id() -> egui::Id {
    egui::Id::new("gorbie_default_section_open")
}

/// Set whether [`CardCtx::section`] starts open (the built-in default)
/// or collapsed, notebook-wide.
///
/// Call once — e.g. at the top of the first card's closure — before any
/// sections render. Only affects each section's FIRST appearance: once
/// the user toggles a section, their choice is persisted per title and
/// wins over this default. [`CardCtx::section_collapsed`] is unaffected
/// (it always defaults closed).
///
/// ```ignore
/// // Start every section collapsed for a dashboard-style notebook:
/// lissajous::card_ctx::set_default_section_open(ui.ctx(), false);
/// ```
pub fn set_default_section_open(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|d| d.insert_temp(section_default_open_id(), open));
}

/// Card-scoped context that threads the state store through all nested layouts.
///
/// `CardCtx` wraps an `egui::Ui` and provides layout methods that pass
/// `&mut CardCtx` (rather than raw `&mut egui::Ui`) to closures, so
/// [`StateId::read`](crate::state::StateId::read) and
/// [`StateId::read_mut`](crate::state::StateId::read_mut) work at any nesting
/// depth.
///
/// All layout methods shadow their `egui::Ui` counterparts. Because owned
/// methods take priority over `Deref` methods, calling `ctx.horizontal(…)`
/// always invokes the GORBIE version automatically.
pub struct CardCtx<'a> {
    ui: &'a mut egui::Ui,
    store: &'a state::StateStore,
}

impl<'a> CardCtx<'a> {
    pub(crate) fn new(ui: &'a mut egui::Ui, store: &'a state::StateStore) -> Self {
        Self { ui, store }
    }

    /// Returns a reference to the backing state store.
    pub fn store(&self) -> &state::StateStore {
        self.store
    }

    /// Access the underlying `egui::Ui` directly.
    ///
    /// Prefer the `CardCtx` layout methods — this escape hatch is for
    /// egui APIs that `CardCtx` does not yet wrap.
    pub fn ui_mut(&mut self) -> &mut egui::Ui {
        self.ui
    }

    // ── Grid-aware widgets ──────────────────────────────────────────

    /// Add a GORBIE button.
    ///
    /// This shadows `egui::Ui::button` via method resolution, so
    /// `ctx.button("text")` always produces a GORBIE button.
    pub fn button(&mut self, text: impl Into<egui::WidgetText>) -> egui::Response {
        self.ui.add(crate::widgets::Button::new(text))
    }

    /// Open a search session against the notebook-wide search bar.
    ///
    /// Calling this during render is what makes the bar appear — when
    /// no widget opens a session, the bar disappears. Read the current
    /// query via [`crate::search::SearchSession::query`] and report
    /// matches via [`crate::search::SearchSession::report`] so the bar
    /// can show counts and let the user navigate prev/next.
    pub fn search(&mut self) -> crate::search::SearchSession {
        crate::search::new_session(self.ui.ctx().clone())
    }

    /// GORBIE-styled toggle button (checkbox-like, latching on/off).
    pub fn toggle(&mut self, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
        self.ui.add(crate::widgets::Button::new(text).on(on))
    }

    /// GORBIE-styled slider.
    pub fn slider<Num: egui::emath::Numeric>(
        &mut self,
        value: &mut Num,
        range: std::ops::RangeInclusive<Num>,
    ) -> egui::Response {
        self.ui.add(crate::widgets::Slider::new(value, range))
    }

    /// GORBIE-styled number field (LCD-style drag/edit).
    pub fn number<Num: egui::emath::Numeric>(&mut self, value: &mut Num) -> egui::Response {
        self.ui.add(crate::widgets::NumberField::new(value))
    }

    /// GORBIE-styled single-line text field (LCD-style).
    pub fn text_field(&mut self, text: &mut dyn egui::TextBuffer) -> egui::Response {
        self.ui.add(crate::widgets::TextField::singleline(text))
    }

    /// GORBIE-styled progress bar.
    pub fn progress(&mut self, fraction: f32) -> egui::Response {
        self.ui.add(crate::widgets::ProgressBar::new(fraction))
    }

    /// Render markdown with GORBIE styling and syntax themes.
    #[cfg(feature = "markdown")]
    pub fn markdown(&mut self, text: &str) {
        crate::widgets::markdown(self.ui, text);
    }

    /// Render a full Typst document string with GORBIE styling (RAL colors,
    /// IosevkaGorbie font, page width matching the card, blue links).
    #[cfg(feature = "typst")]
    pub fn typst(&mut self, source: &str) {
        crate::widgets::typst_widget::typst_with_preamble(self.ui, source);
    }

    /// Render an inline math expression via Typst: `$<expr>$`.
    #[cfg(feature = "typst")]
    pub fn typst_math_inline(&mut self, expr: &str) {
        crate::widgets::typst_widget::typst_math_inline(self.ui, expr);
    }

    /// Render a display-mode math expression via Typst.
    #[cfg(feature = "typst")]
    pub fn typst_math_display(&mut self, expr: &str) {
        crate::widgets::typst_widget::typst_math_display(self.ui, expr);
    }

    // ── Environment ──────────────────────────────────────────────────

    /// True when this render pass is driven by the headless capture
    /// renderer (`--headless`). Widgets can consult this to adapt for
    /// screenshots; [`section`](Self::section) already forces sections
    /// open under it. Convenience for [`crate::is_headless`] — the
    /// underlying marker lives on the `egui::Context`.
    pub fn is_headless(&self) -> bool {
        crate::is_headless(self.ui.ctx())
    }

    /// Set whether [`section`](Self::section) starts open (the
    /// built-in default) or collapsed, notebook-wide. Convenience for
    /// [`set_default_section_open`] — call from any card before the
    /// sections render (the first card of the notebook is the natural
    /// spot). Headless captures ignore this and always render open.
    pub fn set_default_section_open(&self, open: bool) {
        set_default_section_open(self.ui.ctx(), open);
    }

    // ── Section ──────────────────────────────────────────────────────

    /// Collapsible section with a bold colored header (open by default).
    ///
    /// The header color is deterministically assigned from the title via
    /// [`colorhash::ral_categorical`], like colored divider tabs in stationery.
    /// Click to expand/collapse.
    ///
    /// A section is a top-level component of a card: put it at the top of
    /// the card body and nest the layout inside it, a [`grid`](Self::grid)
    /// or one monolithic full-width widget. Never place a section inside a
    /// grid cell, whose inset gives the header a border and margin it is not
    /// designed for, and use one section per card, because a card is the
    /// unit the reader can detach and move around the workspace.
    ///
    /// The "open by default" half can be flipped notebook-wide with
    /// [`set_default_section_open`] — useful for dashboards composing
    /// many heavy sections where starting collapsed keeps the initial
    /// view scannable. A user's persisted open/closed choice always
    /// wins over either default.
    ///
    /// ```ignore
    /// ctx.section("Parameters", |ctx| {
    ///     ctx.text_field(&mut name);
    ///     ctx.number(&mut value);
    /// });
    /// ```
    pub fn section(&mut self, title: &str, add_contents: impl FnOnce(&mut CardCtx<'_>)) {
        let default_open = self
            .ui
            .ctx()
            .data(|d| d.get_temp::<bool>(section_default_open_id()))
            .unwrap_or(true);
        self.section_inner(title, default_open, add_contents);
    }

    /// Collapsible section that starts collapsed (closed by default).
    ///
    /// Identical to [`section`](Self::section) except the initial state is
    /// collapsed. Once the user clicks to expand, their preference is
    /// persisted just like a regular section.
    ///
    /// ```ignore
    /// ctx.section_collapsed("Advanced", |ctx| {
    ///     ctx.label("Hidden until clicked.");
    /// });
    /// ```
    pub fn section_collapsed(&mut self, title: &str, add_contents: impl FnOnce(&mut CardCtx<'_>)) {
        self.section_inner(title, false, add_contents);
    }

    /// Shared implementation for [`section`](Self::section) and
    /// [`section_collapsed`](Self::section_collapsed).
    fn section_inner(
        &mut self,
        title: &str,
        default_open: bool,
        add_contents: impl FnOnce(&mut CardCtx<'_>),
    ) {
        use crate::themes::colorhash;

        // Headless captures always render sections open — a collapsed
        // section screenshots as a bare header bar, which defeats the
        // point of capturing. Overrides both the notebook-wide default
        // and `section_collapsed`.
        let default_open = default_open || crate::is_headless(self.ui.ctx());

        let id = self.ui.make_persistent_id(title);
        let mut open = self
            .ui
            .ctx()
            .data_mut(|d| *d.get_persisted_mut_or(id, default_open));

        let color = colorhash::ral_categorical(title.as_bytes());
        let text_color = colorhash::text_color_on(color);

        // Zero spacing so header and folds sit flush.
        let prev_spacing = self.ui.spacing().item_spacing.y;
        self.ui.spacing_mut().item_spacing.y = 0.0;

        // Header bar: full width, colored background, clickable.
        let header_height = GRID_ROW_MODULE * 5.0;
        let available_width = self.ui.available_width();
        let (header_rect, header_response) = self.ui.allocate_exact_size(
            egui::vec2(available_width, header_height),
            egui::Sense::click(),
        );

        if header_response.clicked() {
            open = !open;
            self.ui.ctx().data_mut(|d| d.insert_persisted(id, open));
        }

        // Paint the header.
        let painter = self.ui.painter();
        painter.rect_filled(header_rect, 0.0, color);

        // Title text — large, centered vertically, left-aligned with padding.
        let text_pos = egui::pos2(
            header_rect.left() + GRID_EDGE_PAD,
            header_rect.bottom() - GRID_ROW_MODULE,
        );
        painter.text(
            text_pos,
            egui::Align2::LEFT_BOTTOM,
            title,
            egui::FontId::proportional(header_height * 0.45),
            text_color,
        );

        if header_response.hovered() {
            self.ui
                .ctx()
                .set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        self.ui.spacing_mut().item_spacing.y = prev_spacing;
        if open {
            add_contents(self);
        }
    }

    // ── Layout wrappers ──────────────────────────────────────────────

    /// Apply padding around the contents while preserving `CardCtx`.
    ///
    /// This is the `CardCtx`-aware equivalent of [`cards::with_padding`].
    pub fn with_padding<R>(
        &mut self,
        padding: impl Into<egui::Margin>,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        egui::Frame::new()
            .inner_margin(padding)
            .show(self.ui, |ui| {
                ui.set_width(ui.available_width());
                let mut ctx = CardCtx::new(ui, store);
                add_contents(&mut ctx)
            })
    }

    /// Horizontal layout, passing `CardCtx` instead of raw `Ui`.
    pub fn horizontal<R>(
        &mut self,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.horizontal(|ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Horizontal-wrapped layout, passing `CardCtx` instead of raw `Ui`.
    pub fn horizontal_wrapped<R>(
        &mut self,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.horizontal_wrapped(|ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Vertical layout, passing `CardCtx` instead of raw `Ui`.
    pub fn vertical<R>(
        &mut self,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.vertical(|ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Custom `egui::Layout`, passing `CardCtx` instead of raw `Ui`.
    pub fn with_layout<R>(
        &mut self,
        layout: egui::Layout,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.with_layout(layout, |ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Push a unique id salt, passing `CardCtx` instead of raw `Ui`.
    pub fn push_id<R>(
        &mut self,
        id_salt: impl Hash,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.push_id(id_salt, |ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Collapsing header, passing `CardCtx` instead of raw `Ui`.
    pub fn collapsing<R>(
        &mut self,
        heading: impl Into<egui::WidgetText>,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::CollapsingResponse<R> {
        let store = self.store;
        self.ui.collapsing(heading, |ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// New visual scope, passing `CardCtx` instead of raw `Ui`.
    pub fn scope<R>(
        &mut self,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.scope(|ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Indented region, passing `CardCtx` instead of raw `Ui`.
    pub fn indent<R>(
        &mut self,
        id_salt: impl Hash,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.indent(id_salt, |ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    /// Visual group with frame, passing `CardCtx` instead of raw `Ui`.
    pub fn group<R>(
        &mut self,
        add_contents: impl FnOnce(&mut CardCtx<'_>) -> R,
    ) -> egui::InnerResponse<R> {
        let store = self.store;
        self.ui.group(|ui| {
            let mut ctx = CardCtx::new(ui, store);
            add_contents(&mut ctx)
        })
    }

    // ── Grid layout ──────────────────────────────────────────────────

    /// Place elements on the 12-column grid.
    ///
    /// The closure receives a [`Grid`] with [`place`](Grid::place) and
    /// [`skip`](Grid::skip) methods. Elements flow left-to-right and
    /// wrap to the next row automatically when spans would exceed 12.
    ///
    /// Pixel widths are derived entirely from constants — same spans
    /// produce identical widths in every card (coordination-free).
    ///
    /// ```ignore
    /// ctx.grid(|g| {
    ///     g.place(12, |ctx| { ctx.heading("Title"); });     // full width
    ///     g.place(3, |ctx| { ctx.paragraph("text"); });     // 3 cols
    ///     g.place(9, |ctx| { ctx.image(path); });           // fills row
    ///     g.place(6, |ctx| { chart(ctx); });                // next row
    ///     g.skip(6);                                        // furniture
    /// });
    /// ```
    pub fn grid(&mut self, build: impl FnOnce(&mut Grid<'_, '_>)) {
        let store = self.store;
        let grid_id = self.ui.id().with("gorbie_grid");
        let left = self.ui.cursor().min.x + GRID_EDGE_PAD;
        let top = self.ui.cursor().min.y + GRID_EDGE_PAD;
        let mut g = Grid {
            ui: self.ui,
            store,
            grid_id,
            left,
            cursor: 0,
            row_index: 0,
            row_top: top,
            row_max_bottom: top,
        };
        build(&mut g);
        // Advance the parent Ui past all rows we painted.
        g.finish();
    }
}

/// Response from [`CardCtx::float`].
pub struct FloatResponse {
    /// `true` if the user clicked the drag handle to dismiss the float.
    pub closed: bool,
}

impl<'a> CardCtx<'a> {
    /// Spawn a floating card at the current mouse position (on first show).
    ///
    /// The float renders as a GORBIE card with drag handle and shadow, hovering
    /// above the notebook. Clicking the drag handle dismisses it (returns
    /// `closed = true`). Dragging the handle repositions it.
    ///
    /// Use [`push_id`](Self::push_id) to give each float a unique identity
    /// when creating multiple floats in a loop.
    ///
    /// ```ignore
    /// for page in &state.pages {
    ///     ctx.push_id(page.id, |ctx| {
    ///         let resp = ctx.float(|ctx| {
    ///             ctx.markdown(&page.content);
    ///         });
    ///         if resp.closed {
    ///             close_page(page.id);
    ///         }
    ///     });
    /// }
    /// ```
    #[track_caller]
    pub fn float(&mut self, add_contents: impl FnOnce(&mut CardCtx<'_>)) -> FloatResponse {
        let float_id = self.ui.id().with("gorbie_float");
        let initial_pos = self
            .ui
            .ctx()
            .input(|i| i.pointer.hover_pos().unwrap_or(egui::pos2(100.0, 100.0)));

        let card_width = crate::NOTEBOOK_COLUMN_WIDTH;
        let store = self.store;
        let mut add_contents = Some(add_contents);

        let resp = crate::floating::show_floating_card(
            self.ui.ctx(),
            float_id,
            initial_pos,
            card_width,
            0.0,
            store,
            "Close",
            &mut |ctx| {
                if let Some(f) = add_contents.take() {
                    f(ctx);
                }
            },
        );

        FloatResponse {
            closed: resp.handle_clicked,
        }
    }
}

impl<'a> state::StateAccess for CardCtx<'a> {
    fn store(&self) -> &state::StateStore {
        self.store
    }
}

impl<'a> Deref for CardCtx<'a> {
    type Target = egui::Ui;

    fn deref(&self) -> &Self::Target {
        self.ui
    }
}

impl<'a> DerefMut for CardCtx<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ui
    }
}

// ── Grid ─────────────────────────────────────────────────────────────

/// Flat grid layout for a card.
///
/// Created by [`CardCtx::grid`]. Call [`place`](Self::place) to add
/// content spanning grid columns, [`skip`](Self::skip) to add furniture.
///
/// Elements flow left-to-right. When a placement would exceed 12
/// columns, the current row is finalized and a new one begins.
///
/// Each cell is rendered immediately into a precisely positioned rect —
/// no closure buffering, no lifetime constraints beyond the current call.
pub struct Grid<'ui, 'store> {
    ui: &'ui mut egui::Ui,
    store: &'store state::StateStore,
    /// Stable identifier for this grid, used as a prefix for per-cell
    /// frame-delayed sizing state.
    grid_id: egui::Id,
    /// Left edge of the grid (pixel x).
    left: f32,
    /// Current column within the row (0..GRID_COLUMNS).
    cursor: u32,
    /// Current row index (0-based). Combines with `cursor` to form a
    /// stable per-cell ID.
    row_index: u32,
    /// Top of the current row (pixel y).
    row_top: f32,
    /// Tallest cell bottom in the current row.
    row_max_bottom: f32,
}

impl<'ui, 'store> Grid<'ui, 'store> {
    // ── Named spans ────────────────────────────────────────────────
    //
    // These are the clean divisions of the 12-column grid.
    // Use these instead of raw column counts to stay on the grid.

    /// Full-width cell (12 columns, 744px).
    pub fn full(&mut self, f: impl FnOnce(&mut CardCtx<'_>)) {
        self.place(12, f);
    }

    /// Three-quarter cell (9 columns, 555px).
    pub fn three_quarters(&mut self, f: impl FnOnce(&mut CardCtx<'_>)) {
        self.place(9, f);
    }

    /// Two-thirds cell (8 columns, 492px).
    pub fn two_thirds(&mut self, f: impl FnOnce(&mut CardCtx<'_>)) {
        self.place(8, f);
    }

    /// Half-width cell (6 columns, 366px).
    pub fn half(&mut self, f: impl FnOnce(&mut CardCtx<'_>)) {
        self.place(6, f);
    }

    /// One-third cell (4 columns, 240px).
    pub fn third(&mut self, f: impl FnOnce(&mut CardCtx<'_>)) {
        self.place(4, f);
    }

    /// Quarter-width cell (3 columns, 177px).
    pub fn quarter(&mut self, f: impl FnOnce(&mut CardCtx<'_>)) {
        self.place(3, f);
    }

    // ── Named skips ─────────────────────────────────────────────────

    /// Skip a half-width gap (6 columns).
    pub fn skip_half(&mut self) {
        self.skip(6);
    }

    /// Skip a third-width gap (4 columns).
    pub fn skip_third(&mut self) {
        self.skip(4);
    }

    /// Skip a quarter-width gap (3 columns).
    pub fn skip_quarter(&mut self) {
        self.skip(3);
    }

    // ── Low-level ──────────────────────────────────────────────────

    /// Place content spanning an arbitrary number of grid columns.
    ///
    /// Prefer the named methods ([`full`](Self::full), [`half`](Self::half),
    /// [`third`](Self::third), [`quarter`](Self::quarter),
    /// [`two_thirds`](Self::two_thirds), [`three_quarters`](Self::three_quarters))
    /// to stay on the grid. This escape hatch exists for unusual layouts.
    pub fn place(&mut self, span: u32, add_contents: impl FnOnce(&mut CardCtx<'_>)) {
        assert!(
            span > 0 && span <= GRID_COLUMNS,
            "span must be 1..={GRID_COLUMNS}, got {span}"
        );

        // Advance to new row if the previous row was completed or
        // this span doesn't fit.
        if self.needs_advance(span) {
            self.new_row();
        }

        let x = self.left + col_x(self.cursor);
        let width = span_width(span);

        // Frame-delayed row sizing: every cell in row N uses the
        // previous frame's measured row height as `max_rect.height`.
        // Sub-layouts inside the cell (e.g. `Align::Center` cross-axis,
        // which fills `frame_size` to `available_rect.height()`) then
        // see the actual row height instead of a fake unbounded one,
        // so vertical centering aligns with the other cells in the row.
        // First frame: 0.0 → no expansion, items at their natural sizes.
        // Subsequent frames: the row's measured height converges in one
        // frame and stays stable, because once `max_rect.height` matches
        // the content height, centering produces the same height back.
        let row_id = self.row_state_id();
        let row_height: f32 = self
            .ui
            .ctx()
            .data(|d| d.get_temp::<f32>(row_id).unwrap_or(0.0));
        let cell_rect =
            egui::Rect::from_min_size(egui::pos2(x, self.row_top), egui::vec2(width, row_height));

        let store = self.store;
        // Use new_child (not scope_builder) so cells don't advance the
        // parent cursor — finish() handles that in one shot.
        let mut child = self
            .ui
            .new_child(egui::UiBuilder::new().max_rect(cell_rect));
        child.set_width(width);
        let mut ctx = CardCtx::new(&mut child, store);
        add_contents(&mut ctx);

        // Snap cell heights up to the next module so rows stay on a
        // typographic baseline grid: every row_top lands on a grid line
        // relative to the start of the grid.
        let used_bottom = child.min_rect().bottom();
        let used_height = (used_bottom - self.row_top).max(0.0);
        let snapped_height = (used_height / GRID_ROW_MODULE).ceil() * GRID_ROW_MODULE;
        let snapped_bottom = self.row_top + snapped_height;
        if snapped_bottom > self.row_max_bottom {
            self.row_max_bottom = snapped_bottom;
        }

        self.cursor += span;
        if self.cursor >= GRID_COLUMNS {
            self.cursor = 0;
        }
    }

    /// Stable identifier for the current row's persisted height.
    fn row_state_id(&self) -> egui::Id {
        self.grid_id.with(("row", self.row_index))
    }

    /// Skip columns (typographic furniture / blank space).
    ///
    /// Common patterns: `skip_quarter()`, `skip_third()`, `skip_half()`.
    pub fn skip(&mut self, span: u32) {
        assert!(
            span > 0 && span <= GRID_COLUMNS,
            "skip must be 1..={GRID_COLUMNS}, got {span}"
        );

        if self.needs_advance(span) {
            self.new_row();
        }

        self.cursor += span;
        if self.cursor >= GRID_COLUMNS {
            self.cursor = 0;
        }
    }

    /// Check if we need to advance to a new row before placing `span` columns.
    fn needs_advance(&self, span: u32) -> bool {
        // Previous row completed (cursor reset to 0) with content above.
        let pending_complete = self.cursor == 0 && self.row_max_bottom > self.row_top;
        // Span doesn't fit current row.
        let overflow = self.cursor > 0 && self.cursor + span > GRID_COLUMNS;
        pending_complete || overflow
    }

    /// Start a new row with one module (12px) of vertical gutter.
    fn new_row(&mut self) {
        // Persist the row we just finished so next frame's cells use
        // the correct `max_rect.height` for cross-axis centering, etc.
        self.persist_row_height();
        self.row_top = self.row_max_bottom + GRID_ROW_MODULE;
        self.row_max_bottom = self.row_top;
        self.cursor = 0;
        self.row_index += 1;
    }

    /// Save the current row's measured height (`row_max_bottom - row_top`)
    /// to `ctx.data` so the next frame's cells in that row can read it
    /// back as their `max_rect.height`.
    fn persist_row_height(&mut self) {
        let height = (self.row_max_bottom - self.row_top).max(0.0);
        let row_id = self.row_state_id();
        self.ui.ctx().data_mut(|d| d.insert_temp(row_id, height));
    }

    /// Advance the parent Ui's cursor past all grid content.
    fn finish(&mut self) {
        // Persist the last row's measured height too (`new_row` only
        // fires between rows, not after the last one).
        self.persist_row_height();
        // Don't snap the last row — just add bottom padding.
        // Inter-row snapping is handled by new_row().
        let total_height = (self.row_max_bottom + GRID_EDGE_PAD - self.ui.cursor().min.y).max(0.0);
        if total_height > 0.0 {
            self.ui.allocate_space(egui::vec2(
                2.0 * GRID_EDGE_PAD + span_width(GRID_COLUMNS),
                total_height,
            ));
        }
    }
}

/// Pixel x-offset of grid column `col` from the left edge.
const fn col_x(col: u32) -> f32 {
    col as f32 * (GRID_COL_WIDTH + GRID_GUTTER)
}
