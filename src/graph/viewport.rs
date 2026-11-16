//! Pan, zoom, fit and culling for a world-space graph drawn into a rect.
//!
//! The interaction handling is lifted from the wiki viewer, including two
//! details that look like noise and are not:
//!
//! * The hover check is `rect.contains(pointer)` directly, not
//!   `response.hovered()`. Inside a notebook's `ScrollArea` the scroll area
//!   claims hover priority, `hovered()` returns false, and wheel events fall
//!   through to the page instead of the graph.
//! * Only the scroll delta actually *used* is consumed. Zooming on horizontal
//!   drift caught trackpad sideways motion on every scroll and zoomed the graph
//!   when the reader only wanted to scroll the page.
//!
//! `fit` is new, and it fixes something the lifted viewer had wrong: it seeded
//! a ring of radius `200 + 5n`, which at its live size is 17,110 world units
//! against a 7,680-unit visible half-width at the zoom floor, so the graph
//! opened entirely off-screen and the reader had to find it by dragging.

use eframe::egui::{vec2, Id, Pos2, Rect, Response, Ui, Vec2};

/// How much detail is worth drawing.
///
/// Chosen by how far apart neighbouring marks land ON SCREEN, which is the
/// question a level of detail actually answers: marks fold when they would
/// overlap, not when the zoom number is small. Those are different, and
/// conflating them is a bug I shipped and then saw — a three-peer mesh fits a
/// card at a zoom of about a tenth, because its ring is laid out in world units
/// that have no fixed relationship to points, and every peer folded to a dot on
/// a picture with all the room in the world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lod {
    /// Glyph, stroke, track and label.
    Full,
    /// Glyph and stroke; no track, no label.
    Marks,
    /// A dot per node.
    Dots,
    /// Below one mark per pixel: fold into screen cells.
    Aggregate,
}

/// Pan and zoom over a world-space graph.
#[derive(Clone, Copy, Debug)]
pub struct GraphViewport {
    pan: Vec2,
    zoom: f32,
    /// Set the first time the reader pans or zooms.
    ///
    /// Until then the view re-frames the graph every frame, so a widget in a
    /// resizing panel behaves the way an immediate-mode one always did and a
    /// settling layout stays on screen while it settles. After it, the reader's
    /// framing is theirs and nothing takes it back.
    touched: bool,
}

impl Default for GraphViewport {
    fn default() -> Self {
        GraphViewport {
            pan: Vec2::ZERO,
            zoom: 1.0,
            touched: false,
        }
    }
}

/// Zoom limits.
///
/// The ceiling is the viewer's. The floor is **not**: the lifted viewer clamped
/// at 0.05, and at that floor the visible half-width is 7,680 world units,
/// which cannot frame a graph of any size this module is now asked to draw —
/// the viewer's own live graph opened at a ring radius of 17,110 and was
/// therefore entirely off-screen. A floor that makes it impossible to see the
/// data is a bug, not a guard, so it moves down to where `fit` can do its job
/// on a chain that spans tens of thousands of units.
const MIN_ZOOM: f32 = 0.002;
const MAX_ZOOM: f32 = 10.0;

impl GraphViewport {
    /// Read this viewport's stored pan and zoom, or start at the identity.
    pub fn load(ui: &Ui, id: Id) -> Self {
        ui.ctx()
            .data(|d| d.get_temp::<GraphViewport>(id))
            .unwrap_or_default()
    }

    pub fn store(&self, ui: &Ui, id: Id) {
        ui.ctx().data_mut(|d| d.insert_temp(id, *self));
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Apply this frame's pointer input.
    pub fn interact(&mut self, ui: &Ui, response: &Response, rect: Rect) {
        let centre = rect.center();
        // Raw zoom input must respect covering areas, including non-dragging floats.
        let pointer_inside = ui.ctx().rect_contains_pointer(
            response.layer_id,
            rect.intersect(ui.clip_rect()),
        );
        if pointer_inside {
            // Pinch-to-zoom, or command/ctrl with the wheel. Plain scroll is
            // deliberately *not* consumed: it belongs to whatever the graph is
            // sitting inside.
            let (pinch, scroll, command) = ui.input(|i| {
                (
                    i.zoom_delta(),
                    i.smooth_scroll_delta.y,
                    i.modifiers.command || i.modifiers.ctrl,
                )
            });
            let factor = if pinch != 1.0 {
                pinch
            } else if command && scroll != 0.0 {
                (1.0 + scroll * 0.004).clamp(0.85, 1.15)
            } else {
                1.0
            };
            if factor != 1.0 {
                self.touched = true;
                let previous = self.zoom;
                self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                if let Some(at) = response.hover_pos() {
                    let offset = at - centre - self.pan;
                    self.pan -= offset * (self.zoom / previous - 1.0);
                }
                if command && scroll != 0.0 {
                    ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
                }
            }
        }

        // egui's drag sense is z-aware, so a floating card dragged across the
        // viewport does not steal the pan.
        let dragged = response.drag_delta();
        if dragged != Vec2::ZERO {
            self.touched = true;
            self.pan += dragged;
        }
    }

    /// Has the reader taken over the framing?
    pub fn touched(&self) -> bool {
        self.touched
    }

    /// Give the framing back to the widget, as a retarget should.
    pub fn release(&mut self) {
        self.touched = false;
    }

    /// Frame the graph, unless the reader has taken the framing over.
    pub fn fit_unless_touched(&mut self, rect: Rect, bounds: [f32; 4], margin: f32) {
        if !self.touched {
            self.fit(rect, bounds, margin);
        }
    }

    /// Put a world bounding box fully inside `rect`, keeping `margin` screen
    /// points clear on every side for whatever the caller draws outside the
    /// marks — labels, mostly, which are the reason a fit that only framed the
    /// node positions would clip the very names the reader needs.
    pub fn fit(&mut self, rect: Rect, bounds: [f32; 4], margin: f32) {
        let width = (bounds[2] - bounds[0]).max(1.0);
        let height = (bounds[3] - bounds[1]).max(1.0);
        if !width.is_finite() || !height.is_finite() {
            return;
        }
        let usable = vec2(
            (rect.width() - margin * 2.0).max(1.0),
            (rect.height() - margin * 2.0).max(1.0),
        );
        self.zoom = (usable.x / width)
            .min(usable.y / height)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        let centre = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
        self.pan = -vec2(centre[0] * self.zoom, centre[1] * self.zoom);
    }

    /// Put `world` at the centre of `rect` without changing the zoom.
    pub fn center_on(&mut self, _rect: Rect, world: [f32; 2]) {
        self.pan = -vec2(world[0] * self.zoom, world[1] * self.zoom);
    }

    pub fn to_screen(&self, rect: Rect, world: [f32; 2]) -> Pos2 {
        rect.center() + self.pan + vec2(world[0] * self.zoom, world[1] * self.zoom)
    }

    pub fn to_world(&self, rect: Rect, screen: Pos2) -> [f32; 2] {
        let offset = screen - rect.center() - self.pan;
        [offset.x / self.zoom, offset.y / self.zoom]
    }

    /// The world box currently on screen, grown by `margin` screen points.
    ///
    /// Culling against this is what stops per-frame cost depending on the node
    /// count: the drawn set is bounded by screen area, and a graph ten times
    /// larger costs the same to paint at the same zoom.
    pub fn visible_world(&self, rect: Rect, margin: f32) -> [f32; 4] {
        let grown = rect.expand(margin);
        let min = self.to_world(rect, grown.min);
        let max = self.to_world(rect, grown.max);
        [min[0], min[1], max[0], max[1]]
    }

    /// The detail level for a layout whose neighbouring nodes sit `pitch` world
    /// units apart.
    ///
    /// The thresholds are multiples of the mark, because what makes a mark
    /// unreadable is another mark on top of it. A track needs room for a track;
    /// a glyph needs room for a glyph; below a mark's width apart there is
    /// nothing to see but ink.
    pub fn lod(&self, pitch: f32) -> Lod {
        let spacing = pitch * self.zoom;
        if spacing >= super::marks::TRACK * 2.4 {
            Lod::Full
        } else if spacing >= super::marks::MARK * 2.4 {
            Lod::Marks
        } else if spacing >= super::marks::MARK {
            Lod::Dots
        } else {
            Lod::Aggregate
        }
    }

    /// The size to draw a mark at, given the current zoom.
    ///
    /// A mark is a SYMBOL, not a measured object: it stands for a thing rather
    /// than showing its extent, so it stays readable instead of tracking the
    /// zoom all the way down to a speck and all the way up to a plate. World
    /// distances still scale; only the vocabulary holds its size.
    pub fn mark_scale(&self) -> f32 {
        self.zoom.clamp(0.45, 1.5)
    }
}

/// Index of the nearest point to `at` within `radius`, if any.
///
/// Picking runs on the CPU over whatever the caller already culled, so its cost
/// is bounded by screen area like everything else in this module.
pub fn pick(points: &[Pos2], at: Pos2, radius: f32) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| (index, (*point - at).length()))
        .filter(|(_, distance)| *distance <= radius)
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(768.0, 400.0))
    }

    #[test]
    fn fit_puts_the_whole_bbox_on_screen() {
        // The size the lifted viewer actually opened at: a ring of radius
        // 200 + 5 * 3382 = 17,110. At the 0.05 zoom floor the visible
        // half-width is 7,680 world units, so it opened off-screen entirely.
        let mut view = GraphViewport::default();
        let radius = 17_110.0_f32;
        view.fit(rect(), [-radius, -radius, radius, radius], 32.0);
        for corner in [
            [-radius, -radius],
            [radius, -radius],
            [-radius, radius],
            [radius, radius],
        ] {
            let at = view.to_screen(rect(), corner);
            assert!(rect().contains(at), "corner {corner:?} landed at {at:?}");
        }
    }

    #[test]
    fn fit_does_not_magnify_a_single_point() {
        // A one-node graph has a zero-size box. Fitting it must not divide by
        // zero or slam the zoom to its ceiling.
        let mut view = GraphViewport::default();
        view.fit(rect(), [4.0, 4.0, 4.0, 4.0], 32.0);
        assert!(view.zoom().is_finite());
        assert!(view.zoom() <= MAX_ZOOM);
        assert!(rect().contains(view.to_screen(rect(), [4.0, 4.0])));
    }

    #[test]
    fn zoom_is_cursor_anchored() {
        // The world point under the cursor must not move when the zoom does,
        // which is the whole reason the pan is adjusted alongside it.
        let mut view = GraphViewport::default();
        let cursor = pos2(600.0, 90.0);
        let before = view.to_world(rect(), cursor);
        let previous = view.zoom;
        view.zoom = (view.zoom * 1.6).clamp(MIN_ZOOM, MAX_ZOOM);
        let offset = cursor - rect().center() - view.pan;
        view.pan -= offset * (view.zoom / previous - 1.0);
        let after = view.to_world(rect(), cursor);
        assert!((before[0] - after[0]).abs() < 1e-2);
        assert!((before[1] - after[1]).abs() < 1e-2);
    }

    #[test]
    fn hit_test_picks_the_nearest_within_hit() {
        let points = [pos2(0.0, 0.0), pos2(12.0, 0.0), pos2(200.0, 0.0)];
        assert_eq!(
            pick(&points, pos2(10.0, 0.0), super::super::marks::HIT),
            Some(1)
        );
        assert_eq!(
            pick(&points, pos2(600.0, 0.0), super::super::marks::HIT),
            None
        );
    }

    #[test]
    fn culling_bounds_the_drawn_set_by_screen_area() {
        // The visible box must not grow when the graph does. It is a function
        // of the rect and the zoom alone, which is the property that makes
        // per-frame cost independent of the node count.
        let view = GraphViewport::default();
        let box_a = view.visible_world(rect(), 50.0);
        let box_b = view.visible_world(rect(), 50.0);
        assert_eq!(box_a, box_b);
        let width = box_a[2] - box_a[0];
        assert!((width - (768.0 + 100.0)).abs() < 1e-3);
    }
}
