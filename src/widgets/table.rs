//! Table view with (optional) fixed header and scrolling body.
//! Cell widths are precalculated with given size hints so we can have tables like this:
//! | fixed size | all available space/minimum | 30% of available width | fixed size |
//! Takes all available height, so if you want something below the table, put it in a strip.

use eframe::egui::{
    self,
    scroll_area::{ScrollAreaOutput, ScrollBarVisibility, ScrollSource},
    Align, Id, NumExt as _, Rangef, Rect, Response, ScrollArea, Ui, Vec2, Vec2b,
};

use super::table_layout::{CellDirection, CellSize, StripLayout, StripLayoutFlags};
use super::table_sizing::{Size, Sizing};

// -----------------------------------------------------------------=----------

/// Panic in debug builds, log otherwise.
macro_rules! log_or_panic {
    ($fmt: literal) => { log_or_panic!($fmt,) };
    ($fmt: literal, $($arg: tt)*) => {{
        if cfg!(debug_assertions) {
            panic!($fmt, $($arg)*);
        } else {
            log::error!($fmt, $($arg)*);
        }
    }};
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InitialColumnSize {
    /// Absolute size in points
    Absolute(f32),

    /// Base on content
    Automatic(f32),

    /// Take all available space
    Remainder,
}

/// Specifies the properties of a column, like its width range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    initial_width: InitialColumnSize,

    width_range: Rangef,

    /// Clip contents if too narrow?
    clip: bool,

    resizable: Option<bool>,

    /// If set, we should accurately measure the size of this column this frame
    /// so that we can correctly auto-size it. This is done as a `sizing_pass`.
    auto_size_this_frame: bool,
}

impl Column {
    /// Automatically sized based on content.
    ///
    /// If you have many thousands of rows and are therefore using [`TableBody::rows`]
    /// or [`TableBody::heterogeneous_rows`], then the automatic size will only be based
    /// on the currently visible rows.
    pub fn auto() -> Self {
        Self::auto_with_initial_suggestion(100.0)
    }

    /// Automatically sized.
    ///
    /// The given fallback is a loose suggestion, that may be used to wrap
    /// cell contents, if they contain a wrapping layout.
    /// In most cases though, the given value is ignored.
    pub fn auto_with_initial_suggestion(suggested_width: f32) -> Self {
        Self::new(InitialColumnSize::Automatic(suggested_width))
    }

    /// With this initial width.
    pub fn initial(width: f32) -> Self {
        Self::new(InitialColumnSize::Absolute(width))
    }

    /// Always this exact width, never shrink or grow.
    pub fn exact(width: f32) -> Self {
        Self::new(InitialColumnSize::Absolute(width))
            .range(width..=width)
            .clip(true)
    }

    /// Take all the space remaining after the other columns have
    /// been sized.
    ///
    /// If you have multiple [`Column::remainder`] they all
    /// share the remaining space equally.
    pub fn remainder() -> Self {
        Self::new(InitialColumnSize::Remainder)
    }

    fn new(initial_width: InitialColumnSize) -> Self {
        Self {
            initial_width,
            width_range: Rangef::new(0.0, f32::INFINITY),
            resizable: None,
            clip: false,
            auto_size_this_frame: false,
        }
    }

    /// Can this column be resized by dragging the column separator?
    ///
    /// If you don't call this, the fallback value of
    /// [`TableBuilder::resizable`] is used (which by default is `false`).
    #[inline]
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = Some(resizable);
        self
    }

    /// If `true`: Allow the column to shrink enough to clip the contents.
    /// If `false`: The column will always be wide enough to contain all its content.
    ///
    /// Clipping can make sense if you expect a column to contain a lot of things,
    /// and you don't want it too take up too much space.
    /// If you turn on clipping you should also consider calling [`Self::at_least`].
    ///
    /// Default: `false`.
    #[inline]
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    /// Won't shrink below this width (in points).
    ///
    /// Default: 0.0
    #[inline]
    pub fn at_least(mut self, minimum: f32) -> Self {
        self.width_range.min = minimum;
        self
    }

    /// Won't grow above this width (in points).
    ///
    /// Default: [`f32::INFINITY`]
    #[inline]
    pub fn at_most(mut self, maximum: f32) -> Self {
        self.width_range.max = maximum;
        self
    }

    /// Won't shrink below a span of `n` grid columns (`n` columns plus `n-1`
    /// gutters), matching [`Button::columns`](crate::widgets::Button::columns).
    ///
    /// Prefer this over a hand-picked `at_least(220.0)`: a raw pixel minimum is
    /// a guess against one font size, and it silently starts wrapping cell text
    /// the moment the type scale moves. A span is defined by the same grid the
    /// page is laid out on, so it tracks the layout instead of drifting from it.
    #[inline]
    pub fn columns(mut self, n: u32) -> Self {
        self.width_range.min = crate::card_ctx::span_width(n);
        self
    }

    /// Allowed range of movement (in points), if in a resizable [`Table`].
    #[inline]
    pub fn range(mut self, range: impl Into<Rangef>) -> Self {
        self.width_range = range.into();
        self
    }

    /// If set, the column will be automatically sized based on the content this frame.
    ///
    /// Do not set this every frame, just on a specific action.
    #[inline]
    pub fn auto_size_this_frame(mut self, auto_size_this_frame: bool) -> Self {
        self.auto_size_this_frame = auto_size_this_frame;
        self
    }

    fn is_auto(&self) -> bool {
        match self.initial_width {
            InitialColumnSize::Automatic(_) => true,
            InitialColumnSize::Absolute(_) | InitialColumnSize::Remainder => false,
        }
    }
}

fn to_sizing(columns: &[Column]) -> Sizing {
    let mut sizing = Sizing::default();
    for column in columns {
        let size = match column.initial_width {
            InitialColumnSize::Absolute(width) => Size::exact(width),
            InitialColumnSize::Automatic(suggested_width) => Size::initial(suggested_width),
            InitialColumnSize::Remainder => Size::remainder(),
        }
        .with_range(column.width_range);
        sizing.add(size);
    }
    sizing
}

// -----------------------------------------------------------------=----------

struct TableScrollOptions {
    vscroll: bool,
    drag_to_scroll: bool,
    stick_to_bottom: bool,
    scroll_to_row: Option<(usize, Option<Align>)>,
    scroll_offset_y: Option<f32>,
    min_scrolled_height: f32,
    max_scroll_height: f32,
    auto_shrink: Vec2b,
    scroll_bar_visibility: ScrollBarVisibility,
    animated: bool,
}

impl Default for TableScrollOptions {
    fn default() -> Self {
        Self {
            vscroll: true,
            drag_to_scroll: true,
            stick_to_bottom: false,
            scroll_to_row: None,
            scroll_offset_y: None,
            min_scrolled_height: 200.0,
            max_scroll_height: f32::INFINITY,
            auto_shrink: Vec2b::TRUE,
            scroll_bar_visibility: ScrollBarVisibility::VisibleWhenNeeded,
            animated: true,
        }
    }
}

// -----------------------------------------------------------------=----------

/// Builder for a [`Table`] with (optional) fixed header and scrolling body.
///
/// You must pre-allocate all columns with [`Self::column`]/[`Self::columns`].
///
/// If you have multiple [`Table`]:s in the same [`Ui`]
/// you will need to give them unique id:s by with [`Self::id_salt`].
///
/// ### Example
/// ```
/// # egui::__run_test_ui(|ui| {
/// use lissajous::widgets::{TableBuilder, Column};
/// TableBuilder::new(ui)
///     .column(Column::auto().resizable(true))
///     .column(Column::remainder())
///     .header(20.0, |mut header| {
///         header.col(|ui| {
///             ui.heading("First column");
///         });
///         header.col(|ui| {
///             ui.heading("Second column");
///         });
///     })
///     .body(|mut body| {
///         body.row(30.0, |mut row| {
///             row.col(|ui| {
///                 ui.label("Hello");
///             });
///             row.col(|ui| {
///                 ui.button("world!");
///             });
///         });
///     });
/// # });
/// ```
pub struct TableBuilder<'a> {
    ui: &'a mut Ui,
    id_salt: Id,
    columns: Vec<Column>,
    striped: Option<bool>,
    resizable: bool,
    cell_layout: egui::Layout,
    scroll_options: TableScrollOptions,
    sense: egui::Sense,
    header_body_gap: f32,
    dense_rows: bool,
}

impl<'a> TableBuilder<'a> {
    pub fn new(ui: &'a mut Ui) -> Self {
        let cell_layout = *ui.layout();
        Self {
            ui,
            id_salt: Id::new("__table_state"),
            columns: Default::default(),
            striped: None,
            resizable: false,
            cell_layout,
            scroll_options: Default::default(),
            sense: egui::Sense::hover(),
            header_body_gap: 0.0,
            dense_rows: false,
        }
    }

    /// Give this table a unique id within the parent [`Ui`].
    ///
    /// This is required if you have multiple tables in the same [`Ui`].
    #[inline]
    #[deprecated = "Renamed id_salt"]
    pub fn id_source(self, id_salt: impl std::hash::Hash) -> Self {
        self.id_salt(id_salt)
    }

    /// Give this table a unique id within the parent [`Ui`].
    ///
    /// This is required if you have multiple tables in the same [`Ui`].
    #[inline]
    pub fn id_salt(mut self, id_salt: impl std::hash::Hash) -> Self {
        self.id_salt = Id::new(id_salt);
        self
    }

    /// Enable striped row background for improved readability.
    ///
    /// Default is whatever is in [`egui::Visuals::striped`].
    #[inline]
    pub fn striped(mut self, striped: bool) -> Self {
        self.striped = Some(striped);
        self
    }

    /// What should table cells sense for? (default: [`egui::Sense::hover()`]).
    #[inline]
    pub fn sense(mut self, sense: egui::Sense) -> Self {
        self.sense = sense;
        self
    }

    /// Make the columns resizable by dragging.
    ///
    /// You can set this for individual columns with [`Column::resizable`].
    /// [`Self::resizable`] is used as a fallback for any column for which you don't call
    /// [`Column::resizable`].
    ///
    /// If the _last_ column is [`Column::remainder`], then it won't be resizable
    /// (and instead use up the remainder).
    ///
    /// Default is `false`.
    #[inline]
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Insert a fixed gap between header and body, clipping body rows underneath it.
    #[inline]
    pub fn header_body_gap(mut self, gap: f32) -> Self {
        self.header_body_gap = gap.max(0.0);
        self
    }

    /// Remove vertical row spacing inside the body.
    #[inline]
    pub fn dense_rows(mut self, dense: bool) -> Self {
        self.dense_rows = dense;
        self
    }

    /// Enable vertical scrolling in body (default: `true`)
    #[inline]
    pub fn vscroll(mut self, vscroll: bool) -> Self {
        self.scroll_options.vscroll = vscroll;
        self
    }

    /// Enables scrolling the table's contents using mouse drag (default: `true`).
    ///
    /// See [`ScrollArea::drag_to_scroll`] for more.
    #[inline]
    pub fn drag_to_scroll(mut self, drag_to_scroll: bool) -> Self {
        self.scroll_options.drag_to_scroll = drag_to_scroll;
        self
    }

    /// Should the scroll handle stick to the bottom position even as the content size changes
    /// dynamically? The scroll handle remains stuck until manually changed, and will become stuck
    /// once again when repositioned to the bottom. Default: `false`.
    #[inline]
    pub fn stick_to_bottom(mut self, stick: bool) -> Self {
        self.scroll_options.stick_to_bottom = stick;
        self
    }

    /// Set a row to scroll to.
    ///
    /// `align` specifies if the row should be positioned in the top, center, or bottom of the view
    /// (using [`Align::TOP`], [`Align::Center`] or [`Align::BOTTOM`]).
    /// If `align` is `None`, the table will scroll just enough to bring the cursor into view.
    ///
    /// See also: [`Self::vertical_scroll_offset`].
    #[inline]
    pub fn scroll_to_row(mut self, row: usize, align: Option<Align>) -> Self {
        self.scroll_options.scroll_to_row = Some((row, align));
        self
    }

    /// Set the vertical scroll offset position, in points.
    ///
    /// See also: [`Self::scroll_to_row`].
    #[inline]
    pub fn vertical_scroll_offset(mut self, offset: f32) -> Self {
        self.scroll_options.scroll_offset_y = Some(offset);
        self
    }

    /// The minimum height of a vertical scroll area which requires scroll bars.
    ///
    /// The scroll area will only become smaller than this if the content is smaller than this
    /// (and so we don't require scroll bars).
    ///
    /// Default: `200.0`.
    #[inline]
    pub fn min_scrolled_height(mut self, min_scrolled_height: f32) -> Self {
        self.scroll_options.min_scrolled_height = min_scrolled_height;
        self
    }

    /// Don't make the scroll area higher than this (add scroll-bars instead!).
    ///
    /// In other words: add scroll-bars when this height is reached.
    /// Default: `800.0`.
    #[inline]
    pub fn max_scroll_height(mut self, max_scroll_height: f32) -> Self {
        self.scroll_options.max_scroll_height = max_scroll_height;
        self
    }

    /// For each axis (x,y):
    /// * If true, add blank space outside the table, keeping the table small.
    /// * If false, add blank space inside the table, expanding the table to fit the containing ui.
    ///
    /// Default: `true`.
    ///
    /// See [`ScrollArea::auto_shrink`] for more.
    #[inline]
    pub fn auto_shrink(mut self, auto_shrink: impl Into<Vec2b>) -> Self {
        self.scroll_options.auto_shrink = auto_shrink.into();
        self
    }

    /// Set the visibility of both horizontal and vertical scroll bars.
    ///
    /// With `ScrollBarVisibility::VisibleWhenNeeded` (default), the scroll bar will be visible only when needed.
    #[inline]
    pub fn scroll_bar_visibility(mut self, scroll_bar_visibility: ScrollBarVisibility) -> Self {
        self.scroll_options.scroll_bar_visibility = scroll_bar_visibility;
        self
    }

    /// Should the scroll area animate `scroll_to_*` functions?
    ///
    /// Default: `true`.
    #[inline]
    pub fn animate_scrolling(mut self, animated: bool) -> Self {
        self.scroll_options.animated = animated;
        self
    }

    /// What layout should we use for the individual cells?
    #[inline]
    pub fn cell_layout(mut self, cell_layout: egui::Layout) -> Self {
        self.cell_layout = cell_layout;
        self
    }

    /// Allocate space for one column.
    #[inline]
    pub fn column(mut self, column: Column) -> Self {
        self.columns.push(column);
        self
    }

    /// Allocate space for several columns at once.
    #[inline]
    pub fn columns(mut self, column: Column, count: usize) -> Self {
        for _ in 0..count {
            self.columns.push(column);
        }
        self
    }

    fn available_width(&self) -> f32 {
        self.ui.available_rect_before_wrap().width()
            - (self.scroll_options.vscroll as i32 as f32)
                * self.ui.spacing().scroll.allocated_width()
    }

    /// Reset all column widths.
    pub fn reset(&self) {
        let state_id = self.ui.id().with(self.id_salt);
        TableState::reset(self.ui, state_id);
    }

    /// Create a header row which always stays visible and at the top
    pub fn header(self, height: f32, add_header_row: impl FnOnce(TableRow<'_, '_>)) -> Table<'a> {
        let available_width = self.available_width();

        let Self {
            ui,
            id_salt,
            mut columns,
            striped,
            resizable,
            cell_layout,
            scroll_options,
            sense,
            header_body_gap,
            dense_rows,
        } = self;

        for (i, column) in columns.iter_mut().enumerate() {
            let column_resize_id = ui.id().with("resize_column").with(i);
            if let Some(response) = ui.ctx().read_response(column_resize_id) {
                if response.double_clicked() {
                    column.auto_size_this_frame = true;
                }
            }
        }

        let striped = striped.unwrap_or(ui.visuals().striped);

        let state_id = ui.id().with(id_salt);

        let (is_sizing_pass, state) =
            TableState::load(ui, state_id, resizable, &columns, available_width);

        let mut max_used_widths = vec![0.0; columns.len()];
        let table_top = ui.cursor().top();

        let mut ui_builder = egui::UiBuilder::new();
        if is_sizing_pass {
            ui_builder = ui_builder.sizing_pass();
        }
        let prev_item_spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing.y = 0.0;
        // Track actual header text bounds so the divider hugs glyphs, not the row rect.
        let mut header_text_bottom = f32::NEG_INFINITY;
        let header_response = ui.scope_builder(ui_builder, |ui| {
            let mut layout = StripLayout::new(ui, CellDirection::Horizontal, cell_layout, sense);
            let mut response: Option<Response> = None;
            add_header_row(TableRow {
                layout: &mut layout,
                columns: &columns,
                widths: &state.column_widths,
                max_used_widths: &mut max_used_widths,
                row_index: 0,
                col_index: 0,
                height,
                striped: false,
                hovered: false,
                selected: false,
                overline: false,
                content_bottom: Some(&mut header_text_bottom),
                response: &mut response,
            });
            layout.allocate_rect()
        });

        let header_divider = ui.visuals().widgets.noninteractive.bg_stroke;
        let header_left = header_response.inner.rect.left();
        let header_right = header_left + available_width;
        let header_bottom = header_response.inner.rect.bottom();
        if header_divider.width > 0.0 {
            let divider_y = if header_text_bottom.is_finite() {
                header_text_bottom.clamp(header_response.inner.rect.top(), header_bottom)
            } else {
                header_bottom
            };
            ui.painter().hline(
                Rangef::new(header_left, header_right),
                divider_y,
                header_divider,
            );
        }

        let mut body_top = header_bottom;
        if header_body_gap > 0.0 {
            ui.add_space(header_body_gap);
            body_top = ui.cursor().top();
        }
        ui.spacing_mut().item_spacing = prev_item_spacing;

        Table {
            ui,
            table_top,
            body_top,
            state_id,
            columns,
            available_width,
            state,
            max_used_widths,
            is_sizing_pass,
            resizable,
            striped,
            cell_layout,
            scroll_options,
            sense,
            dense_rows,
        }
    }

    /// Create table body without a header row
    pub fn body<F>(self, add_body_contents: F) -> ScrollAreaOutput<()>
    where
        F: for<'b> FnOnce(TableBody<'b>),
    {
        let available_width = self.available_width();

        let Self {
            ui,
            id_salt,
            columns,
            striped,
            resizable,
            cell_layout,
            scroll_options,
            sense,
            header_body_gap: _header_body_gap,
            dense_rows,
        } = self;

        let striped = striped.unwrap_or(ui.visuals().striped);

        let state_id = ui.id().with(id_salt);

        let (is_sizing_pass, state) =
            TableState::load(ui, state_id, resizable, &columns, available_width);

        let max_used_widths = vec![0.0; columns.len()];
        let table_top = ui.cursor().top();
        let body_top = table_top;

        Table {
            ui,
            table_top,
            body_top,
            state_id,
            columns,
            available_width,
            state,
            max_used_widths,
            is_sizing_pass,
            resizable,
            striped,
            cell_layout,
            scroll_options,
            sense,
            dense_rows,
        }
        .body(add_body_contents)
    }
}

// ----------------------------------------------------------------------------

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
struct TableState {
    column_widths: Vec<f32>,

    /// If known from previous frame
    #[cfg_attr(feature = "serde", serde(skip))]
    max_used_widths: Vec<f32>,
}

impl TableState {
    fn clamp_widths_to_available(
        columns: &[Column],
        widths: &mut [f32],
        available_width: f32,
        spacing_x: f32,
    ) {
        if widths.is_empty() {
            return;
        }

        let available_for_columns =
            (available_width - spacing_x * (widths.len().saturating_sub(1) as f32)).max(0.0);
        let mut sum: f32 = widths.iter().sum();
        if sum <= available_for_columns || sum <= 0.0 {
            return;
        }

        let min_sum: f32 = columns.iter().map(|c| c.width_range.min).sum();
        if min_sum >= available_for_columns {
            let scale = available_for_columns / sum;
            for (width, column) in widths.iter_mut().zip(columns) {
                let scaled = *width * scale;
                *width = scaled.min(column.width_range.max);
            }
            return;
        }

        let scale = available_for_columns / sum;
        for (width, column) in widths.iter_mut().zip(columns) {
            let scaled = *width * scale;
            *width = scaled.clamp(column.width_range.min, column.width_range.max);
        }

        sum = widths.iter().sum();
        if sum <= available_for_columns {
            return;
        }

        let excess = sum - available_for_columns;
        let mut reducible = 0.0;
        for (width, column) in widths.iter().zip(columns) {
            reducible += (*width - column.width_range.min).max(0.0);
        }
        if reducible <= 0.0 {
            return;
        }

        for (width, column) in widths.iter_mut().zip(columns) {
            let slack = (*width - column.width_range.min).max(0.0);
            if slack > 0.0 {
                *width -= excess * (slack / reducible);
            }
        }
    }

    /// Return true if we should do a sizing pass.
    fn load(
        ui: &Ui,
        state_id: egui::Id,
        resizable: bool,
        columns: &[Column],
        available_width: f32,
    ) -> (bool, Self) {
        let rect = Rect::from_min_size(ui.available_rect_before_wrap().min, Vec2::ZERO);
        ui.ctx().check_for_id_clash(state_id, rect, "Table");

        #[cfg(feature = "serde")]
        let state = ui.data_mut(|d| d.get_persisted::<Self>(state_id));
        #[cfg(not(feature = "serde"))]
        let state = ui.data_mut(|d| d.get_temp::<Self>(state_id));

        // Make sure that the stored widths aren't out-dated:
        let state = state.filter(|state| state.column_widths.len() == columns.len());

        let is_sizing_pass =
            ui.is_sizing_pass() || state.is_none() && columns.iter().any(|c| c.is_auto());

        let mut state = state.unwrap_or_else(|| {
            let initial_widths =
                to_sizing(columns).to_lengths(available_width, ui.spacing().item_spacing.x);
            Self {
                column_widths: initial_widths,
                max_used_widths: Default::default(),
            }
        });

        if !is_sizing_pass && state.max_used_widths.len() == columns.len() {
            // Make sure any non-resizable `remainder` columns are updated
            // to take up the remainder of the current available width.
            // Also handles changing item spacing.
            let mut sizing = Sizing::default();
            for ((prev_width, max_used), column) in state
                .column_widths
                .iter()
                .zip(&state.max_used_widths)
                .zip(columns)
            {
                let column_resizable = column.resizable.unwrap_or(resizable);
                let size = if column_resizable {
                    // Resiable columns keep their width:
                    Size::exact(*prev_width)
                } else {
                    match column.initial_width {
                        InitialColumnSize::Absolute(width) => Size::exact(width),
                        InitialColumnSize::Automatic(_) => Size::exact(*prev_width),
                        InitialColumnSize::Remainder => Size::remainder(),
                    }
                    .at_least(column.width_range.min.max(*max_used))
                    .at_most(column.width_range.max)
                };
                sizing.add(size);
            }
            state.column_widths = sizing.to_lengths(available_width, ui.spacing().item_spacing.x);
        }

        Self::clamp_widths_to_available(
            columns,
            &mut state.column_widths,
            available_width,
            ui.spacing().item_spacing.x,
        );

        (is_sizing_pass, state)
    }

    fn store(self, ui: &egui::Ui, state_id: egui::Id) {
        #[cfg(feature = "serde")]
        ui.data_mut(|d| {
            d.insert_persisted(state_id, self);
        });
        #[cfg(not(feature = "serde"))]
        ui.data_mut(|d| {
            d.insert_temp(state_id, self);
        });
    }

    fn reset(ui: &egui::Ui, state_id: egui::Id) {
        ui.data_mut(|d| d.remove::<Self>(state_id));
    }
}

// ----------------------------------------------------------------------------

/// Table struct which can construct a [`TableBody`].
///
/// Is created by [`TableBuilder`] by either calling [`TableBuilder::body`] or after creating a header row with [`TableBuilder::header`].
pub struct Table<'a> {
    ui: &'a mut Ui,
    table_top: f32,
    body_top: f32,
    state_id: egui::Id,
    columns: Vec<Column>,
    available_width: f32,
    state: TableState,

    /// Accumulated maximum used widths for each column.
    max_used_widths: Vec<f32>,

    /// During the sizing pass we calculate the width of columns with [`Column::auto`].
    is_sizing_pass: bool,
    resizable: bool,
    striped: bool,
    cell_layout: egui::Layout,

    scroll_options: TableScrollOptions,

    sense: egui::Sense,

    dense_rows: bool,
}

impl Table<'_> {
    /// Access the contained [`egui::Ui`].
    ///
    /// You can use this to e.g. modify the [`egui::Style`] with [`egui::Ui::style_mut`].
    pub fn ui_mut(&mut self) -> &mut egui::Ui {
        self.ui
    }

    /// Create table body after adding a header row
    pub fn body<F>(self, add_body_contents: F) -> ScrollAreaOutput<()>
    where
        F: for<'b> FnOnce(TableBody<'b>),
    {
        let Table {
            ui,
            table_top,
            body_top,
            state_id,
            columns,
            resizable,
            mut available_width,
            mut state,
            mut max_used_widths,
            is_sizing_pass,
            striped,
            cell_layout,
            scroll_options,
            sense,
            dense_rows,
        } = self;
        let table_width = available_width;

        let TableScrollOptions {
            vscroll,
            drag_to_scroll,
            stick_to_bottom,
            scroll_to_row,
            scroll_offset_y,
            min_scrolled_height,
            max_scroll_height,
            auto_shrink,
            scroll_bar_visibility,
            animated,
        } = scroll_options;

        let cursor_position = ui.cursor().min;

        let mut scroll_area = ScrollArea::new([false, vscroll])
            .id_salt(state_id.with("__scroll_area"))
            .scroll_source(ScrollSource {
                drag: drag_to_scroll,
                ..Default::default()
            })
            .stick_to_bottom(stick_to_bottom)
            .min_scrolled_height(min_scrolled_height)
            .max_height(max_scroll_height)
            .auto_shrink(auto_shrink)
            .scroll_bar_visibility(scroll_bar_visibility)
            .animated(animated);

        if let Some(scroll_offset_y) = scroll_offset_y {
            scroll_area = scroll_area.vertical_scroll_offset(scroll_offset_y);
        }

        let columns_ref = &columns;
        let widths_ref = &state.column_widths;
        let max_used_widths_ref = &mut max_used_widths;

        let scroll_area_out = scroll_area.show(ui, move |ui| {
            let mut scroll_to_y_range = None;

            if dense_rows {
                ui.spacing_mut().item_spacing.y = 0.0;
            }

            let mut clip_rect = ui.clip_rect();
            if body_top > clip_rect.min.y {
                clip_rect.min.y = body_top;
                ui.set_clip_rect(clip_rect);
            }
            let clip_rect = ui.clip_rect();

            let mut ui_builder = egui::UiBuilder::new();
            if is_sizing_pass {
                ui_builder = ui_builder.sizing_pass();
            }
            ui.scope_builder(ui_builder, |ui| {
                let hovered_row_index_id = self.state_id.with("__table_hovered_row");
                let hovered_row_index =
                    ui.data_mut(|data| data.remove_temp::<usize>(hovered_row_index_id));

                let layout = StripLayout::new(ui, CellDirection::Horizontal, cell_layout, sense);

                add_body_contents(TableBody {
                    layout,
                    columns: columns_ref,
                    widths: widths_ref,
                    max_used_widths: max_used_widths_ref,
                    striped,
                    row_index: 0,
                    y_range: clip_rect.y_range(),
                    scroll_to_row: scroll_to_row.map(|(r, _)| r),
                    scroll_to_y_range: &mut scroll_to_y_range,
                    hovered_row_index,
                    hovered_row_index_id,
                });

                if scroll_to_row.is_some() && scroll_to_y_range.is_none() {
                    // TableBody::row didn't find the correct row, so scroll to the bottom:
                    scroll_to_y_range = Some(Rangef::new(f32::INFINITY, f32::INFINITY));
                }
            });

            if let Some(y_range) = scroll_to_y_range {
                let x = 0.0; // ignored, we only have vertical scrolling
                let rect = egui::Rect::from_x_y_ranges(x..=x, y_range);
                let align = scroll_to_row.and_then(|(_, a)| a);
                ui.scroll_to_rect(rect, align);
            }
        });

        let bottom = ui.min_rect().bottom();

        let spacing_x = ui.spacing().item_spacing.x;
        let total_spacing = spacing_x * (state.column_widths.len().saturating_sub(1) as f32);
        let mut width_snapshot = state.column_widths.clone();
        let resize_drag_id = state_id.with("resize_column_drag");
        if !ui.input(|i| i.pointer.any_down()) {
            ui.data_mut(|data| data.remove_temp::<ColumnResizeDrag>(resize_drag_id));
        }
        let drag_anchor = ui.data_mut(|data| data.get_temp::<ColumnResizeDrag>(resize_drag_id));
        let drag_anchor_index = drag_anchor.as_ref().map(|state| state.column_index);
        let mut x = cursor_position.x - spacing_x * 0.5;
        let width_count = state.column_widths.len();
        for i in 0..width_count {
            let (left, right) = state.column_widths.split_at_mut(i + 1);
            let (left_cols, column_slice) = left.split_at_mut(i);
            let column_width = &mut column_slice[0];
            let column = &columns[i];
            let column_is_resizable = column.resizable.unwrap_or(resizable);
            let width_range = column.width_range;
            let freeze_width = drag_anchor_index.is_some_and(|index| i > index);

            let is_last_column = i + 1 == columns.len();
            if is_last_column
                && column.initial_width == InitialColumnSize::Remainder
                && !ui.is_sizing_pass()
                && !freeze_width
            {
                // If the last column is 'remainder', then let it fill the remainder!
                let eps = 0.1; // just to avoid some rounding errors.
                *column_width = available_width - eps;
                if !column.clip {
                    *column_width = column_width.at_least(max_used_widths[i]);
                }
                *column_width = width_range.clamp(*column_width);
                width_snapshot[i] = *column_width;
                break;
            }

            if !freeze_width && ui.is_sizing_pass() {
                if column.clip {
                    // If we clip, we don't need to be as wide as the max used width
                    *column_width = column_width.min(max_used_widths[i]);
                } else {
                    *column_width = max_used_widths[i];
                }
            } else if !freeze_width && !column.clip {
                // Unless we clip we don't want to shrink below the
                // size that was actually used:
                *column_width = column_width.at_least(max_used_widths[i]);
            }
            if !freeze_width {
                *column_width = width_range.clamp(*column_width);
            }

            x += *column_width + spacing_x;

            if !freeze_width && column.is_auto() && (is_sizing_pass || !column_is_resizable) {
                *column_width = width_range.clamp(max_used_widths[i]);
            } else if column_is_resizable && !is_last_column {
                let column_resize_id = state_id.with("resize_column").with(i);

                let mut p0 = egui::pos2(x, table_top);
                let mut p1 = egui::pos2(x, bottom);
                let line_rect = egui::Rect::from_min_max(p0, p1)
                    .expand(ui.style().interaction.resize_grab_radius_side);

                let resize_response =
                    ui.interact(line_rect, column_resize_id, egui::Sense::click_and_drag());

                if column.auto_size_this_frame {
                    // Auto-size: resize to what is needed.
                    *column_width = width_range.clamp(max_used_widths[i]);
                } else if resize_response.dragged() {
                    if let Some(pointer) = ui.ctx().pointer_latest_pos() {
                        let mut drag_state =
                            ui.data_mut(|data| data.get_temp::<ColumnResizeDrag>(resize_drag_id));
                        if resize_response.drag_started() {
                            let mut start_widths = width_snapshot.clone();
                            start_widths[i] = *column_width;
                            for (j, width) in right.iter().enumerate() {
                                start_widths[i + 1 + j] = *width;
                            }
                            let state = ColumnResizeDrag {
                                column_index: i,
                                start_x: pointer.x,
                                start_widths,
                            };
                            ui.data_mut(|data| data.insert_temp(resize_drag_id, state.clone()));
                            drag_state = Some(state);
                        }
                        if let Some(state) =
                            drag_state.as_ref().filter(|state| state.column_index == i)
                        {
                            let start_widths = &state.start_widths;
                            for (j, width) in left_cols.iter_mut().enumerate() {
                                *width = start_widths[j];
                                width_snapshot[j] = *width;
                            }
                            *column_width = start_widths[i];
                            width_snapshot[i] = *column_width;
                            for (j, width) in right.iter_mut().enumerate() {
                                *width = start_widths[i + 1 + j];
                                width_snapshot[i + 1 + j] = *width;
                            }

                            let columns_left = &columns[..i];
                            let columns_right = &columns[i + 1..];

                            let mut delta = pointer.x - state.start_x;
                            if !column.clip {
                                let max_shrinkage_per_frame = 8.0;
                                let min_allowed = (max_used_widths[i] - max_shrinkage_per_frame)
                                    .max(width_range.min);
                                if start_widths[i] + delta < min_allowed {
                                    delta = min_allowed - start_widths[i];
                                }
                            }

                            let start_boundary_x = {
                                let mut sum = 0.0;
                                for width in start_widths.iter().take(i + 1) {
                                    sum += *width;
                                }
                                cursor_position.x - spacing_x * 0.5
                                    + sum
                                    + spacing_x * ((i + 1) as f32)
                            };

                            let applied_delta;
                            if delta >= 0.0 {
                                let max_increase = (width_range.max - start_widths[i]).max(0.0);
                                let mut desired = delta.min(max_increase);
                                let total_sum: f32 = start_widths.iter().sum();
                                let free_space = (table_width - total_spacing - total_sum).max(0.0);
                                let right_slack = sum_slack_to_min(right, columns_right, resizable);
                                let allowed = free_space + right_slack;
                                if desired > allowed {
                                    desired = allowed;
                                }

                                *column_width = start_widths[i] + desired;
                                width_snapshot[i] = *column_width;

                                let mut remaining = (desired - free_space).max(0.0);
                                remaining = shrink_forward(
                                    right,
                                    columns_right,
                                    i + 1,
                                    resizable,
                                    &mut width_snapshot,
                                    remaining,
                                );
                                let _ = remaining;

                                applied_delta = desired;
                            } else {
                                let mut desired = -delta;
                                let left_shrink =
                                    sum_slack_to_min(left_cols, columns_left, resizable)
                                        + (start_widths[i] - width_range.min).max(0.0);
                                let right_grow = sum_room_to_max(right, columns_right, resizable);
                                let allowed = left_shrink.min(right_grow);
                                if desired > allowed {
                                    desired = allowed;
                                }

                                let mut remaining = desired;
                                let min_i = width_range.min;
                                let slack_i = (start_widths[i] - min_i).max(0.0);
                                let take_i = slack_i.min(remaining);
                                *column_width = start_widths[i] - take_i;
                                width_snapshot[i] = *column_width;
                                remaining -= take_i;

                                remaining = shrink_reverse(
                                    left_cols,
                                    columns_left,
                                    0,
                                    resizable,
                                    &mut width_snapshot,
                                    remaining,
                                );
                                let _ = remaining;

                                let _ = grow_forward(
                                    right,
                                    columns_right,
                                    i + 1,
                                    resizable,
                                    &mut width_snapshot,
                                    desired,
                                );

                                applied_delta = -desired;
                            }

                            x = start_boundary_x + applied_delta;
                        }
                        (p0.x, p1.x) = (x, x);
                    }
                }

                let dragging_something_else =
                    ui.input(|i| i.pointer.any_down() || i.pointer.any_pressed());
                let resize_hover = resize_response.hovered() && !dragging_something_else;

                if resize_hover || resize_response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                }

                let stroke = if resize_response.dragged() {
                    ui.style().visuals.widgets.active.bg_stroke
                } else if resize_hover {
                    ui.style().visuals.widgets.hovered.bg_stroke
                } else {
                    // ui.visuals().widgets.inactive.bg_stroke
                    ui.visuals().widgets.noninteractive.bg_stroke
                };

                ui.painter().line_segment([p0, p1], stroke);
            }

            width_snapshot[i] = *column_width;
            available_width -= *column_width + spacing_x;
        }

        state.max_used_widths = max_used_widths;

        state.store(ui, state_id);
        scroll_area_out
    }
}

/// The body of a table.
///
/// Is created by calling `body` on a [`Table`] (after adding a header row) or [`TableBuilder`] (without a header row).
pub struct TableBody<'a> {
    layout: StripLayout<'a>,

    columns: &'a [Column],

    /// Current column widths.
    widths: &'a [f32],

    /// Accumulated maximum used widths for each column.
    max_used_widths: &'a mut [f32],

    striped: bool,
    row_index: usize,
    y_range: Rangef,

    /// Look for this row to scroll to.
    scroll_to_row: Option<usize>,

    /// If we find the correct row to scroll to,
    /// this is set to the y-range of the row.
    scroll_to_y_range: &'a mut Option<Rangef>,

    hovered_row_index: Option<usize>,

    /// Used to store the hovered row index between frames.
    hovered_row_index_id: egui::Id,
}

fn sum_slack_to_min(widths: &[f32], cols: &[Column], resizable_default: bool) -> f32 {
    widths
        .iter()
        .zip(cols)
        .filter(|(_, col)| col.resizable.unwrap_or(resizable_default))
        .map(|(width, col)| (width - col.width_range.min).max(0.0))
        .sum()
}

fn sum_room_to_max(widths: &[f32], cols: &[Column], resizable_default: bool) -> f32 {
    widths
        .iter()
        .zip(cols)
        .filter(|(_, col)| col.resizable.unwrap_or(resizable_default))
        .map(|(width, col)| (col.width_range.max - width).max(0.0))
        .sum()
}

fn shrink_forward(
    widths: &mut [f32],
    cols: &[Column],
    offset: usize,
    resizable_default: bool,
    width_snapshot: &mut [f32],
    mut remaining: f32,
) -> f32 {
    for idx in 0..widths.len() {
        let col = &cols[idx];
        if !col.resizable.unwrap_or(resizable_default) {
            continue;
        }
        let slack = (widths[idx] - col.width_range.min).max(0.0);
        if slack <= 0.0 {
            continue;
        }
        let take = slack.min(remaining);
        widths[idx] -= take;
        width_snapshot[offset + idx] = widths[idx];
        remaining -= take;
        if remaining <= 0.0 {
            break;
        }
    }
    remaining
}

fn shrink_reverse(
    widths: &mut [f32],
    cols: &[Column],
    offset: usize,
    resizable_default: bool,
    width_snapshot: &mut [f32],
    mut remaining: f32,
) -> f32 {
    for idx in (0..widths.len()).rev() {
        let col = &cols[idx];
        if !col.resizable.unwrap_or(resizable_default) {
            continue;
        }
        let slack = (widths[idx] - col.width_range.min).max(0.0);
        if slack <= 0.0 {
            continue;
        }
        let take = slack.min(remaining);
        widths[idx] -= take;
        width_snapshot[offset + idx] = widths[idx];
        remaining -= take;
        if remaining <= 0.0 {
            break;
        }
    }
    remaining
}

fn grow_forward(
    widths: &mut [f32],
    cols: &[Column],
    offset: usize,
    resizable_default: bool,
    width_snapshot: &mut [f32],
    mut remaining: f32,
) -> f32 {
    for idx in 0..widths.len() {
        let col = &cols[idx];
        if !col.resizable.unwrap_or(resizable_default) {
            continue;
        }
        let room = (col.width_range.max - widths[idx]).max(0.0);
        if room <= 0.0 {
            continue;
        }
        let give = room.min(remaining);
        widths[idx] += give;
        width_snapshot[offset + idx] = widths[idx];
        remaining -= give;
        if remaining <= 0.0 {
            break;
        }
    }
    remaining
}

#[derive(Clone, Default)]
struct ColumnResizeDrag {
    column_index: usize,
    start_x: f32,
    start_widths: Vec<f32>,
}

impl<'a> TableBody<'a> {
    /// Access the contained [`egui::Ui`].
    ///
    /// You can use this to e.g. modify the [`egui::Style`] with [`egui::Ui::style_mut`].
    pub fn ui_mut(&mut self) -> &mut egui::Ui {
        self.layout.ui
    }

    /// Where in screen-space is the table body?
    pub fn max_rect(&self) -> Rect {
        self.layout
            .rect
            .translate(egui::vec2(0.0, self.scroll_offset_y()))
    }

    fn scroll_offset_y(&self) -> f32 {
        self.y_range.min - self.layout.rect.top()
    }

    /// Return a vector containing all column widths for this table body.
    ///
    /// This is primarily meant for use with [`TableBody::heterogeneous_rows`] in cases where row
    /// heights are expected to according to the width of one or more cells -- for example, if text
    /// is wrapped rather than clipped within the cell.
    pub fn widths(&self) -> &[f32] {
        self.widths
    }

    /// Add a single row with the given height.
    ///
    /// ⚠️ It is much more performant to use [`Self::rows`] or [`Self::heterogeneous_rows`],
    /// as those functions will only render the visible rows.
    pub fn row(&mut self, height: f32, add_row_content: impl FnOnce(TableRow<'a, '_>)) {
        let mut response: Option<Response> = None;
        let top_y = self.layout.cursor.y;
        add_row_content(TableRow {
            layout: &mut self.layout,
            columns: self.columns,
            widths: self.widths,
            max_used_widths: self.max_used_widths,
            row_index: self.row_index,
            col_index: 0,
            height,
            striped: self.striped && self.row_index.is_multiple_of(2),
            hovered: self.hovered_row_index == Some(self.row_index),
            selected: false,
            overline: false,
            content_bottom: None,
            response: &mut response,
        });
        self.capture_hover_state(&response, self.row_index);
        let bottom_y = self.layout.cursor.y;

        if Some(self.row_index) == self.scroll_to_row {
            *self.scroll_to_y_range = Some(Rangef::new(top_y, bottom_y));
        }

        self.row_index += 1;
    }

    /// Add many rows with same height.
    ///
    /// Is a lot more performant than adding each individual row as non visible rows must not be rendered.
    ///
    /// If you need many rows with different heights, use [`Self::heterogeneous_rows`] instead.
    ///
    /// ### Example
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// use lissajous::widgets::{TableBuilder, Column};
    /// TableBuilder::new(ui)
    ///     .column(Column::remainder().at_least(100.0))
    ///     .body(|mut body| {
    ///         let row_height = 18.0;
    ///         let num_rows = 10_000;
    ///         body.rows(row_height, num_rows, |mut row| {
    ///             let row_index = row.index();
    ///             row.col(|ui| {
    ///                 ui.label(format!("First column of row {row_index}"));
    ///             });
    ///         });
    ///     });
    /// # });
    /// ```
    pub fn rows(
        mut self,
        row_height_sans_spacing: f32,
        total_rows: usize,
        mut add_row_content: impl FnMut(TableRow<'_, '_>),
    ) {
        let spacing = self.layout.ui.spacing().item_spacing;
        let row_height_with_spacing = row_height_sans_spacing + spacing.y;

        if let Some(scroll_to_row) = self.scroll_to_row {
            let scroll_to_row = scroll_to_row.at_most(total_rows.saturating_sub(1)) as f32;
            *self.scroll_to_y_range = Some(Rangef::new(
                self.layout.cursor.y + scroll_to_row * row_height_with_spacing,
                self.layout.cursor.y + (scroll_to_row + 1.0) * row_height_with_spacing,
            ));
        }

        let scroll_offset_y = self
            .scroll_offset_y()
            .min(total_rows as f32 * row_height_with_spacing);
        let max_height = self.y_range.span();
        let mut min_row = 0;

        if scroll_offset_y > 0.0 {
            min_row = (scroll_offset_y / row_height_with_spacing).floor() as usize;
            self.add_buffer(min_row as f32 * row_height_with_spacing);
        }

        let max_row =
            ((scroll_offset_y + max_height) / row_height_with_spacing).ceil() as usize + 1;
        let max_row = max_row.min(total_rows);

        for row_index in min_row..max_row {
            let mut response: Option<Response> = None;
            add_row_content(TableRow {
                layout: &mut self.layout,
                columns: self.columns,
                widths: self.widths,
                max_used_widths: self.max_used_widths,
                row_index,
                col_index: 0,
                height: row_height_sans_spacing,
                striped: self.striped && (row_index + self.row_index).is_multiple_of(2),
                hovered: self.hovered_row_index == Some(row_index),
                selected: false,
                overline: false,
                content_bottom: None,
                response: &mut response,
            });
            self.capture_hover_state(&response, row_index);
        }

        if total_rows - max_row > 0 {
            let skip_height = (total_rows - max_row) as f32 * row_height_with_spacing;
            self.add_buffer(skip_height - spacing.y);
        }
    }

    /// Add rows with varying heights.
    ///
    /// This takes a very slight performance hit compared to [`TableBody::rows`] due to the need to
    /// iterate over all row heights in order to calculate the virtual table height above and below the
    /// visible region, but it is many orders of magnitude more performant than adding individual
    /// heterogeneously-sized rows using [`TableBody::row`] at the cost of the additional complexity
    /// that comes with pre-calculating row heights and representing them as an iterator.
    ///
    /// ### Example
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// use lissajous::widgets::{TableBuilder, Column};
    /// TableBuilder::new(ui)
    ///     .column(Column::remainder().at_least(100.0))
    ///     .body(|mut body| {
    ///         let row_heights: Vec<f32> = vec![60.0, 18.0, 31.0, 240.0];
    ///         body.heterogeneous_rows(row_heights.into_iter(), |mut row| {
    ///             let row_index = row.index();
    ///             let thick = row_index % 6 == 0;
    ///             row.col(|ui| {
    ///                 ui.centered_and_justified(|ui| {
    ///                     ui.label(row_index.to_string());
    ///                 });
    ///             });
    ///         });
    ///     });
    /// # });
    /// ```
    pub fn heterogeneous_rows(
        mut self,
        heights: impl Iterator<Item = f32>,
        mut add_row_content: impl FnMut(TableRow<'_, '_>),
    ) {
        let spacing = self.layout.ui.spacing().item_spacing;
        let mut enumerated_heights = heights.enumerate();

        let max_height = self.y_range.span();
        let scroll_offset_y = self.scroll_offset_y() as f64;

        let scroll_to_y_range_offset = self.layout.cursor.y as f64;

        let mut cursor_y: f64 = 0.0;

        // Skip the invisible rows, and populate the first non-virtual row.
        for (row_index, row_height) in &mut enumerated_heights {
            let old_cursor_y = cursor_y;
            cursor_y += (row_height + spacing.y) as f64;

            if Some(row_index) == self.scroll_to_row {
                *self.scroll_to_y_range = Some(Rangef::new(
                    (scroll_to_y_range_offset + old_cursor_y) as f32,
                    (scroll_to_y_range_offset + cursor_y) as f32,
                ));
            }

            if cursor_y >= scroll_offset_y {
                // This row is visible:
                self.add_buffer(old_cursor_y as f32); // skip all the invisible rows
                let mut response: Option<Response> = None;
                add_row_content(TableRow {
                    layout: &mut self.layout,
                    columns: self.columns,
                    widths: self.widths,
                    max_used_widths: self.max_used_widths,
                    row_index,
                    col_index: 0,
                    height: row_height,
                    striped: self.striped && (row_index + self.row_index).is_multiple_of(2),
                    hovered: self.hovered_row_index == Some(row_index),
                    selected: false,
                    overline: false,
                    content_bottom: None,
                    response: &mut response,
                });
                self.capture_hover_state(&response, row_index);
                break;
            }
        }

        // populate visible rows:
        for (row_index, row_height) in &mut enumerated_heights {
            let top_y = cursor_y;
            let mut response: Option<Response> = None;
            add_row_content(TableRow {
                layout: &mut self.layout,
                columns: self.columns,
                widths: self.widths,
                max_used_widths: self.max_used_widths,
                row_index,
                col_index: 0,
                height: row_height,
                striped: self.striped && (row_index + self.row_index).is_multiple_of(2),
                hovered: self.hovered_row_index == Some(row_index),
                overline: false,
                selected: false,
                content_bottom: None,
                response: &mut response,
            });
            self.capture_hover_state(&response, row_index);
            cursor_y += (row_height + spacing.y) as f64;

            if Some(row_index) == self.scroll_to_row {
                *self.scroll_to_y_range = Some(Rangef::new(
                    (scroll_to_y_range_offset + top_y) as f32,
                    (scroll_to_y_range_offset + cursor_y) as f32,
                ));
            }

            if cursor_y > scroll_offset_y + max_height as f64 {
                break;
            }
        }

        // calculate height below the visible table range:
        let mut height_below_visible: f64 = 0.0;
        for (row_index, row_height) in enumerated_heights {
            height_below_visible += (row_height + spacing.y) as f64;

            let top_y = cursor_y;
            cursor_y += (row_height + spacing.y) as f64;
            if Some(row_index) == self.scroll_to_row {
                *self.scroll_to_y_range = Some(Rangef::new(
                    (scroll_to_y_range_offset + top_y) as f32,
                    (scroll_to_y_range_offset + cursor_y) as f32,
                ));
            }
        }

        if self.scroll_to_row.is_some() && self.scroll_to_y_range.is_none() {
            // Catch desire to scroll past the end:
            *self.scroll_to_y_range =
                Some(Rangef::point((scroll_to_y_range_offset + cursor_y) as f32));
        }

        if height_below_visible > 0.0 {
            // we need to add a buffer to allow the table to
            // accurately calculate the scrollbar position
            self.add_buffer(height_below_visible as f32);
        }
    }

    // Create a table row buffer of the given height to represent the non-visible portion of the
    // table.
    fn add_buffer(&mut self, height: f32) {
        self.layout.skip_space(egui::vec2(0.0, height));
    }

    // Capture the hover information for the just created row. This is used in the next render
    // to ensure that the entire row is highlighted.
    fn capture_hover_state(&self, response: &Option<Response>, row_index: usize) {
        let is_row_hovered = response.as_ref().is_some_and(|r| r.hovered());
        if is_row_hovered {
            self.layout
                .ui
                .data_mut(|data| data.insert_temp(self.hovered_row_index_id, row_index));
        }
    }
}

impl Drop for TableBody<'_> {
    fn drop(&mut self) {
        self.layout.allocate_rect();
    }
}

/// The row of a table.
/// Is created by [`TableRow`] for each created [`TableBody::row`] or each visible row in rows created by calling [`TableBody::rows`].
pub struct TableRow<'a, 'b> {
    layout: &'b mut StripLayout<'a>,
    columns: &'b [Column],
    widths: &'b [f32],

    /// grows during building with the maximum widths
    max_used_widths: &'b mut [f32],

    row_index: usize,
    col_index: usize,
    height: f32,

    striped: bool,
    hovered: bool,
    selected: bool,
    overline: bool,

    content_bottom: Option<&'b mut f32>,

    response: &'b mut Option<Response>,
}

impl TableRow<'_, '_> {
    /// Add the contents of a column on this row (i.e. a cell).
    ///
    /// Returns the used space (`min_rect`) plus the [`Response`] of the whole cell.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn col(&mut self, add_cell_contents: impl FnOnce(&mut Ui)) -> (Rect, Response) {
        let col_index = self.col_index;

        let clip = self.columns.get(col_index).is_some_and(|c| c.clip);
        let auto_size_this_frame = self
            .columns
            .get(col_index)
            .is_some_and(|c| c.auto_size_this_frame);

        let width = if let Some(width) = self.widths.get(col_index) {
            self.col_index += 1;
            *width
        } else {
            log_or_panic!(
                "Added more `Table` columns than were pre-allocated ({} pre-allocated)",
                self.widths.len()
            );
            8.0 // anything will look wrong, so pick something that is obviously wrong
        };

        let width = CellSize::Absolute(width);
        let height = CellSize::Absolute(self.height);

        let flags = StripLayoutFlags {
            clip,
            striped: self.striped,
            hovered: self.hovered,
            selected: self.selected,
            overline: self.overline,
            sizing_pass: auto_size_this_frame || self.layout.ui.is_sizing_pass(),
        };

        let (used_rect, response) = self.layout.add(
            flags,
            width,
            height,
            egui::Id::new((self.row_index, col_index)),
            add_cell_contents,
        );

        if let Some(content_bottom) = self.content_bottom.as_deref_mut() {
            *content_bottom = content_bottom.max(used_rect.bottom());
        }

        if let Some(max_w) = self.max_used_widths.get_mut(col_index) {
            *max_w = max_w.max(used_rect.width());
        }

        *self.response = Some(
            self.response
                .as_ref()
                .map_or(response.clone(), |r| r.union(response.clone())),
        );

        (used_rect, response)
    }

    /// Set the selection highlight state for cells added after a call to this function.
    #[inline]
    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    /// Set the hovered highlight state for cells added after a call to this function.
    #[inline]
    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    /// Set the overline state for this row. The overline is a line above the row,
    /// usable for e.g. visually grouping rows.
    #[inline]
    pub fn set_overline(&mut self, overline: bool) {
        self.overline = overline;
    }

    /// Returns a union of the [`Response`]s of the cells added to the row up to this point.
    ///
    /// You need to add at least one row to the table before calling this function.
    pub fn response(&self) -> Response {
        self.response
            .clone()
            .expect("Should only be called after `col`")
    }

    /// Returns the index of the row.
    #[inline]
    pub fn index(&self) -> usize {
        self.row_index
    }

    /// Returns the index of the column. Incremented after a column is added.
    #[inline]
    pub fn col_index(&self) -> usize {
        self.col_index
    }
}

impl Drop for TableRow<'_, '_> {
    #[inline]
    fn drop(&mut self) {
        self.layout.end_line();
    }
}
