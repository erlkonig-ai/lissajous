use eframe::egui;
use egui::epaint;
use typst::layout::{Frame, FrameItem, Transform};
use typst::model::Destination;
use typst::syntax::Span;
use typst::visualize::{CurveItem, FillRule, Geometry, Paint};

use super::outline;

/// A positioned source glyph for selection and highlighting.
/// Generated content (detached spans) is excluded — selection operates
/// on source content only, and copying the source regenerates them.
#[derive(Clone)]
pub struct PositionedChar {
    /// Bounding rect in frame-relative coordinates.
    pub rect: egui::Rect,
    /// The unicode text this glyph represents.
    pub text: String,
    /// Source span (always non-detached).
    pub span: (Span, u16),
}

/// A non-text element (shape/line/rect) with a source span.
#[derive(Clone)]
pub struct PositionedSpan {
    /// Bounding rect in frame-relative coordinates.
    pub rect: egui::Rect,
    /// Source span pointing back to the Typst construct.
    pub span: Span,
}

/// A positioned link region for hit-testing.
#[derive(Clone)]
pub struct PositionedLink {
    /// Bounding rect in frame-relative coordinates.
    pub rect: egui::Rect,
    /// The URL this link points to (only URL destinations are supported).
    pub url: String,
}

/// A placed image: the quad it covers after every enclosing transform, and
/// where it falls in paint order among the returned shapes.
#[derive(Clone)]
pub struct PositionedImage {
    /// The decoded Typst image (raster pixels, SVG or PDF).
    pub image: typst::visualize::Image,
    /// Corners in frame-relative coordinates: the image's top-left,
    /// top-right, bottom-right and bottom-left, in that order.
    pub corners: [egui::Pos2; 4],
    /// Number of shapes painted before this image.
    pub shape_index: usize,
}

/// Layout info collected during rendering for selection.
#[derive(Clone, Default)]
pub struct TextLayout {
    /// All positioned glyphs in layout order (source + generated).
    pub chars: Vec<PositionedChar>,
    /// Non-text elements with spans (table borders, lines, rects, etc.).
    pub spans: Vec<PositionedSpan>,
    /// Link regions for click/hover handling.
    pub links: Vec<PositionedLink>,
    /// Images, painted by the caller because they need GPU textures.
    pub images: Vec<PositionedImage>,
}

/// Render a Typst frame into egui shapes plus text layout info for selection.
///
/// Memoized via comemo — identical frames with the same render params
/// return cached results without re-walking the frame tree.
///
/// `pixels_per_point_bits` is `f32::to_bits()` so it can be hashed.
#[comemo::memoize]
pub fn render_frame_to_shapes(
    frame: &Frame,
    text_color: egui::Color32,
    pixels_per_point_bits: u32,
) -> (Vec<egui::Shape>, egui::Vec2, TextLayout) {
    let pixels_per_point = f32::from_bits(pixels_per_point_bits);
    let feathering = 1.0 / pixels_per_point;
    let mut shapes = Vec::new();
    let mut text_layout = TextLayout::default();
    let state = RenderState::identity();
    render_frame_inner(
        &mut shapes,
        &mut text_layout,
        frame,
        state,
        text_color,
        feathering,
    );

    let size = egui::vec2(frame.width().to_pt() as f32, frame.height().to_pt() as f32);
    (shapes, size, text_layout)
}

#[derive(Clone, Copy)]
struct RenderState {
    // Affine transform: x' = a*x + b*y + tx, y' = c*x + d*y + ty
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl RenderState {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Pre-translate by (dx, dy) in points.
    fn pre_translate(self, dx: f32, dy: f32) -> Self {
        Self {
            tx: self.tx + self.a * dx + self.b * dy,
            ty: self.ty + self.c * dx + self.d * dy,
            ..self
        }
    }

    /// Pre-concatenate with a Typst Transform.
    fn pre_concat(self, t: &Transform) -> Self {
        let sx = t.sx.get() as f32;
        let ky = t.ky.get() as f32;
        let kx = t.kx.get() as f32;
        let sy = t.sy.get() as f32;
        let ttx = t.tx.to_pt() as f32;
        let tty = t.ty.to_pt() as f32;

        Self {
            a: self.a * sx + self.b * ky,
            b: self.a * kx + self.b * sy,
            c: self.c * sx + self.d * ky,
            d: self.c * kx + self.d * sy,
            tx: self.tx + self.a * ttx + self.b * tty,
            ty: self.ty + self.c * ttx + self.d * tty,
        }
    }

    /// Transform a point from local coordinates to screen coordinates.
    fn transform_point(&self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(
            self.a * x + self.b * y + self.tx,
            self.c * x + self.d * y + self.ty,
        )
    }
}

fn render_frame_inner(
    shapes: &mut Vec<egui::Shape>,
    text_layout: &mut TextLayout,
    frame: &Frame,
    state: RenderState,
    text_color: egui::Color32,
    feathering: f32,
) {
    for (pos, item) in frame.items() {
        let local = state.pre_translate(pos.x.to_pt() as f32, pos.y.to_pt() as f32);

        match item {
            FrameItem::Group(group) => {
                let child = local.pre_concat(&group.transform);
                render_frame_inner(
                    shapes,
                    text_layout,
                    &group.frame,
                    child,
                    text_color,
                    feathering,
                );
            }
            FrameItem::Text(text_item) => {
                render_text(
                    shapes,
                    text_layout,
                    text_item,
                    local,
                    text_color,
                    feathering,
                );
            }
            FrameItem::Shape(shape, span) => {
                let shape_rect = shape_bounds(shape, local);
                if !span.is_detached() {
                    text_layout.spans.push(PositionedSpan {
                        rect: shape_rect,
                        span: *span,
                    });
                }
                render_shape(shapes, shape, local, text_color);
            }
            FrameItem::Link(dest, size) => {
                if let Destination::Url(url) = dest {
                    let p0 = local.transform_point(0.0, 0.0);
                    let p1 = local.transform_point(size.x.to_pt() as f32, size.y.to_pt() as f32);
                    text_layout.links.push(PositionedLink {
                        rect: egui::Rect::from_two_pos(p0, p1),
                        url: url.to_string(),
                    });
                }
            }
            FrameItem::Image(image, size, _) => {
                let (w, h) = (size.x.to_pt() as f32, size.y.to_pt() as f32);
                text_layout.images.push(PositionedImage {
                    image: image.clone(),
                    corners: [
                        local.transform_point(0.0, 0.0),
                        local.transform_point(w, 0.0),
                        local.transform_point(w, h),
                        local.transform_point(0.0, h),
                    ],
                    shape_index: shapes.len(),
                });
            }
            FrameItem::Tag(..) => {}
        }
    }
}

fn render_text(
    shapes: &mut Vec<egui::Shape>,
    text_layout: &mut TextLayout,
    text: &typst::text::TextItem,
    state: RenderState,
    default_color: egui::Color32,
    feathering: f32,
) {
    let font = &text.font;
    let size = text.size.to_pt() as f32;
    let upem = font.ttf().units_per_em() as f32;
    let scale = size / upem;

    // Text fill color.
    let color = paint_to_color32(&text.fill, default_color);

    // Text stroke (e.g. `#set text(stroke: 0.5pt + red)`).
    let stroke = text.stroke.as_ref().map(|s| {
        egui::Stroke::new(
            s.thickness.to_pt() as f32,
            paint_to_color32(&s.paint, default_color),
        )
    });

    // Font metrics for selection rects (in local Typst Y-down coords).
    let ascender = font.ttf().ascender() as f32 * scale;
    let descender = font.ttf().descender() as f32 * scale;

    let mut cursor_x: f32 = 0.0;
    let mut cursor_y: f32 = 0.0;

    for glyph in &text.glyphs {
        let x_offset = glyph.x_offset.get() as f32 * size;
        let y_offset = glyph.y_offset.get() as f32 * size;

        let gx = cursor_x + x_offset;
        let gy = cursor_y - y_offset;

        let origin = state.transform_point(gx, gy);

        let glyph_mesh = outline::glyph_mesh(font.clone(), glyph.id);

        // Fill pass.
        let mesh = outline::render_glyph_mesh(&glyph_mesh, origin, scale, color, feathering);
        if !mesh.is_empty() {
            shapes.push(egui::Shape::mesh(mesh));
        }

        // Stroke pass.
        if let Some(stroke) = stroke {
            for contour in &glyph_mesh.contours {
                if contour.len() < 2 {
                    continue;
                }
                let points: Vec<egui::Pos2> = contour
                    .iter()
                    .map(|&[x, y]| egui::pos2(origin.x + x * scale, origin.y - y * scale))
                    .collect();
                shapes.push(egui::Shape::Path(epaint::PathShape {
                    points,
                    closed: true,
                    fill: egui::Color32::TRANSPARENT,
                    stroke: stroke.into(),
                }));
            }
        }

        // Collect text layout info for selection (source glyphs only).
        let adv_x = glyph.x_advance.get() as f32 * size;
        if !glyph.span.0.is_detached() {
            let top_left = state.transform_point(cursor_x, cursor_y - ascender);
            let bottom_right = state.transform_point(cursor_x + adv_x, cursor_y - descender);
            let glyph_text = &text.text[glyph.range()];
            text_layout.chars.push(PositionedChar {
                rect: egui::Rect::from_two_pos(top_left, bottom_right),
                text: glyph_text.to_string(),
                span: glyph.span,
            });
        }

        cursor_x += adv_x;
        cursor_y += glyph.y_advance.get() as f32 * size;
    }
}

/// Compute the bounding rect of a shape in frame-relative coordinates.
fn shape_bounds(shape: &typst::visualize::Shape, state: RenderState) -> egui::Rect {
    match &shape.geometry {
        Geometry::Line(end) => {
            let p0 = state.transform_point(0.0, 0.0);
            let p1 = state.transform_point(end.x.to_pt() as f32, end.y.to_pt() as f32);
            egui::Rect::from_two_pos(p0, p1)
        }
        Geometry::Rect(size) => {
            let p0 = state.transform_point(0.0, 0.0);
            let p1 = state.transform_point(size.x.to_pt() as f32, size.y.to_pt() as f32);
            egui::Rect::from_two_pos(p0, p1)
        }
        Geometry::Curve(curve) => {
            let mut rect = egui::Rect::NOTHING;
            let mut pen;
            for item in &curve.0 {
                match item {
                    CurveItem::Move(p) => {
                        pen = egui::pos2(p.x.to_pt() as f32, p.y.to_pt() as f32);
                        rect = rect.union(egui::Rect::from_center_size(
                            state.transform_point(pen.x, pen.y),
                            egui::Vec2::ZERO,
                        ));
                    }
                    CurveItem::Line(p) => {
                        pen = egui::pos2(p.x.to_pt() as f32, p.y.to_pt() as f32);
                        rect = rect.union(egui::Rect::from_center_size(
                            state.transform_point(pen.x, pen.y),
                            egui::Vec2::ZERO,
                        ));
                    }
                    CurveItem::Cubic(_, _, end) => {
                        pen = egui::pos2(end.x.to_pt() as f32, end.y.to_pt() as f32);
                        rect = rect.union(egui::Rect::from_center_size(
                            state.transform_point(pen.x, pen.y),
                            egui::Vec2::ZERO,
                        ));
                    }
                    CurveItem::Close => {}
                }
            }
            rect
        }
    }
}

fn render_shape(
    shapes: &mut Vec<egui::Shape>,
    shape: &typst::visualize::Shape,
    state: RenderState,
    default_color: egui::Color32,
) {
    let fill = shape
        .fill
        .as_ref()
        .map(|p| paint_to_color32(p, default_color));

    let stroke = shape.stroke.as_ref().map(|s| {
        egui::Stroke::new(
            s.thickness.to_pt() as f32,
            paint_to_color32(&s.paint, default_color),
        )
    });

    let dash: Option<DashInfo> = shape
        .stroke
        .as_ref()
        .and_then(|s| s.dash.as_ref())
        .map(|d| DashInfo {
            array: d.array.iter().map(|a| a.to_pt() as f32).collect(),
            phase: d.phase.to_pt() as f32,
        });

    let even_odd = matches!(shape.fill_rule, FillRule::EvenOdd);

    match &shape.geometry {
        Geometry::Line(end) => {
            let p0 = state.transform_point(0.0, 0.0);
            let p1 = state.transform_point(end.x.to_pt() as f32, end.y.to_pt() as f32);
            if let Some(stroke) = stroke {
                if let Some(ref dash) = dash {
                    for seg in dash_polyline(&[p0, p1], dash) {
                        if seg.len() >= 2 {
                            shapes.push(egui::Shape::Path(epaint::PathShape::line(seg, stroke)));
                        }
                    }
                } else {
                    shapes.push(egui::Shape::line_segment([p0, p1], stroke));
                }
            }
        }
        Geometry::Rect(size) => {
            let p0 = state.transform_point(0.0, 0.0);
            let p1 = state.transform_point(size.x.to_pt() as f32, size.y.to_pt() as f32);
            let rect = egui::Rect::from_two_pos(p0, p1);
            let fill_color = fill.unwrap_or(egui::Color32::TRANSPARENT);
            let stroke_val = stroke.unwrap_or(egui::Stroke::NONE);
            shapes.push(egui::Shape::rect_filled(rect, 0.0, fill_color));
            if stroke_val.width > 0.0 {
                shapes.push(egui::Shape::rect_stroke(
                    rect,
                    0.0,
                    stroke_val,
                    egui::StrokeKind::Outside,
                ));
            }
        }
        Geometry::Curve(curve) => {
            render_curve(
                shapes,
                &curve.0,
                state,
                fill,
                stroke,
                dash.as_ref(),
                even_odd,
            );
        }
    }
}

fn render_curve(
    shapes: &mut Vec<egui::Shape>,
    items: &[CurveItem],
    state: RenderState,
    fill: Option<egui::Color32>,
    stroke: Option<egui::Stroke>,
    dash: Option<&DashInfo>,
    even_odd: bool,
) {
    // Walk CurveItems, building transformed polyline subpaths.
    let mut subpaths: Vec<Vec<egui::Pos2>> = Vec::new();
    let mut current: Vec<egui::Pos2> = Vec::new();
    let mut pen = egui::pos2(0.0, 0.0);
    let mut subpath_start = pen;

    for item in items {
        match item {
            CurveItem::Move(p) => {
                if !current.is_empty() {
                    subpaths.push(std::mem::take(&mut current));
                }
                pen = egui::pos2(p.x.to_pt() as f32, p.y.to_pt() as f32);
                subpath_start = pen;
                current.push(state.transform_point(pen.x, pen.y));
            }
            CurveItem::Line(p) => {
                pen = egui::pos2(p.x.to_pt() as f32, p.y.to_pt() as f32);
                current.push(state.transform_point(pen.x, pen.y));
            }
            CurveItem::Cubic(c1, c2, end) => {
                let p0 = pen;
                let p1 = egui::pos2(c1.x.to_pt() as f32, c1.y.to_pt() as f32);
                let p2 = egui::pos2(c2.x.to_pt() as f32, c2.y.to_pt() as f32);
                let p3 = egui::pos2(end.x.to_pt() as f32, end.y.to_pt() as f32);
                let mut local_pts = Vec::new();
                flatten_cubic(p0, p1, p2, p3, CURVE_TOLERANCE, &mut local_pts);
                for lp in local_pts {
                    current.push(state.transform_point(lp.x, lp.y));
                }
                pen = p3;
            }
            CurveItem::Close => {
                if !current.is_empty() {
                    let start_screen = state.transform_point(subpath_start.x, subpath_start.y);
                    if let Some(&last) = current.last() {
                        if (last - start_screen).length_sq() > 1e-4 {
                            current.push(start_screen);
                        }
                    }
                    subpaths.push(std::mem::take(&mut current));
                }
                pen = subpath_start;
            }
        }
    }
    if !current.is_empty() {
        subpaths.push(current);
    }

    let fill_color = fill.unwrap_or(egui::Color32::TRANSPARENT);
    let stroke_val = stroke.unwrap_or(egui::Stroke::NONE);

    // ── Fill pass ──────────────────────────────────────────────────────
    if fill_color != egui::Color32::TRANSPARENT {
        let closed: Vec<&Vec<egui::Pos2>> = subpaths.iter().filter(|p| is_closed(p)).collect();

        if closed.len() == 1 && !even_odd {
            // Single closed subpath, non-zero fill — PathShape handles concave fills.
            shapes.push(egui::Shape::Path(epaint::PathShape::convex_polygon(
                closed[0].clone(),
                fill_color,
                egui::Stroke::NONE,
            )));
        } else if closed.len() == 1 && even_odd {
            // Single self-intersecting path with even-odd fill — decompose
            // into simple faces and keep only those with odd winding.
            let mesh = outline::even_odd_single_path(closed[0], fill_color);
            if !mesh.is_empty() {
                shapes.push(egui::Shape::mesh(mesh));
            }
        } else if closed.len() > 1 {
            // Multiple subpaths — triangulate with fill rule.
            let owned: Vec<Vec<egui::Pos2>> = closed.into_iter().cloned().collect();
            let mesh = outline::triangulate_subpaths(&owned, even_odd, fill_color);
            if !mesh.is_empty() {
                shapes.push(egui::Shape::mesh(mesh));
            }
        }
    }

    // ── Stroke pass (with optional dashing) ────────────────────────────
    if stroke_val.width > 0.0 {
        for path in &subpaths {
            if path.len() < 2 {
                continue;
            }
            if let Some(dash) = dash {
                for seg in dash_polyline(path, dash) {
                    if seg.len() >= 2 {
                        shapes.push(egui::Shape::Path(epaint::PathShape::line(seg, stroke_val)));
                    }
                }
            } else {
                shapes.push(egui::Shape::Path(epaint::PathShape {
                    points: path.clone(),
                    closed: is_closed(path),
                    fill: egui::Color32::TRANSPARENT,
                    stroke: stroke_val.into(),
                }));
            }
        }
    }
}

fn is_closed(path: &[egui::Pos2]) -> bool {
    path.len() >= 3
        && (path.first().unwrap().to_vec2() - path.last().unwrap().to_vec2()).length_sq() < 1e-2
}

// ── Dash pattern support ──────────────────────────────────────────────

struct DashInfo {
    array: Vec<f32>,
    phase: f32,
}

/// Split a polyline into visible dash segments according to a dash pattern.
fn dash_polyline(points: &[egui::Pos2], pattern: &DashInfo) -> Vec<Vec<egui::Pos2>> {
    if points.len() < 2 || pattern.array.is_empty() {
        return vec![points.to_vec()];
    }

    let total_len: f32 = pattern.array.iter().sum();
    if total_len <= 0.0 {
        return vec![points.to_vec()];
    }

    let mut result = Vec::new();
    let mut current: Vec<egui::Pos2> = Vec::new();

    // Initialize pattern position from phase.
    let pat_pos = pattern.phase.rem_euclid(total_len);

    // Find starting element and remaining distance in it.
    let mut elem_idx = 0usize;
    let mut remaining = 0.0f32;
    {
        let mut acc = 0.0f32;
        for (i, &len) in pattern.array.iter().enumerate() {
            if acc + len > pat_pos {
                elem_idx = i;
                remaining = acc + len - pat_pos;
                break;
            }
            acc += len;
        }
    }

    let is_dash = |idx: usize| idx % 2 == 0; // even = dash, odd = gap
    let mut drawing = is_dash(elem_idx);

    if drawing {
        current.push(points[0]);
    }

    for window in points.windows(2) {
        let (p0, p1) = (window[0], window[1]);
        let dx = p1.x - p0.x;
        let dy = p1.y - p0.y;
        let seg_len = (dx * dx + dy * dy).sqrt();
        if seg_len < 1e-8 {
            continue;
        }

        let mut t_consumed = 0.0f32;

        loop {
            let t_remaining = seg_len - t_consumed;
            if t_remaining < 1e-8 {
                break;
            }

            if remaining <= t_remaining {
                // Pattern boundary falls within this segment.
                t_consumed += remaining;
                let frac = t_consumed / seg_len;
                let boundary = egui::pos2(p0.x + dx * frac, p0.y + dy * frac);

                if drawing {
                    // End of dash.
                    current.push(boundary);
                    if current.len() >= 2 {
                        result.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                }

                // Advance to next pattern element.
                elem_idx = (elem_idx + 1) % pattern.array.len();
                remaining = pattern.array[elem_idx];
                drawing = is_dash(elem_idx);

                if drawing {
                    // Start of new dash.
                    current.push(boundary);
                }
            } else {
                // Segment ends before next boundary.
                remaining -= t_remaining;
                if drawing {
                    current.push(p1);
                }
                break;
            }
        }
    }

    // Flush remaining dash.
    if drawing && current.len() >= 2 {
        result.push(current);
    }

    result
}

/// Tolerance for flattening curve geometry (in Typst points).
const CURVE_TOLERANCE: f32 = 0.25;

fn flatten_cubic(
    p0: egui::Pos2,
    p1: egui::Pos2,
    p2: egui::Pos2,
    p3: egui::Pos2,
    tolerance: f32,
    out: &mut Vec<egui::Pos2>,
) {
    let d1 = point_line_dist(p1, p0, p3);
    let d2 = point_line_dist(p2, p0, p3);
    if d1 <= tolerance && d2 <= tolerance {
        out.push(p3);
    } else {
        let mid = |a: egui::Pos2, b: egui::Pos2| egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
        let q0 = mid(p0, p1);
        let q1 = mid(p1, p2);
        let q2 = mid(p2, p3);
        let r0 = mid(q0, q1);
        let r1 = mid(q1, q2);
        let s0 = mid(r0, r1);
        flatten_cubic(p0, q0, r0, s0, tolerance, out);
        flatten_cubic(s0, r1, q2, p3, tolerance, out);
    }
}

fn point_line_dist(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-10 {
        ((p.x - a.x).powi(2) + (p.y - a.y).powi(2)).sqrt()
    } else {
        ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / len_sq.sqrt()
    }
}

fn paint_to_color32(paint: &Paint, default: egui::Color32) -> egui::Color32 {
    match paint {
        Paint::Solid(color) => {
            let [r, g, b, a] = color.to_vec4_u8();
            egui::Color32::from_rgba_unmultiplied(r, g, b, a)
        }
        // Gradient and tiling paints: fall back to default.
        _ => default,
    }
}
