use std::cell::RefCell;
use std::fmt::Write;
use std::ops::Range;
use std::sync::LazyLock;

use eframe::egui;
use typst::syntax::Span;

use super::typst_render::painter;
use super::typst_render::world::{FileResolver, GorbieWorld, UNAVAILABLE_IMAGE};
use crate::themes::colorhash::RAL_CATEGORICAL;
use crate::themes::ral::RAL_COLORS;

/// Persistent state for Typst compilation and rendering.
///
/// Holds a `GorbieWorld` (benefits from comemo's incremental memoization)
/// Glyph meshes are cached via comemo.
struct TypstState {
    world: GorbieWorld,
}

impl TypstState {
    fn new() -> Self {
        Self {
            world: GorbieWorld::new(),
        }
    }
}

thread_local! {
    static TYPST_STATE: RefCell<TypstState> = RefCell::new(TypstState::new());
}

/// Render a full Typst document string into the UI.
///
/// The source is compiled as-is — callers are responsible for any
/// `#set page(...)` / `#set text(...)` preamble they want.
pub fn typst(ui: &mut egui::Ui, source: &str) {
    TYPST_STATE.with(|state| {
        let mut state = state.borrow_mut();
        render_typst(ui, &mut state, source, 0, None);
    });
}

/// Render an inline math expression: `$<expr>$`.
pub fn typst_math_inline(ui: &mut egui::Ui, expr: &str) {
    let fg = typst_rgb(ui.visuals().text_color());
    let size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(15.0, |d| d.size);
    let preamble = format!(
        "#set page(width: auto, height: auto, margin: 0pt)\n\
         #set text(size: {size}pt, fill: {fg})\n"
    );
    let source = format!("{preamble}${expr}$");
    TYPST_STATE.with(|state| {
        let mut state = state.borrow_mut();
        render_typst(ui, &mut state, &source, preamble.len(), None);
    });
}

/// Render a display-mode math expression (centered block):
/// `$ <expr> $` (with spaces for display mode in Typst).
pub fn typst_math_display(ui: &mut egui::Ui, expr: &str) {
    let fg = typst_rgb(ui.visuals().text_color());
    let size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(15.0, |d| d.size);
    let preamble = format!(
        "#set page(width: auto, height: auto, margin: 0pt)\n\
         #set text(size: {size}pt, fill: {fg})\n"
    );
    let source = format!("{preamble}$ {expr} $");
    TYPST_STATE.with(|state| {
        let mut state = state.borrow_mut();
        render_typst(ui, &mut state, &source, preamble.len(), None);
    });
}

/// Returns a closure suitable for use as a `render_math_fn` in gorbie-commonmark.
///
/// The closure renders `$...$` (inline) or `$$...$$` (display) math
/// using Typst compilation + direct vector painting.
pub fn typst_math_fn() -> impl Fn(&mut egui::Ui, &str, bool) {
    |ui: &mut egui::Ui, expr: &str, display: bool| {
        if display {
            typst_math_display(ui, expr);
        } else {
            typst_math_inline(ui, expr);
        }
    }
}

/// Render Typst content with a GORBIE-appropriate preamble automatically injected.
///
/// Sets page width to the available UI width with GRID_EDGE_PAD margins,
/// body text size from the UI style, and IosevkaGorbie as the default font.
/// The Typst page margin replaces the grid edge padding so the interactive
/// area extends to the card edges (better for drag-selection).
pub fn typst_with_preamble(ui: &mut egui::Ui, content: &str) {
    typst_with_preamble_inner(ui, content, None);
}

/// [`typst_with_preamble`], with `files` answering the files the content
/// names (`image("files:…")` and the like). See [`FileResolver`].
pub fn typst_with_preamble_and_files(ui: &mut egui::Ui, content: &str, files: &FileResolver<'_>) {
    typst_with_preamble_inner(ui, content, Some(files));
}

fn typst_with_preamble_inner(ui: &mut egui::Ui, content: &str, files: Option<&FileResolver<'_>>) {
    let width = ui.available_width();
    let pad = crate::card_ctx::GRID_EDGE_PAD;
    let body_size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(15.0, |d| d.size);
    let palette = ral_preamble(ui);

    let preamble = format!(
        "{palette}\
         #set page(width: {width}pt, height: auto, margin: (x: {pad}pt, top: {pad}pt, bottom: {pad}pt))\n\
         #set text(size: {body_size}pt, font: \"IosevkaGorbie\", fill: ral-fg)\n\
         #set table(stroke: ral-fg)\n\
         #set line(stroke: ral-fg)\n\
         #set rect(stroke: ral-fg)\n\
         #set circle(stroke: ral-fg)\n\
         #set ellipse(stroke: ral-fg)\n\
         #set polygon(stroke: ral-fg)\n\
         #set path(stroke: ral-fg)\n\
         #show link: set text(fill: ral-blue)\n"
    );
    let source = format!("{preamble}{content}");
    TYPST_STATE.with(|state| {
        let mut state = state.borrow_mut();
        render_typst(ui, &mut state, &source, preamble.len(), files);
    });
}

/// Render a Typst document with GORBIE grid defaults.
///
/// Automatically sets page width to the available width with edge padding
/// as Typst page margins, IosevkaGorbie as the body font, and the current
/// body text size. The widget fills the full card width so drag-selection
/// extends to the card edges.
///
/// ```ignore
/// typst!(ctx, "= Hello World\nThis is *Typst* in GORBIE.");
/// typst!(ctx, "$ {} $", some_math_expr);
/// ```
#[macro_export]
macro_rules! typst {
    ($ctx:expr, $fmt:expr $(, $args:expr)*) => {
        {
            let text = format!($fmt $(, $args)*);
            $crate::widgets::typst_widget::typst_with_preamble($ctx, &text);
        }
    };
}

/// Format a `Color32` as a Typst `rgb("#RRGGBB")` literal.
fn typst_rgb(c: egui::Color32) -> String {
    format!("rgb(\"#{:02X}{:02X}{:02X}\")", c.r(), c.g(), c.b())
}

/// RAL color names for the categorical palette, matching `RAL_CATEGORICAL` order.
const RAL_CAT_NAMES: &[&str] = &[
    "yellow", "orange", "pink", "red", "violet", "blue", "sky", "water", "lime", "mint", "green",
    "teal",
];

/// The static part of the GORBIE Typst preamble: grid constants, full 272-color
/// RAL lookup dictionary, and semantic named aliases. Generated once, reused.
static RAL_TYPST_STATIC: LazyLock<String> = LazyLock::new(|| {
    use crate::card_ctx::{GRID_COLUMNS, GRID_COL_WIDTH, GRID_GUTTER, GRID_ROW_MODULE};

    let mut s = String::with_capacity(16384);

    // Grid system: matches the Rust-side 12-column layout exactly.
    let _ = writeln!(s, "#let grid-col = {GRID_COL_WIDTH}pt");
    let _ = writeln!(s, "#let grid-gutter = {GRID_GUTTER}pt");
    let _ = writeln!(s, "#let grid-columns = {GRID_COLUMNS}");
    let _ = writeln!(s, "#let grid-row = {GRID_ROW_MODULE}pt");
    let _ = writeln!(
        s,
        "#let grid-span(n) = n * {GRID_COL_WIDTH}pt + (n - 1) * {GRID_GUTTER}pt"
    );

    // Full RAL dictionary: ral(1003) → rgb("#F9A800")
    let _ = writeln!(s, "#let ral-table = (");
    for &(code, _, color) in RAL_COLORS {
        let _ = writeln!(
            s,
            "  \"{code}\": rgb(\"#{:02X}{:02X}{:02X}\"),",
            color.r(),
            color.g(),
            color.b()
        );
    }
    let _ = writeln!(s, ")");
    let _ = writeln!(s, "#let ral(num) = ral-table.at(str(num))");
    // Semantic aliases from the dictionary
    let _ = writeln!(s, "#let ral-accent = ral(2009)");
    for (&code, &name) in RAL_CATEGORICAL.iter().zip(RAL_CAT_NAMES) {
        let _ = writeln!(s, "#let ral-{name} = ral({code})");
    }
    s
});

/// Generate a Typst preamble with GORBIE grid constants and RAL color palette.
///
/// Injects:
/// - `grid-col`, `grid-gutter`, `grid-columns`, `grid-row` — grid constants
/// - `grid-span(n)` — width of `n` grid columns including inner gutters
/// - `ral(num)` — lookup any of the 272 RAL Classic colors by number
/// - `ral-fg` / `ral-bg` — current theme foreground and background
/// - `ral-accent` — RAL 2009 Traffic orange
/// - The 12 categorical colors (`ral-yellow`, `ral-blue`, …)
///
/// Included automatically by [`typst_with_preamble`] and the [`typst!`] macro.
/// Call manually when building custom source strings that need palette access.
pub fn ral_preamble(ui: &egui::Ui) -> String {
    let fg = ui.visuals().text_color();
    let bg = ui.visuals().panel_fill;

    let mut s = RAL_TYPST_STATIC.clone();
    let _ = writeln!(s, "#let ral-fg = {}", typst_rgb(fg));
    let _ = writeln!(s, "#let ral-bg = {}", typst_rgb(bg));
    s
}

// ── Selection ─────────────────────────────────────────────────────────

/// Persistent selection state for a single Typst render area.
#[derive(Clone, Default)]
struct TypstSelection {
    /// Frame-relative position where the drag started.
    anchor: Option<egui::Pos2>,
    /// Frame-relative position at the current pointer.
    cursor: Option<egui::Pos2>,
    /// Exact glyph range override (set by double-click).
    glyph_override: Option<Range<usize>>,
}

impl TypstSelection {
    /// Compute the glyph index range from anchor and cursor positions.
    ///
    /// Uses 2-point selection: finds the nearest glyph to each endpoint,
    /// then selects all glyphs between them in document order. This gives
    /// natural text selection across line boundaries (no box artifacts).
    fn range(&self, chars: &[painter::PositionedChar]) -> Option<Range<usize>> {
        if let Some(ref r) = self.glyph_override {
            return Some(r.clone());
        }

        let anchor = self.anchor?;
        let cursor = self.cursor?;

        let a = nearest_glyph(chars, anchor)?;
        let b = nearest_glyph(chars, cursor)?;

        let lo = a.min(b);
        let hi = a.max(b);
        Some(lo..hi + 1)
    }
}

/// Find the glyph whose center is nearest to `pos`.
fn nearest_glyph(chars: &[painter::PositionedChar], pos: egui::Pos2) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = a.rect.center().distance_sq(pos);
            let db = b.rect.center().distance_sq(pos);
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
}

/// Double-click: AST-aware selection at the clicked position.
///
/// Finds the nearest glyph, then walks up the source AST from its span
/// to find the first "interesting" structural boundary (equation, strong,
/// heading, list item, table cell). Falls back to word boundaries for
/// plain text.
fn double_click_range(
    source: &typst::syntax::Source,
    chars: &[painter::PositionedChar],
    pos: egui::Pos2,
) -> Option<Range<usize>> {
    use typst::syntax::SyntaxKind;

    let idx = nearest_glyph(chars, pos)?;
    let (span, _) = chars[idx].span;
    let node = source.find(span)?;

    // Walk up to the first structural node.
    let mut current = &node;
    loop {
        let dominated_by = |k: SyntaxKind| {
            matches!(
                k,
                SyntaxKind::Strong
                    | SyntaxKind::Emph
                    | SyntaxKind::Equation
                    | SyntaxKind::Heading
                    | SyntaxKind::ListItem
                    | SyntaxKind::EnumItem
                    | SyntaxKind::TermItem
                    | SyntaxKind::FuncCall
                    | SyntaxKind::Markup
            )
        };
        if dominated_by(current.kind()) {
            // Found a structural node — select glyphs whose source
            // positions fall within this node's range.
            let nr = current.range();
            let lo = chars.iter().position(|ch| {
                let (s, off) = ch.span;
                source
                    .range(s)
                    .map_or(false, |r| r.start + (off as usize) >= nr.start)
            })?;
            let hi = chars.iter().rposition(|ch| {
                let (s, off) = ch.span;
                source
                    .range(s)
                    .map_or(false, |r| r.start + (off as usize) < nr.end)
            })?;
            return Some(lo..hi + 1);
        }
        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }

    // Fallback: word boundaries.
    let is_word = |i: usize| -> bool {
        let ch = chars[i].text.chars().next().unwrap_or(' ');
        ch.is_alphanumeric() || ch == '_' || ch == '-'
    };
    let mut lo = idx;
    while lo > 0 && is_word(lo - 1) {
        lo -= 1;
    }
    let mut hi = idx;
    while hi + 1 < chars.len() && is_word(hi + 1) {
        hi += 1;
    }
    Some(lo..hi + 1)
}

/// Selection result: highlight and copy from one tree walk.
#[derive(Clone)]
struct SelectionResult {
    /// Which glyphs to highlight.
    sel_set: Vec<bool>,
    /// Source byte range for copy (contiguous min..max).
    copy_range: Option<Range<usize>>,
}

/// Compute selection: geometric range + tree walk for both highlight and copy.
///
/// One tree walk produces both:
/// - Highlight: geometric selection + detached glyphs from fully-selected nodes
/// - Copy range: structural source spans from fully-selected nodes + per-glyph
///   spans for partial content, merged into a contiguous min..max
///
/// Highlight and copy derive from the same walk — they can't diverge.
fn compute_selection(
    source: &typst::syntax::Source,
    chars: &[painter::PositionedChar],
    glyph_range: &Option<Range<usize>>,
) -> SelectionResult {
    let Some(ref range) = glyph_range else {
        return SelectionResult {
            sel_set: Vec::new(),
            copy_range: None,
        };
    };

    let mut min_byte = usize::MAX;
    let mut max_byte = 0usize;

    // Step 1: Geometric selection → source byte range.
    for i in range.clone() {
        if i >= chars.len() {
            continue;
        }
        let (span, offset) = chars[i].span;
        if let Some(node_range) = source.range(span) {
            let glyph_start = node_range.start + offset as usize;
            let glyph_end = (glyph_start + chars[i].text.len()).min(node_range.end);
            min_byte = min_byte.min(glyph_start);
            max_byte = max_byte.max(glyph_end);
        }
    }

    // Step 2: AST walk — expand to structural boundaries.
    expand_copy_from_ast(
        source,
        chars,
        min_byte,
        max_byte,
        &mut min_byte,
        &mut max_byte,
    );

    // Step 3: Highlight glyphs whose source position falls within copy_range.
    let mut sel_set = vec![false; chars.len()];
    if min_byte < max_byte {
        for (i, ch) in chars.iter().enumerate() {
            let (span, offset) = ch.span;
            if let Some(node_range) = source.range(span) {
                let glyph_start = node_range.start + offset as usize;
                let glyph_end = (glyph_start + ch.text.len()).min(node_range.end);
                if glyph_start < max_byte && glyph_end > min_byte {
                    sel_set[i] = true;
                }
            }
        }
    }

    SelectionResult {
        sel_set,
        copy_range: if min_byte < max_byte {
            Some(min_byte..max_byte)
        } else {
            None
        },
    }
}

/// Expand the copy range using source AST LCA (lowest common ancestor).
///
/// Finds the AST nodes at the min and max byte positions, walks up to
/// their LCA, and expands the range to cover the LCA's children between
/// them. This naturally includes structural markup (list markers, bold
/// asterisks, heading prefixes) and inter-child whitespace.
fn expand_copy_from_ast(
    source: &typst::syntax::Source,
    chars: &[painter::PositionedChar],
    in_min: usize,
    in_max: usize,
    min_byte: &mut usize,
    max_byte: &mut usize,
) {
    if in_min >= in_max {
        return;
    }
    *min_byte = in_min;
    *max_byte = in_max;

    // Find spans near min and max byte positions.
    let find_span_near = |target: usize| -> Option<Span> {
        chars
            .iter()
            .filter_map(|ch| {
                let (span, offset) = ch.span;
                let nr = source.range(span)?;
                let pos = nr.start + offset as usize;
                Some((span, pos.abs_diff(target)))
            })
            .filter(|(_, dist)| *dist < 1000) // reasonable proximity
            .min_by_key(|(_, dist)| *dist)
            .map(|(span, _)| span)
    };

    let Some(lo_span) = find_span_near(in_min) else {
        return;
    };
    let Some(hi_span) = find_span_near(in_max.saturating_sub(1)) else {
        return;
    };

    // Collapse upward from EACH endpoint: walk up from the leaf node,
    // expanding whenever all source glyphs within the parent's range
    // are already covered. This captures structural markup at each end
    // independently (`- ` for each list item, `*` for bold, etc.).
    let collapse_endpoint = |source: &typst::syntax::Source,
                             chars: &[painter::PositionedChar],
                             span: Span,
                             min_byte: &mut usize,
                             max_byte: &mut usize| {
        let Some(node) = source.find(span) else {
            return;
        };
        let mut current = &node;
        loop {
            let Some(parent) = current.parent() else {
                break;
            };
            let pr = parent.range();

            let all_covered = chars
                .iter()
                .filter_map(|ch| {
                    let (span, offset) = ch.span;
                    let nr = source.range(span)?;
                    Some(nr.start + offset as usize)
                })
                .filter(|&pos| pos >= pr.start && pos < pr.end)
                .all(|pos| pos >= *min_byte && pos < *max_byte);

            if all_covered {
                *min_byte = (*min_byte).min(pr.start);
                *max_byte = (*max_byte).max(pr.end);
                current = parent;
            } else {
                break;
            }
        }
    };

    collapse_endpoint(source, chars, lo_span, min_byte, max_byte);
    collapse_endpoint(source, chars, hi_span, min_byte, max_byte);

    // Trim trailing whitespace.
    while *max_byte > *min_byte && source.text().as_bytes()[*max_byte - 1].is_ascii_whitespace() {
        *max_byte -= 1;
    }

    // Include preceding `#` — Typst splits `#expr` into sibling nodes.
    if *min_byte > 0 && source.text().as_bytes()[*min_byte - 1] == b'#' {
        *min_byte -= 1;
    }
}

// ── Error rendering ───────────────────────────────────────────────────

/// Render Typst compilation errors as a rustc-style diagnostic block.
fn render_typst_errors(
    ui: &mut egui::Ui,
    source: &str,
    preamble_len: usize,
    diags: &[super::typst_render::world::TypstDiag],
) {
    let pad = crate::card_ctx::GRID_EDGE_PAD;
    let frame = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(pad as i8, pad as i8));
    frame.show(ui, |ui| {
        render_typst_errors_inner(ui, source, preamble_len, diags);
    });
}

fn render_typst_errors_inner(
    ui: &mut egui::Ui,
    source: &str,
    preamble_len: usize,
    diags: &[super::typst_render::world::TypstDiag],
) {
    use typst::diag::Severity;

    let user_source = &source[preamble_len..];
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(user_source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let byte_to_line = |byte: usize| -> usize {
        line_starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    };
    let line_text = |line: usize| -> &str {
        let start = line_starts[line];
        let end = if line + 1 < line_starts.len() {
            line_starts[line + 1].saturating_sub(1)
        } else {
            user_source.len()
        };
        &user_source[start..end]
    };

    let mono = egui::FontId::monospace(12.0);
    let error_color = egui::Color32::from_rgb(0xFF, 0x44, 0x44);
    let warning_color = egui::Color32::from_rgb(0xFF, 0xCC, 0x22);
    let hint_color = egui::Color32::from_rgb(0x55, 0xBB, 0xFF);
    let line_num_color = ui.visuals().weak_text_color();
    let source_color = ui.visuals().text_color();

    let fmt = |color: egui::Color32| egui::text::TextFormat {
        font_id: mono.clone(),
        color,
        ..Default::default()
    };

    let label_no_wrap = |ui: &mut egui::Ui, mut job: egui::text::LayoutJob| {
        job.wrap = egui::text::TextWrapping {
            max_rows: 1,
            break_anywhere: false,
            overflow_character: Some('…'),
            ..Default::default()
        };
        ui.label(job);
    };

    let prev_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;

    // Deduplicate: keep only the first diagnostic per source line.
    let mut seen_lines = std::collections::HashSet::new();
    let diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            let line = d
                .span_range
                .as_ref()
                .filter(|r| r.start >= preamble_len)
                .map(|r| byte_to_line(r.start - preamble_len));
            match line {
                Some(l) => seen_lines.insert(l),
                None => true,
            }
        })
        .collect();

    for diag in diags {
        let color = match diag.severity {
            Severity::Error => error_color,
            Severity::Warning => warning_color,
        };
        let prefix = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };

        ui.add_space(4.0);
        let mut job = egui::text::LayoutJob::default();
        job.append(&format!("{prefix}: "), 0.0, fmt(color));
        job.append(&diag.message, 0.0, fmt(source_color));
        ui.label(job);

        if let Some(range) = &diag.span_range {
            if range.start >= preamble_len {
                let user_start = range.start - preamble_len;
                let user_end = (range.end - preamble_len).min(user_source.len());
                let err_line = byte_to_line(user_start);
                let line_num = err_line + 1;
                let gutter_width = format!("{line_num}").len().max(3);
                let text = line_text(err_line);

                let mut bar_job = egui::text::LayoutJob::default();
                bar_job.append(
                    &format!("{:>gutter_width$} ┃", ""),
                    0.0,
                    fmt(line_num_color),
                );
                label_no_wrap(ui, bar_job);

                let mut line_job = egui::text::LayoutJob::default();
                line_job.append(
                    &format!("{line_num:>gutter_width$} ┃ "),
                    0.0,
                    fmt(line_num_color),
                );
                line_job.append(text, 0.0, fmt(source_color));
                label_no_wrap(ui, line_job);

                let line_start = line_starts[err_line];
                let col_start = user_start - line_start;
                let col_end = (user_end - line_start).max(col_start + 1);
                let underline: String = " ".repeat(col_start) + &"─".repeat(col_end - col_start);
                let mut ul_job = egui::text::LayoutJob::default();
                ul_job.append(
                    &format!("{:>gutter_width$} ┃ ", ""),
                    0.0,
                    fmt(line_num_color),
                );
                ul_job.append(&underline, 0.0, fmt(color));
                label_no_wrap(ui, ul_job);
            }
        }

        for hint in &diag.hints {
            let mut hint_job = egui::text::LayoutJob::default();
            hint_job.append("  hint: ", 0.0, fmt(hint_color));
            hint_job.append(hint, 0.0, fmt(source_color));
            ui.label(hint_job);
        }
    }

    ui.spacing_mut().item_spacing.y = prev_spacing;
}

// ── Images ────────────────────────────────────────────────────────────

/// Longest texture side for one document image. A larger picture is
/// downscaled once on upload; wiki pages show photos far below this size.
const MAX_IMAGE_TEXTURE_SIDE: usize = 2048;

/// Render passes an unused image texture is kept before it is freed.
const IMAGE_TEXTURE_IDLE_PASSES: u64 = 600;

/// GPU textures for document images, keyed by Typst's image hash (which
/// covers the bytes and decode options), with the pass that last used them.
#[derive(Clone, Default)]
struct ImageTextures(
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u128, (egui::TextureHandle, u64)>>>,
);

/// The texture for a raster image, or `None` for anything that is not
/// painted as pixels (the unavailable placeholder, SVG, PDF).
fn image_texture(
    ctx: &egui::Context,
    image: &typst::visualize::Image,
) -> Option<egui::TextureHandle> {
    use typst::visualize::{ImageKind, ImageScaling};

    let ImageKind::Raster(raster) = image.kind() else {
        return None;
    };
    if raster.data().as_slice() == UNAVAILABLE_IMAGE {
        return None;
    }
    let key = typst::utils::hash128(image);
    let pass = ctx.cumulative_pass_nr();
    let textures = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<ImageTextures>(egui::Id::new("gorbie_typst_image_textures"))
            .clone()
    });
    let mut textures = textures
        .0
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some((texture, used)) = textures.get_mut(&key) {
        *used = pass;
        return Some(texture.clone());
    }
    textures.retain(|_, (_, used)| pass.saturating_sub(*used) <= IMAGE_TEXTURE_IDLE_PASSES);

    let max_side = ctx
        .input(|i| i.max_texture_side)
        .min(MAX_IMAGE_TEXTURE_SIDE) as u32;
    let dynamic = raster.dynamic();
    let rgba = if dynamic.width() > max_side || dynamic.height() > max_side {
        dynamic.thumbnail(max_side, max_side).to_rgba8()
    } else {
        dynamic.to_rgba8()
    };
    let pixels = egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    );
    let options = match image.scaling() {
        typst::foundations::Smart::Custom(ImageScaling::Pixelated) => egui::TextureOptions::NEAREST,
        _ => egui::TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..egui::TextureOptions::LINEAR
        },
    };
    let texture = ctx.load_texture(format!("typst-image-{key:032x}"), pixels, options);
    textures.insert(key, (texture.clone(), pass));
    Some(texture)
}

/// Paint one placed image: its pixels on the transformed quad, or a
/// labelled box when the pixels are unavailable or not raster.
fn paint_image(ui: &egui::Ui, placed: &painter::PositionedImage, offset: egui::Vec2) {
    let corners = placed.corners.map(|corner| corner + offset);
    let bounds = egui::Rect::from_points(&corners);
    if !ui.is_rect_visible(bounds) {
        return;
    }
    if let Some(texture) = image_texture(ui.ctx(), &placed.image) {
        let mut mesh = egui::Mesh::with_texture(texture.id());
        let uvs = [
            egui::pos2(0.0, 0.0),
            egui::pos2(1.0, 0.0),
            egui::pos2(1.0, 1.0),
            egui::pos2(0.0, 1.0),
        ];
        for (pos, uv) in corners.into_iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv,
                color: egui::Color32::WHITE,
            });
        }
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        ui.painter().add(egui::Shape::mesh(mesh));
        return;
    }
    let color = ui.visuals().weak_text_color();
    let painter = ui
        .painter()
        .with_clip_rect(ui.clip_rect().intersect(bounds));
    painter.rect_stroke(
        bounds,
        2.0,
        egui::Stroke::new(1.0_f32, color),
        egui::StrokeKind::Inside,
    );
    let label = match placed.image.alt() {
        Some(alt) if !alt.trim().is_empty() => format!("image unavailable: {alt}"),
        _ => "image unavailable".to_owned(),
    };
    let galley = painter.layout(
        label,
        egui::FontId::proportional(12.0),
        color,
        (bounds.width() - 8.0).max(8.0),
    );
    let pos = bounds.center() - galley.size() / 2.0;
    painter.galley(pos, galley, color);
}

// ── Main render ───────────────────────────────────────────────────────

fn render_typst(
    ui: &mut egui::Ui,
    state: &mut TypstState,
    source: &str,
    preamble_len: usize,
    files: Option<&FileResolver<'_>>,
) {
    state.world.set_source(source.to_string());

    let compiled = match files {
        Some(files) => state.world.compile_with_files(files),
        None => state.world.compile(),
    };
    let doc = match compiled {
        Ok(doc) => doc,
        Err(diags) => {
            render_typst_errors(ui, source, preamble_len, &diags);
            return;
        }
    };

    // Open a notebook-wide search session. This makes the search bar
    // appear automatically and lets us feed per-page matches back so
    // the bar can show counts + drive prev/next navigation.
    let mut search = crate::search::new_session(ui.ctx().clone());
    let needle = search.query().to_lowercase();
    let search_active = !needle.is_empty();

    // Pre-compute the list of source byte ranges that match the needle
    // ONCE for the whole document (instead of re-finding per page).
    // Each entry is `(start, end)` in source bytes; ranges are sorted
    // and disjoint so we can binary-search into them later.
    let match_ranges: Vec<std::ops::Range<usize>> = if search_active {
        let source_obj = state.world.main_source();
        let source_text = source_obj.text();
        let lower = source_text.to_lowercase();
        // If lowercasing changed the byte count (rare — e.g. German ß),
        // the byte-indices in `lower` no longer map onto `source_text`.
        // Skip search rather than misalign.
        if lower.len() == source_text.len() {
            let mut out = Vec::new();
            let start = preamble_len.min(lower.len());
            let mut cursor = start;
            while let Some(pos) = lower[cursor..].find(&needle) {
                let s = cursor + pos;
                let e = s + needle.len();
                out.push(s..e);
                cursor = e;
            }
            out
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let text_color = ui.visuals().text_color();
    let pixels_per_point = ui.ctx().pixels_per_point();

    for (page_idx, page) in doc.pages.iter().enumerate() {
        let (shapes, size, text_layout) =
            painter::render_frame_to_shapes(&page.frame, text_color, pixels_per_point.to_bits());

        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

        let offset = rect.min.to_vec2();
        let chars = &text_layout.chars;
        let has_text = !chars.is_empty();
        let visible = ui.is_rect_visible(rect);

        // ── Search match reporting ─────────────────────────────────
        // Runs for ALL pages (even those scrolled off-screen) so the
        // global match counter reflects the whole document, not just
        // what's currently in view. Painting underlines is gated on
        // visibility; scroll-to-focused is unconditional so prev/next
        // can navigate to off-screen matches.
        if !match_ranges.is_empty() && has_text {
            let source_obj = state.world.main_source();
            let glyph_ranges: Vec<Option<(usize, usize)>> = chars
                .iter()
                .map(|ch| {
                    let (span, off) = ch.span;
                    source_obj.range(span).map(|nr| {
                        let s = nr.start + off as usize;
                        let e = (s + ch.text.len()).min(nr.end);
                        (s, e)
                    })
                })
                .collect();
            let mut match_chars: Vec<Vec<usize>> =
                (0..match_ranges.len()).map(|_| Vec::new()).collect();
            for (ci, gr) in glyph_ranges.iter().enumerate() {
                let Some((gs, ge)) = gr else { continue };
                let mi = match_ranges.partition_point(|m| m.end <= *gs);
                if let Some(m) = match_ranges.get(mi) {
                    if m.start < *ge {
                        match_chars[mi].push(ci);
                    }
                }
            }
            for (mi, overlapping) in match_chars.iter().enumerate() {
                let match_id = response.id.with(("typst_match", page_idx, mi));
                let info = search.report(match_id);
                if overlapping.is_empty() {
                    continue;
                }
                let mut union = egui::Rect::NOTHING;
                for &ci in overlapping {
                    let r = chars[ci].rect.translate(offset);
                    if visible {
                        crate::search::paint_match_underline(ui.painter(), r, info.is_focused);
                    }
                    union = union.union(r);
                }
                if info.should_scroll_to && union != egui::Rect::NOTHING {
                    ui.scroll_to_rect(union, Some(egui::Align::Center));
                }
            }
        }

        if !visible {
            continue;
        }

        // ── Selection state ────────────────────────────────────────
        let sel_id = response.id;
        let mut sel = ui
            .data_mut(|d| d.get_temp::<TypstSelection>(sel_id))
            .unwrap_or_default();

        // Build glyphs rect for drag clamping.
        let glyphs_rect = if has_text {
            let mut r = egui::Rect::NOTHING;
            for ch in chars {
                r = r.union(ch.rect);
            }
            r.expand(12.0)
        } else {
            egui::Rect::NOTHING
        };

        if has_text {
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }

            if response.double_clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let frame_pos = pos - offset;
                    if let Some(range) =
                        double_click_range(state.world.main_source(), chars, frame_pos)
                    {
                        sel.glyph_override = Some(range);
                    }
                }
            } else if response.drag_started() {
                sel.glyph_override = None;
                if let Some(pos) = response.interact_pointer_pos() {
                    let frame_pos = pos - offset;
                    sel.anchor = Some(frame_pos);
                    sel.cursor = Some(frame_pos);
                }
            } else if response.dragged() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let frame_pos = pos - offset;
                    let clamped = glyphs_rect.clamp(frame_pos);
                    sel.cursor = Some(clamped);
                }
                ui.ctx()
                    .input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            }

            if response.clicked() && !response.double_clicked() && !response.dragged() {
                sel = TypstSelection::default();
            }
        }

        // ── Compute selection (cached) ───────────────────────────
        let glyph_range = sel.range(chars);
        let source = state.world.main_source();

        let sel_cache_id = sel_id.with("sel_cache");
        let sel_key = (sel.anchor, sel.cursor, sel.glyph_override.clone());
        let cached: Option<(
            (Option<egui::Pos2>, Option<egui::Pos2>, Option<Range<usize>>),
            SelectionResult,
        )> = ui.ctx().data_mut(|d| d.get_temp(sel_cache_id));

        let sel_result = if let Some((cached_key, cached_result)) = cached {
            if cached_key == sel_key {
                cached_result
            } else {
                let result = compute_selection(source, chars, &glyph_range);
                ui.ctx()
                    .data_mut(|d| d.insert_temp(sel_cache_id, (sel_key, result.clone())));
                result
            }
        } else {
            let result = compute_selection(source, chars, &glyph_range);
            if glyph_range.is_some() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(sel_cache_id, (sel_key, result.clone())));
            }
            result
        };

        let selected = &sel_result.sel_set;

        // ── Paint selection highlights (behind text) ─────────────
        {
            let highlight_color = ui.visuals().selection.bg_fill;
            for (i, ch) in chars.iter().enumerate() {
                if *selected.get(i).unwrap_or(&false) {
                    ui.painter()
                        .rect_filled(ch.rect.translate(offset), 0.0, highlight_color);
                }
            }
        }

        // ── Paint shapes, with images at their place in paint order ──
        let mut images = text_layout.images.iter().peekable();
        for (index, mut shape) in shapes.into_iter().enumerate() {
            while let Some(image) = images.next_if(|image| image.shape_index <= index) {
                paint_image(ui, image, offset);
            }
            shape.translate(offset);
            ui.painter().add(shape);
        }
        for image in images {
            paint_image(ui, image, offset);
        }
        if let Some(hover_pos) = response.hover_pos() {
            let frame_pos = hover_pos - offset;
            let hovered_alt = text_layout.images.iter().rev().find_map(|image| {
                let bounds = egui::Rect::from_points(&image.corners);
                bounds
                    .contains(frame_pos)
                    .then(|| image.image.alt())
                    .flatten()
            });
            if let Some(alt) = hovered_alt {
                response.clone().on_hover_text_at_pointer(alt);
            }
        }

        // ── Link interaction ─────────────────────────────────────
        if !text_layout.links.is_empty() {
            if let Some(hover_pos) = response.hover_pos() {
                let frame_pos = hover_pos - offset;
                for link in &text_layout.links {
                    if link.rect.contains(frame_pos) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if response.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&link.url));
                        }
                        break;
                    }
                }
            }
        }

        // ── Copy to clipboard ────────────────────────────────────
        if selected.iter().any(|&s| s) {
            response.request_focus();

            let wants_copy = ui.input(|i| {
                i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                    || (i.modifiers.command && i.key_pressed(egui::Key::C))
            });

            if wants_copy {
                let text = if let Some(ref r) = sel_result.copy_range {
                    source.text()[r.clone()].to_string()
                } else {
                    // Fallback: rendered text.
                    selected
                        .iter()
                        .enumerate()
                        .filter(|(_, &s)| s)
                        .filter_map(|(i, _)| chars.get(i).map(|c| c.text.as_str()))
                        .collect()
                };
                ui.ctx().copy_text(text);
            }
        }

        ui.data_mut(|d| d.insert_temp(sel_id, sel));
    }
}
