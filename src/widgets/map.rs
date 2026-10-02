//! Borrowed, bounded geospatial display geometry. No tiles, network, or store.
//!
//! Coordinates are WGS84 longitude/latitude in degrees, displayed in Web
//! Mercator. The caller owns the camera and selection, so refreshing a query
//! does not reset either. IDs are opaque: the widget never derives or looks up
//! an entity. Raster ingestion/reprojection belongs upstream, not here.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Pos2, Rect, Response, Sense, Stroke, TextStyle, Ui,
};

/// Web Mercator cannot display the geographic poles.
pub const MERCATOR_MAX_LATITUDE: f64 = 85.051_128_779_806_6;
const EARTH_CIRCUMFERENCE_METRES: f64 = 40_075_016.685_578_49;

/// `[longitude, latitude]` in WGS84 degrees, never a projected metre pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoPosition(pub [f64; 2]);

impl GeoPosition {
    /// Reject nonfinite/out-of-domain coordinates rather than guessing a CRS,
    /// silently wrapping bad longitude, or moving a polar point southward.
    pub fn project(self) -> Option<[f64; 2]> {
        let [lon, lat] = self.0;
        if !lon.is_finite()
            || !lat.is_finite()
            || !(-180.0..=180.0).contains(&lon)
            || !(-MERCATOR_MAX_LATITUDE..=MERCATOR_MAX_LATITUDE).contains(&lat)
        {
            return None;
        }
        let y = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0;
        Some([((lon + 180.0) / 360.0).rem_euclid(1.0), y.clamp(0.0, 1.0)])
    }

    fn unproject([x, y]: [f64; 2]) -> Self {
        Self([
            x.rem_euclid(1.0) * 360.0 - 180.0,
            (std::f64::consts::PI * (1.0 - 2.0 * y))
                .sinh()
                .atan()
                .to_degrees(),
        ])
    }
}

/// Borrowed display primitives, not an owned copy of the source database.
#[derive(Clone, Copy, Debug)]
pub enum MapGeometry<'a> {
    Point(GeoPosition),
    /// Consecutive vertices follow the shorter longitude arc. Invalid vertices
    /// break the path; no segment bridges across a skipped observation.
    Path(&'a [GeoPosition]),
}

/// One bounded query result with an opaque caller identity.
#[derive(Clone, Copy, Debug)]
pub struct MapFeature<'a, Id> {
    pub id: Id,
    pub geometry: MapGeometry<'a>,
    pub label: &'a str,
    pub color: Color32,
}

/// A legend entry names the caller's semantics; colour alone never names them.
#[derive(Clone, Copy, Debug)]
pub struct MapLegendEntry<'a> {
    pub label: &'a str,
    pub color: Color32,
}

/// Caller-owned pan/zoom state. Initially fits once; later data changes never
/// refit automatically. `fit` or the widget's fit button is explicit.
#[derive(Clone, Debug, PartialEq)]
pub struct MapCamera {
    center: [f64; 2],
    pixels_per_world: f64,
    initialized: bool,
}

impl Default for MapCamera {
    fn default() -> Self {
        Self {
            center: [0.5, 0.5],
            pixels_per_world: 512.0,
            initialized: false,
        }
    }
}

impl MapCamera {
    /// The current geographic centre (including after a pan).
    pub fn center(&self) -> GeoPosition {
        GeoPosition::unproject(self.center)
    }

    /// Explicitly fit valid geometry, including the short arc at the dateline.
    /// Returns false when nothing can be displayed; the previous view survives.
    pub fn fit<Id>(&mut self, features: &[MapFeature<'_, Id>], size: [f64; 2]) -> bool {
        if size.iter().any(|s| !s.is_finite() || *s <= 0.0) {
            return false;
        }
        let mut points = Vec::new();
        for feature in features {
            match feature.geometry {
                MapGeometry::Point(p) => points.extend(p.project()),
                MapGeometry::Path(path) => points.extend(path.iter().filter_map(|p| p.project())),
            }
        }
        let Some((min, max)) = fit_bounds(&mut points) else {
            return false;
        };
        self.center = [
            (0.5 * min[0] + 0.5 * max[0]).rem_euclid(1.0),
            0.5 * min[1] + 0.5 * max[1],
        ];
        let span = [
            (max[0] - min[0]).max(0.000_01),
            (max[1] - min[1]).max(0.000_01),
        ];
        self.pixels_per_world = ((size[0] - 64.0).max(1.0) / span[0])
            .min((size[1] - 64.0).max(1.0) / span[1])
            .clamp(1.0, 1.0e9);
        self.initialized = true;
        true
    }

    fn prepare<Id>(&mut self, features: &[MapFeature<'_, Id>], size: [f64; 2]) {
        if !self.initialized {
            self.fit(features, size);
        }
        self.pixels_per_world = self.pixels_per_world.clamp((size[0] / 2.0).max(1.0), 1.0e9);
    }

    fn pan(&mut self, delta: [f64; 2]) {
        self.center[0] = (self.center[0] - delta[0] / self.pixels_per_world).rem_euclid(1.0);
        self.center[1] = (self.center[1] - delta[1] / self.pixels_per_world).clamp(0.0, 1.0);
    }

    fn zoom(&mut self, factor: f64, anchor: [f64; 2], min_scale: f64) {
        let before = self.pixels_per_world;
        self.pixels_per_world = (before * factor).clamp(min_scale.max(1.0), 1.0e9);
        for (i, offset) in anchor.into_iter().enumerate() {
            self.center[i] += offset * (1.0 / before - 1.0 / self.pixels_per_world);
        }
        self.center[0] = self.center[0].rem_euclid(1.0);
        self.center[1] = self.center[1].clamp(0.0, 1.0);
    }

    fn screen(&self, rect: Rect, p: [f64; 2]) -> Pos2 {
        pos2(
            rect.center().x + ((p[0] - self.center[0]) * self.pixels_per_world) as f32,
            rect.center().y + ((p[1] - self.center[1]) * self.pixels_per_world) as f32,
        )
    }
}

/// Picked identities and honest display omissions for this frame.
pub struct MapResponse<Id> {
    pub response: Response,
    pub hovered: Option<Id>,
    /// Some only on a click that hit a feature, not on every selected frame.
    pub clicked: Option<Id>,
    pub selected: Option<Id>,
    /// Invalid coordinate vertices (including polar Web Mercator exclusions).
    pub skipped_vertices: usize,
    /// Features with no displayable point or segment.
    pub skipped_features: usize,
}

/// A local, tile-free map. Empty areas mean no supplied geometry, not land or
/// water. The graticule and WGS84 label make that boundary visible.
pub struct MapView<'a, Id> {
    features: &'a [MapFeature<'a, Id>],
    camera: &'a mut MapCamera,
    selected: &'a mut Option<Id>,
    legend: &'a [MapLegendEntry<'a>],
    height: f32,
    show_controls: bool,
    show_projection_label: bool,
}

impl<'a, Id: Copy + PartialEq> MapView<'a, Id> {
    pub fn new(
        features: &'a [MapFeature<'a, Id>],
        camera: &'a mut MapCamera,
        selected: &'a mut Option<Id>,
    ) -> Self {
        Self {
            features,
            camera,
            selected,
            legend: &[],
            height: 360.0,
            show_controls: true,
            show_projection_label: true,
        }
    }

    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height > 0.0 {
            self.height = height;
        }
        self
    }

    pub fn legend(mut self, legend: &'a [MapLegendEntry<'a>]) -> Self {
        self.legend = legend;
        self
    }

    /// Hide the separate fit/header row for a direct-manipulation map. Pan,
    /// wheel zoom and double-click-to-fit remain available. Centering clears
    /// selection; a blank single click clears immediately.
    pub fn show_controls(mut self, show: bool) -> Self {
        self.show_controls = show;
        self
    }

    /// Show the projection annotation in the header or canvas (default true).
    /// This changes only presentation; geometry, scale and gestures are kept.
    pub fn show_projection_label(mut self, show: bool) -> Self {
        self.show_projection_label = show;
        self
    }

    pub fn show(self, ui: &mut Ui) -> MapResponse<Id> {
        let width = ui.available_width().max(1.0);
        if self.show_controls {
            ui.horizontal(|ui| {
                if ui.small_button("fit geometry").clicked() {
                    self.camera
                        .fit(self.features, [width as f64, self.height as f64]);
                    *self.selected = None;
                }
                if self.show_projection_label {
                    ui.small("WGS84 · Web Mercator · local geometry");
                }
            });
        }
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, self.height), Sense::click_and_drag());
        self.camera
            .prepare(self.features, [rect.width() as f64, rect.height() as f64]);
        let double_clicked = response.double_clicked();
        if double_clicked {
            self.camera
                .fit(self.features, [rect.width() as f64, rect.height() as f64]);
            *self.selected = None;
        }
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta());
            self.camera.pan([delta.x as f64, delta.y as f64]);
        }
        if response.hovered() {
            let (scroll, pointer) = ui.input_mut(|i| {
                let y = i.smooth_scroll_delta.y;
                i.smooth_scroll_delta.y = 0.0;
                (y, i.pointer.hover_pos())
            });
            if let Some(pointer) = pointer {
                let anchor = pointer - rect.center();
                self.camera.zoom(
                    (scroll as f64 * 0.003).exp(),
                    [anchor.x as f64, anchor.y as f64],
                    rect.width() as f64 / 2.0,
                );
            }
        }
        let painter = ui.painter().with_clip_rect(rect);
        let ink = ui.visuals().text_color();
        let accent = crate::themes::button_light_on();
        let track = crate::themes::blend(ui.visuals().window_fill, ink, 0.12);
        painter.rect_filled(rect, 0.0, ui.visuals().panel_fill);
        draw_graticule(&painter, self.camera, rect, track);
        let pointer = response.hover_pos();
        let mut hit = None;
        let mut best_distance = 8.0_f32;
        let mut skipped_vertices = 0;
        let mut skipped_features = 0;
        for (index, feature) in self.features.iter().enumerate() {
            let selected = *self.selected == Some(feature.id);
            let color = if selected { accent } else { feature.color };
            let width: f32 = if selected { 3.0 } else { 1.5 };
            let mut valid = false;
            match feature.geometry {
                MapGeometry::Point(p) => {
                    if let Some(p) = p.project() {
                        valid = true;
                        for p in copies(p, self.camera.center[0]) {
                            let at = self.camera.screen(rect, p);
                            if rect.expand(8.0).contains(at) {
                                painter.circle_filled(at, if selected { 5.0 } else { 3.0 }, color);
                                painter.circle_stroke(at, 7.0, Stroke::new(0.8_f32, color));
                                if let Some(pointer) = pointer {
                                    pick_hit(
                                        &mut hit,
                                        &mut best_distance,
                                        index,
                                        pointer.distance(at),
                                    );
                                }
                            }
                        }
                    } else {
                        skipped_vertices += 1;
                    }
                }
                MapGeometry::Path(path) => {
                    skipped_vertices += path.iter().filter(|p| p.project().is_none()).count();
                    for pair in path.windows(2) {
                        let (Some(a), Some(b)) = (pair[0].project(), pair[1].project()) else {
                            continue;
                        };
                        valid = true;
                        let [a, b] = short_segment(a, b);
                        let middle = 0.5 * a[0] + 0.5 * b[0];
                        let base = (self.camera.center[0] - middle).round();
                        for shift in [base - 1.0, base, base + 1.0] {
                            let a = self.camera.screen(rect, [a[0] + shift, a[1]]);
                            let b = self.camera.screen(rect, [b[0] + shift, b[1]]);
                            if Rect::from_two_pos(a, b).expand(8.0).intersects(rect) {
                                painter.line_segment([a, b], Stroke::new(width, color));
                                if let Some(pointer) = pointer {
                                    pick_hit(
                                        &mut hit,
                                        &mut best_distance,
                                        index,
                                        segment_distance(pointer, a, b),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if !valid {
                skipped_features += 1;
            }
        }
        let hovered = hit.map(|index| self.features[index].id);
        let clicked = (response.clicked() && !double_clicked)
            .then_some(hovered)
            .flatten();
        if response.clicked() && !double_clicked {
            *self.selected = hovered;
        }
        if response.clicked() {
            ui.ctx().request_repaint();
        }
        if let Some(index) = hit {
            response.clone().on_hover_text(self.features[index].label);
        }
        let font = TextStyle::Small.resolve(ui.style());
        if !self.show_controls && self.show_projection_label {
            painter.text(
                rect.left_top() + vec2(8.0, 8.0),
                Align2::LEFT_TOP,
                "WGS84 · Web Mercator · local geometry",
                font.clone(),
                ink,
            );
        }
        draw_scale(&painter, self.camera, rect, ink, font);
        ui.horizontal_wrapped(|ui| {
            for entry in self.legend {
                ui.colored_label(entry.color, "●");
                ui.small(entry.label);
            }
            if skipped_vertices > 0 || skipped_features > 0 {
                ui.small(format!("skipped {skipped_vertices} invalid vertices · {skipped_features} undrawable features"));
            }
            if self.features.is_empty() { ui.small("No located observations in this query."); }
        });
        MapResponse {
            response,
            hovered,
            clicked,
            selected: *self.selected,
            skipped_vertices,
            skipped_features,
        }
    }
}

fn copies(p: [f64; 2], center_x: f64) -> [[f64; 2]; 3] {
    let base = (center_x - p[0]).round();
    [base - 1.0, base, base + 1.0].map(|shift| [p[0] + shift, p[1]])
}

fn short_segment(a: [f64; 2], mut b: [f64; 2]) -> [[f64; 2]; 2] {
    b[0] += (a[0] - b[0]).round();
    [a, b]
}

fn fit_bounds(points: &mut [[f64; 2]]) -> Option<([f64; 2], [f64; 2])> {
    if points.is_empty() {
        return None;
    }
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut largest_gap = -1.0;
    let mut start = points[0][0];
    for i in 0..points.len() {
        let next =
            points[(i + 1) % points.len()][0] + if i + 1 == points.len() { 1.0 } else { 0.0 };
        let gap = next - points[i][0];
        if gap > largest_gap {
            largest_gap = gap;
            start = next.rem_euclid(1.0);
        }
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in points {
        let x = if p[0] < start { p[0] + 1.0 } else { p[0] };
        min[0] = min[0].min(x);
        max[0] = max[0].max(x);
        min[1] = min[1].min(p[1]);
        max[1] = max[1].max(p[1]);
    }
    Some((min, max))
}

fn pick_hit(hit: &mut Option<usize>, distance: &mut f32, index: usize, candidate: f32) {
    if candidate < *distance {
        *hit = Some(index);
        *distance = candidate;
    }
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = if d.length_sq() > 0.0 {
        ((p - a).dot(d) / d.length_sq()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + t * d)
}

fn draw_graticule(painter: &egui::Painter, camera: &MapCamera, rect: Rect, color: Color32) {
    let degrees = 360.0 * rect.width() as f64 / camera.pixels_per_world;
    let step = [
        0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 90.0,
    ]
    .into_iter()
    .find(|step| degrees / step < 12.0)
    .unwrap_or(90.0);
    let center_lon = camera.center().0[0];
    let center_lat = camera.center().0[1];
    for offset in -12..=12 {
        let lon = ((center_lon / step).floor() + offset as f64) * step;
        let x = (lon + 180.0) / 360.0;
        let at = camera.screen(rect, [x + (camera.center[0] - x).round(), camera.center[1]]);
        if rect.contains(at) {
            painter.vline(at.x, rect.y_range(), Stroke::new(0.5_f32, color));
        }
        let lat = ((center_lat / step).floor() + offset as f64) * step;
        if let Some(p) = GeoPosition([0.0, lat]).project() {
            let at = camera.screen(rect, [camera.center[0], p[1]]);
            if rect.contains(at) {
                painter.hline(rect.x_range(), at.y, Stroke::new(0.5_f32, color));
            }
        }
    }
}

fn draw_scale(
    painter: &egui::Painter,
    camera: &MapCamera,
    rect: Rect,
    ink: Color32,
    font: egui::FontId,
) {
    // Local ground distance at the view centre, not an area-preserving claim.
    let latitude = camera.center().0[1].to_radians();
    let metres_per_pixel = EARTH_CIRCUMFERENCE_METRES * latitude.cos() / camera.pixels_per_world;
    let desired = metres_per_pixel * (rect.width() as f64 / 4.0).min(100.0);
    let magnitude = 10.0_f64.powf(desired.log10().floor());
    let metres = [1.0, 2.0, 5.0]
        .into_iter()
        .rev()
        .find(|n| n * magnitude <= desired)
        .unwrap_or(1.0)
        * magnitude;
    let pixels = (metres / metres_per_pixel) as f32;
    let start = rect.left_bottom() + vec2(12.0, -24.0);
    painter.line_segment(
        [start, start + vec2(pixels, 0.0)],
        Stroke::new(1.5_f32, ink),
    );
    for x in [0.0, pixels] {
        painter.line_segment(
            [start + vec2(x, -3.0), start + vec2(x, 3.0)],
            Stroke::new(1.0_f32, ink),
        );
    }
    let label = if metres >= 1000.0 {
        format!("≈ {:.3} km at centre", metres / 1000.0)
    } else {
        format!("≈ {metres:.3} m at centre")
    };
    painter.text(start + vec2(0.0, 6.0), Align2::LEFT_TOP, label, font, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_frame(
        context: &egui::Context,
        features: &[MapFeature<'_, u8>],
        camera: &mut MapCamera,
        selected: &mut Option<u8>,
        time: f64,
        events: Vec<egui::Event>,
        controls: bool,
    ) -> MapResponse<u8> {
        let mut result = None;
        let _ = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(600.0);
                result = Some(
                    MapView::new(features, camera, selected)
                        .show_controls(controls)
                        .height(320.0)
                        .show(ui),
                );
            },
        );
        result.unwrap()
    }

    fn pointer(at: Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn controls_are_opt_out_and_map_uses_full_body_width() {
        let context = egui::Context::default();
        let features = [feature(31, GeoPosition([4.0, 52.0]))];
        let mut camera = MapCamera::default();
        let mut selected = None;
        let controls = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.0,
            vec![],
            true,
        );
        let direct = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            1.0,
            vec![],
            false,
        );
        assert!(controls.response.rect.top() > direct.response.rect.top());
        assert_eq!(direct.response.rect.width(), 600.0);
        assert_eq!(direct.response.rect.height(), 320.0);
    }

    #[test]
    fn native_clicks_select_opaque_marker_and_clear_background_immediately() {
        let context = egui::Context::default();
        let features = [feature(77, GeoPosition([4.0, 52.0]))];
        let mut camera = MapCamera::default();
        let mut selected = None;
        let rect = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.0,
            vec![],
            false,
        )
        .response
        .rect;
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.1,
            pointer(rect.center(), true),
            false,
        );
        let click = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.12,
            pointer(rect.center(), false),
            false,
        );
        assert_eq!(click.clicked, Some(77));
        assert_eq!(selected, Some(77));
        let blank = rect.left_top() + vec2(25.0, 40.0);
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.6,
            pointer(blank, true),
            false,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.62,
            pointer(blank, false),
            false,
        );
        assert_eq!(selected, None, "blank click clears on its release frame");
    }

    #[test]
    fn native_double_click_fits_immediately_and_drag_never_deselects() {
        let context = egui::Context::default();
        let features = [feature(31, GeoPosition([4.0, 52.0]))];
        let mut camera = MapCamera::default();
        let mut selected = Some(31);
        let rect = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.0,
            vec![],
            false,
        )
        .response
        .rect;
        let fitted = camera.clone();
        let blank = rect.left_top() + vec2(25.0, 40.0);
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.1,
            pointer(blank, true),
            false,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.2,
            vec![egui::Event::PointerMoved(blank + vec2(80.0, 30.0))],
            false,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.22,
            pointer(blank + vec2(80.0, 30.0), false),
            false,
        );
        assert_eq!(selected, Some(31));
        assert_ne!(camera, fitted, "native drag pans");
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.6,
            pointer(blank, true),
            false,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.62,
            pointer(blank, false),
            false,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.7,
            pointer(blank, true),
            false,
        );
        let second = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.72,
            pointer(blank, false),
            false,
        );
        assert!(second.response.double_clicked());
        assert_eq!(camera, fitted, "second release fits immediately");
        assert_eq!(selected, None, "centering clears selection");
    }

    #[test]
    fn marker_double_click_fits_and_clears_without_reselecting() {
        let context = egui::Context::default();
        let features = [feature(31, GeoPosition([4.0, 52.0]))];
        let mut camera = MapCamera::default();
        let mut selected = None;
        let rect = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.0,
            vec![],
            false,
        )
        .response
        .rect;
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.1,
            pointer(rect.center(), true),
            false,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.12,
            pointer(rect.center(), false),
            false,
        );
        assert_eq!(selected, Some(31));
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.2,
            pointer(rect.center(), true),
            false,
        );
        let second = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.22,
            pointer(rect.center(), false),
            false,
        );
        assert!(second.response.double_clicked());
        assert_eq!(
            second.clicked, None,
            "double click is center/clear, not marker selection"
        );
        assert_eq!(selected, None);
    }

    #[test]
    fn native_scroll_zoom_preserves_selection_and_header_fit_clears_it() {
        let context = egui::Context::default();
        let features = [feature(31, GeoPosition([4.0, 52.0]))];
        let mut camera = MapCamera::default();
        let mut selected = Some(31);
        let rect = map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.0,
            vec![],
            true,
        )
        .response
        .rect;
        let fitted = camera.clone();
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.1,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 60.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        assert_eq!(selected, Some(31), "wheel zoom must not clear selection");
        assert_ne!(camera, fitted, "actual wheel input zooms the map");
        let button = pos2(30.0, rect.top() / 2.0);
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.6,
            pointer(button, true),
            true,
        );
        map_frame(
            &context,
            &features,
            &mut camera,
            &mut selected,
            0.62,
            pointer(button, false),
            true,
        );
        assert_eq!(selected, None, "explicit center has the same clear policy");
    }

    #[test]
    fn opaque_copy_identity_does_not_need_hash() {
        #[derive(Clone, Copy, PartialEq)]
        struct Opaque(u8);
        let features = [MapFeature {
            id: Opaque(31),
            geometry: MapGeometry::Point(GeoPosition([4.0, 52.0])),
            label: "opaque",
            color: Color32::WHITE,
        }];
        let mut camera = MapCamera::default();
        let mut selected = Some(Opaque(31));
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            MapView::new(&features, &mut camera, &mut selected)
                .show_controls(false)
                .show(ui);
        });
        assert!(selected == Some(Opaque(31)));
    }

    #[test]
    fn projection_annotation_can_be_hidden_without_hiding_scale() {
        fn text(shape: &egui::Shape, output: &mut String) {
            match shape {
                egui::Shape::Text(s) => output.push_str(&s.galley.job.text),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        text(shape, output);
                    }
                }
                _ => {}
            }
        }
        for controls in [false, true] {
            for show_projection in [false, true] {
                let features = [feature(1, GeoPosition([4.0, 52.0]))];
                let mut camera = MapCamera::default();
                let mut selected = Some(1);
                let context = egui::Context::default();
                let frame = context.run_ui(egui::RawInput::default(), |ui| {
                    MapView::new(&features, &mut camera, &mut selected)
                        .show_controls(controls)
                        .show_projection_label(show_projection)
                        .show(ui);
                });
                let mut labels = String::new();
                for shape in frame.shapes {
                    text(&shape.shape, &mut labels);
                }
                assert_eq!(labels.contains("Web Mercator"), show_projection);
                assert!(labels.contains("at centre"), "scale remains on the map");
                assert_eq!(selected, Some(1));
            }
        }
    }

    fn feature(id: u8, p: GeoPosition) -> MapFeature<'static, u8> {
        MapFeature {
            id,
            geometry: MapGeometry::Point(p),
            label: "observation",
            color: Color32::WHITE,
        }
    }

    #[test]
    fn projection_is_degrees_and_round_trips() {
        assert_eq!(GeoPosition([0.0, 0.0]).project(), Some([0.5, 0.5]));
        let p = GeoPosition([4.43, 52.23]);
        let result = GeoPosition::unproject(p.project().unwrap());
        assert!((p.0[0] - result.0[0]).abs() < 1.0e-10);
        assert!((p.0[1] - result.0[1]).abs() < 1.0e-10);
    }

    #[test]
    fn invalid_and_polar_coordinates_are_not_relocated() {
        for p in [
            [f64::NAN, 0.0],
            [0.0, f64::INFINITY],
            [181.0, 0.0],
            [0.0, 90.0],
            [0.0, -86.0],
        ] {
            assert!(GeoPosition(p).project().is_none());
        }
        assert!(GeoPosition([-180.0, MERCATOR_MAX_LATITUDE])
            .project()
            .is_some());
        assert_eq!(
            GeoPosition([-180.0, 0.0]).project(),
            GeoPosition([180.0, 0.0]).project()
        );
    }

    #[test]
    fn dateline_segment_is_short_and_fit_does_not_span_the_world() {
        let a = GeoPosition([179.0, 0.0]);
        let b = GeoPosition([-179.0, 1.0]);
        let [a_projected, b_projected] = short_segment(a.project().unwrap(), b.project().unwrap());
        assert!((b_projected[0] - a_projected[0]).abs() < 0.01);
        let features = [feature(1, a), feature(2, b)];
        let mut camera = MapCamera::default();
        assert!(camera.fit(&features, [600.0, 400.0]));
        assert!(camera.center().0[0].abs() > 179.0);
        assert!(camera.pixels_per_world > 10_000.0);
    }

    #[test]
    fn data_refresh_preserves_camera_and_empty_fit_preserves_view() {
        let mut camera = MapCamera::default();
        camera.prepare(&[feature(1, GeoPosition([4.0, 52.0]))], [600.0, 400.0]);
        camera.pan([100.0, 30.0]);
        let saved = camera.clone();
        camera.prepare(&[feature(99, GeoPosition([-120.0, 30.0]))], [600.0, 400.0]);
        assert_eq!(camera, saved);
        assert!(!camera.fit::<u8>(&[], [600.0, 400.0]));
        assert_eq!(camera, saved);
        assert!(!camera.fit(&[feature(1, GeoPosition([4.0, 52.0]))], [f64::NAN, 400.0]));
        assert_eq!(camera, saved);
    }

    #[test]
    fn zoom_keeps_pointer_world_position_fixed() {
        let mut camera = MapCamera::default();
        camera.initialized = true;
        let before = [
            camera.center[0] + 50.0 / camera.pixels_per_world,
            camera.center[1] + 20.0 / camera.pixels_per_world,
        ];
        camera.zoom(2.0, [50.0, 20.0], 1.0);
        let after = [
            camera.center[0] + 50.0 / camera.pixels_per_world,
            camera.center[1] + 20.0 / camera.pixels_per_world,
        ];
        assert_eq!(before, after);
    }

    #[test]
    fn nearest_hit_keeps_caller_identity_not_label_or_coordinate() {
        let features = [
            feature(31, GeoPosition([4.0, 52.0])),
            feature(77, GeoPosition([4.0, 52.0])),
        ];
        let mut hit = None;
        let mut distance = 8.0;
        pick_hit(&mut hit, &mut distance, 0, 5.0);
        pick_hit(&mut hit, &mut distance, 1, 2.0);
        assert_eq!(features[hit.unwrap()].id, 77);
        assert_eq!(
            segment_distance(pos2(3.0, 2.0), pos2(0.0, 0.0), pos2(6.0, 0.0)),
            2.0
        );
    }

    #[test]
    fn widget_reports_invalid_geometry_and_does_not_bridge_a_hole() {
        let path = [
            GeoPosition([4.0, 52.0]),
            GeoPosition([f64::NAN, 52.1]),
            GeoPosition([4.2, 52.2]),
        ];
        let features = [
            feature(1, GeoPosition([4.0, 52.0])),
            feature(2, GeoPosition([181.0, 52.0])),
            MapFeature {
                id: 3,
                geometry: MapGeometry::Path(&path),
                label: "path with a hole",
                color: Color32::WHITE,
            },
        ];
        let context = egui::Context::default();
        let mut camera = MapCamera::default();
        let mut selected = Some(1);
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let result = MapView::new(&features, &mut camera, &mut selected).show(ui);
                assert_eq!(result.skipped_vertices, 2);
                assert_eq!(result.skipped_features, 2);
                assert_eq!(result.selected, Some(1));
            });
        });
    }
}
