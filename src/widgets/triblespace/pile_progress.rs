//! Latest-value loading telemetry and a reusable, data-only progress instrument.
//! No resource, query, or notebook ownership lives in the widget.
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Opening,
    Replay,
    Snapshot,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub phase: Phase,
    pub replayed: Option<u64>,
    pub observed: Option<u64>,
    pub observed_at: Instant,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            phase: Phase::Opening,
            replayed: None,
            observed: None,
            observed_at: Instant::now(),
        }
    }
}
impl Progress {
    pub(super) fn bytes(&mut self, replayed: u64, observed: u64) {
        self.replayed = Some(replayed);
        self.observed = Some(observed);
        self.observed_at = Instant::now();
    }
    pub fn pending_bytes(&self) -> Option<u64> {
        Some(self.observed?.saturating_sub(self.replayed?))
    }
    pub fn replay_fraction(&self) -> Option<f32> {
        let (done, total) = (self.replayed?, self.observed?);
        // A replacement/shrink is not a completed old observation.
        if done > total {
            return None;
        }
        Some(if total == 0 {
            1.0
        } else {
            done as f32 / total as f32
        })
    }
    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Opening | Phase::Replay | Phase::Snapshot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batch {
    Yield,
    CaughtUp,
    Cancelled,
}

/// Cooperative scheduling between complete records. A single step may itself
/// wait on a file lock or process a huge proof; this is not a time guarantee.
/// `target` is the sampled high-water mark for this pass, not an immutable total.
pub fn replay_batch<E>(
    offset: &mut u64,
    target: u64,
    cancelled: impl Fn() -> bool,
    mut next: impl FnMut() -> Result<Option<usize>, E>,
) -> Result<Batch, E> {
    let started = Instant::now();
    for _ in 0..512 {
        if cancelled() {
            return Ok(Batch::Cancelled);
        }
        if *offset >= target {
            return Ok(Batch::CaughtUp);
        }
        match next()? {
            Some(end) => *offset = end as u64,
            None => return Ok(Batch::CaughtUp),
        }
        if started.elapsed() >= Duration::from_millis(8) {
            break;
        }
    }
    Ok(Batch::Yield)
}

/// Data-only face of a pile resource. No file is opened or read here.
///
/// The tick measures replayed bytes, not snapshot readiness. The returned
/// response is clicked only by the small refresh/retry affordance; its caller
/// decides whether to request a refresh. Errors stay visible below the rail.
pub struct PileProgress<'a> {
    path: &'a Path,
    progress: Progress,
    error: Option<&'a str>,
    refreshable: bool,
}
impl<'a> PileProgress<'a> {
    pub fn new(path: &'a Path, progress: Progress) -> Self {
        Self {
            path,
            progress,
            error: None,
            refreshable: true,
        }
    }

    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    /// Omit the action when the caller has no refresh/retry operation yet.
    pub fn refreshable(mut self, refreshable: bool) -> Self {
        self.refreshable = refreshable;
        self
    }
}

impl egui::Widget for PileProgress<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        use egui::{pos2, vec2, Align2, FontId, Sense, Stroke, StrokeKind};
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width().max(1.0), 48.0), Sense::hover());
        let painter = ui.painter_at(rect.intersect(ui.clip_rect()));
        let text = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let path_area = egui::Rect::from_min_max(
            rect.min,
            pos2(
                rect.right() - if self.refreshable { 22.0 } else { 0.0 },
                rect.top() + 17.0,
            ),
        );
        let path_font = FontId::monospace(10.0);
        let path = self.path.to_string_lossy();
        let label = middle_elide(&path, path_area.width(), |label| {
            painter
                .layout_no_wrap(label.to_owned(), path_font.clone(), weak)
                .size()
                .x
        });
        painter.text(
            path_area.left_center(),
            Align2::LEFT_CENTER,
            label,
            path_font,
            weak,
        );
        // Debug retains escaped non-UTF8 path bytes instead of silently replacing
        // them. Ordinary UTF-8 paths are shown verbatim, including all parents.
        let full_path = self
            .path
            .to_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{:?}", self.path));
        ui.interact(path_area, response.id.with("path"), Sense::hover())
            .on_hover_text(full_path);

        let retry = self.progress.phase == Phase::Failed;
        let refresh = self.refreshable.then(|| {
            ui.place(
                egui::Rect::from_min_size(pos2(rect.right() - 18.0, rect.top()), vec2(18.0, 17.0)),
                egui::Button::new("↻").frame(false).small(),
            )
            .on_hover_text(if retry {
                "Retry this source"
            } else {
                "Refresh this source"
            })
        });

        let rail = egui::Rect::from_min_max(pos2(rect.left(), rect.top() + 20.0), rect.max);
        let stroke = if retry {
            ui.visuals().error_fg_color
        } else {
            weak
        };
        painter.rect_stroke(rail, 2.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
        let inner = rail.shrink(2.0);
        let fraction = self.progress.replay_fraction();
        let watermark = fraction.map(|fraction| inner.left() + inner.width() * fraction);
        let unread = egui::Rect::from_min_max(
            pos2(watermark.unwrap_or(inner.left()), inner.top()),
            inner.max,
        );
        if unread.width() > 0.0 {
            let hatch = painter.with_clip_rect(unread.intersect(painter.clip_rect()));
            let mut x = unread.left() - unread.height();
            while x < unread.right() {
                hatch.line_segment(
                    [
                        pos2(x, unread.bottom()),
                        pos2(x + unread.height(), unread.top()),
                    ],
                    Stroke::new(0.75, weak.gamma_multiply(0.18)),
                );
                x += 8.0;
            }
        }
        let amount = rail_label(self.progress);
        let mut font = FontId::monospace(11.0);
        while font.size > 8.0
            && painter
                .layout_no_wrap(amount.clone(), font.clone(), text)
                .size()
                .x
                > inner.width() - 8.0
        {
            font.size -= 0.5;
        }
        let galley = painter.layout_no_wrap(amount, font, text);
        let label_rect = egui::Rect::from_center_size(inner.center(), galley.size()).expand(2.0);
        if let Some(x) = watermark {
            let tick = Stroke::new(2.0, if retry { stroke } else { text });
            if x >= label_rect.left() && x <= label_rect.right() {
                // The watermark stays at its measured byte position without
                // striking through the amount when it crosses the label.
                for (top, bottom) in [
                    (inner.top(), label_rect.top()),
                    (label_rect.bottom(), inner.bottom()),
                ] {
                    if top < bottom {
                        painter.line_segment([pos2(x, top), pos2(x, bottom)], tick);
                    }
                }
            } else {
                painter.line_segment([pos2(x, inner.top()), pos2(x, inner.bottom())], tick);
            }
        }
        painter
            .with_clip_rect(inner.intersect(painter.clip_rect()))
            .galley(inner.center() - galley.size() * 0.5, galley, text);
        let detail = format!(
            "{}\nReplayed: {}\nObserved: {}",
            phase_label(self.progress.phase),
            exact_bytes(self.progress.replayed),
            exact_bytes(self.progress.observed)
        );
        ui.interact(rail, response.id.with("bytes"), Sense::hover())
            .on_hover_text(detail);
        let response = match refresh {
            Some(refresh) => response.union(refresh),
            None => response,
        };
        if let Some(error) = self.error {
            let error = ui.add(
                egui::Label::new(
                    egui::RichText::new(error)
                        .monospace()
                        .size(11.0)
                        .color(ui.visuals().error_fg_color),
                )
                .wrap(),
            );
            return response.union(error);
        }
        response
    }
}

fn middle_elide(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    if measure("…") > width {
        return String::new();
    }
    let chars: Vec<_> = text.chars().collect();
    // Prefer keeping the immediate parent with the basename: two different
    // project.pile paths should not both collapse to the same trailing name.
    let tail = text
        .rsplit('/')
        .take(2)
        .map(|part| part.chars().count())
        .sum::<usize>()
        + 1;
    let (mut low, mut high) = (0, chars.len());
    let candidate = |keep: usize| {
        let left = if keep < 2 {
            0
        } else {
            keep.saturating_sub(tail).min(keep / 3).max(1)
        };
        let right = keep - left;
        chars[..left]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(chars[chars.len() - right..].iter())
            .collect::<String>()
    };
    while low < high {
        let middle = (low + high + 1) / 2;
        if measure(&candidate(middle)) <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    candidate(low)
}

fn exact_bytes(value: Option<u64>) -> String {
    value
        .map(|value| format!("{value} bytes"))
        .unwrap_or_else(|| "unknown".into())
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Opening => "opening",
        Phase::Replay => "replay",
        Phase::Snapshot => "snapshot",
        Phase::Ready => "ready",
        Phase::Failed => "failed",
    }
}

fn rail_label(progress: Progress) -> String {
    let maximum = progress
        .replayed
        .into_iter()
        .chain(progress.observed)
        .max()
        .unwrap_or(0);
    let mut unit = 1_u64;
    let mut suffix = "B";
    for next in ["kB", "MB", "GB", "TB", "PB", "EB"] {
        if maximum / unit < 1000 {
            break;
        }
        unit *= 1000;
        suffix = next;
    }
    let value = |value: Option<u64>| match value {
        Some(value) if unit == 1 => value.to_string(),
        Some(value) => format!("{:.1}", value as f64 / unit as f64),
        None => "?".into(),
    };
    format!(
        "{}/{} {suffix} · {}",
        value(progress.replayed),
        value(progress.observed),
        phase_label(progress.phase)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_completion_is_not_snapshot_readiness() {
        let mut progress = Progress {
            phase: Phase::Replay,
            ..Default::default()
        };
        for end in 1..=10_000 {
            progress.bytes(end, 10_000);
        }
        assert_eq!(progress.replayed, Some(10_000));
        assert_eq!(progress.replay_fraction(), Some(1.0));
        assert_eq!(progress.phase, Phase::Replay);
        progress.bytes(10_000, 12_000);
        assert_eq!(progress.pending_bytes(), Some(2_000));
        progress.phase = Phase::Snapshot;
        assert!(progress.active());
        progress.phase = Phase::Ready;
        assert!(!progress.active());
    }
    #[test]
    fn replay_yields_cancels_and_does_not_chase_an_appending_total() {
        let mut offset = 0;
        let mut next = 0;
        assert_eq!(
            replay_batch::<()>(
                &mut offset,
                2_000,
                || false,
                || {
                    next += 1;
                    Ok(Some(next))
                }
            )
            .unwrap(),
            Batch::Yield
        );
        assert!(offset > 0 && offset <= 512);
        let before = offset;
        assert_eq!(
            replay_batch::<()>(&mut offset, 2_000, || true, || panic!("cancelled step")).unwrap(),
            Batch::Cancelled
        );
        assert_eq!(offset, before);
        offset = 1_999;
        assert_eq!(
            replay_batch::<()>(&mut offset, 2_000, || false, || Ok(Some(2_001))).unwrap(),
            Batch::CaughtUp
        );
        assert_eq!(offset, 2_001, "complete record may cross the sampled bound");
    }
    #[test]
    fn replay_end_and_errors_are_not_fabricated_progress() {
        let mut offset = 10;
        assert_eq!(
            replay_batch::<()>(&mut offset, 20, || false, || Ok(None)).unwrap(),
            Batch::CaughtUp
        );
        assert_eq!(offset, 10);
        assert_eq!(
            replay_batch(&mut offset, 20, || false, || Err("read failed")),
            Err("read failed")
        );
        assert_eq!(offset, 10);
        let progress = Progress {
            replayed: Some(20),
            observed: Some(10),
            ..Default::default()
        };
        assert_eq!(progress.replay_fraction(), None);
    }
    #[test]
    fn instrument_is_bounded_at_narrow_and_wide_widths() {
        for (width, dark) in [180.0, 240.0, 360.0, 640.0, 1100.0]
            .into_iter()
            .flat_map(|width| [(width, false), (width, true)])
        {
            let context = egui::Context::default();
            context.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 300.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let response = ui.add(PileProgress::new(
                        Path::new(
                            "/public/observations/another-team/long-source-directory/project.pile",
                        ),
                        Progress {
                            replayed: Some(84_325_103_000),
                            observed: Some(233_305_898_752),
                            phase: Phase::Snapshot,
                            ..Default::default()
                        },
                    ));
                    assert_eq!(response.rect.height(), 48.0);
                    assert!(ui.min_rect().width() <= width);
                    assert!(ui.min_rect().height() <= 52.0);
                },
            );
            assert!(!output.shapes.is_empty());
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(
                        clipped.clip_rect.expand(1.0).contains_rect(bounds),
                        "text clipped at {width}: {} / {bounds:?} / {:?}",
                        text.galley.job.text,
                        clipped.clip_rect
                    );
                }
            }
        }
    }

    #[test]
    fn labels_preserve_unknowns_and_do_not_promote_byte_completion() {
        assert_eq!(rail_label(Progress::default()), "?/? B · opening");
        let mut progress = Progress {
            replayed: Some(1_000),
            observed: Some(1_000),
            phase: Phase::Replay,
            ..Default::default()
        };
        assert_eq!(rail_label(progress), "1.0/1.0 kB · replay");
        progress.phase = Phase::Snapshot;
        assert!(rail_label(progress).ends_with("snapshot"));
        progress.phase = Phase::Ready;
        assert!(rail_label(progress).ends_with("ready"));
        progress.observed = Some(2_000);
        assert_eq!(progress.pending_bytes(), Some(1_000));
        assert_eq!(progress.replay_fraction(), Some(0.5));
        assert_eq!(rail_label(progress), "1.0/2.0 kB · ready");
        progress.observed = None;
        assert_eq!(progress.replay_fraction(), None);
        assert_eq!(rail_label(progress), "1.0/? kB · ready");
        progress.observed = Some(0);
        assert_eq!(progress.replay_fraction(), None);
        progress.replayed = Some(0);
        assert_eq!(rail_label(progress), "0/0 B · ready");
        assert_eq!(progress.replay_fraction(), Some(1.0));
    }

    #[test]
    fn path_elision_keeps_context_and_is_unicode_safe() {
        let measure = |text: &str| text.chars().count() as f32;
        let path = "/public/Δelta/other-source/project.pile";
        assert_eq!(middle_elide(path, 100.0, measure), path);
        let short = middle_elide(path, 29.0, measure);
        assert!(short.contains('…'));
        assert!(short.starts_with('/'));
        assert!(short.ends_with("source/project.pile"));
        assert!(measure(&short) <= 29.0);
        assert_eq!(middle_elide(path, 0.0, measure), "");
        assert_ne!(
            middle_elide(
                "/public/observations/delta-team/project.pile",
                26.0,
                measure
            ),
            middle_elide(
                "/public/observations/coast-team/project.pile",
                26.0,
                measure
            )
        );
    }

    #[test]
    fn errors_remain_visible_and_action_can_be_absent() {
        let context = egui::Context::default();
        let error = "Permission denied while opening /public/source/project.pile";
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(180.0, 250.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.add(
                    PileProgress::new(
                        Path::new("/public/source/project.pile"),
                        Progress {
                            phase: Phase::Failed,
                            ..Default::default()
                        },
                    )
                    .error(Some(error))
                    .refreshable(false),
                );
                assert!(ui.min_rect().width() <= 180.0);
            },
        );
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains(&error));
        assert!(!text.contains(&"↻"));
        assert!(text.iter().any(|text| text.ends_with("failed")));
    }

    #[test]
    fn refresh_preserves_following_layout_and_errors_stay_below_the_rail() {
        for refreshable in [false, true] {
            for width in [180.0, 320.0] {
                let context = egui::Context::default();
                let error = "Invalid record; the last snapshot remains available.";
                let mut face_bottom = 0.0;
                let mut face_rect = egui::Rect::NOTHING;
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 400.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let first = ui.add(
                            PileProgress::new(
                                Path::new("/public/source.pile"),
                                Progress::default(),
                            )
                            .refreshable(refreshable),
                        );
                        assert!(ui.next_widget_position().y >= first.rect.bottom());
                        let start = ui.next_widget_position();
                        face_bottom = start.y + 48.0;
                        let failed = ui.add(
                            PileProgress::new(
                                Path::new("/public/source.pile"),
                                Progress {
                                    phase: Phase::Failed,
                                    ..Default::default()
                                },
                            )
                            .error(Some(error))
                            .refreshable(refreshable),
                        );
                        face_rect = failed.rect;
                        assert!(failed.rect.bottom() > face_bottom);
                        assert!(ui.next_widget_position().y >= failed.rect.bottom());
                        assert!(ui.min_rect().contains_rect(failed.rect));
                        assert!(ui.min_rect().width() <= width);
                    },
                );
                let error_shape = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == error => Some(text),
                        _ => None,
                    })
                    .expect("visible error text");
                let bounds = egui::Rect::from_min_size(error_shape.pos, error_shape.galley.size());
                assert!(
                    bounds.top() >= face_bottom,
                    "error overlaps rail: {bounds:?}"
                );
                assert!(face_rect.expand(1.0).contains_rect(bounds));
            }
        }
    }

    #[test]
    fn only_the_refresh_affordance_requests_an_action() {
        for (point, refreshable, expected) in [
            (egui::pos2(271.0, 8.5), true, true),
            (egui::pos2(140.0, 34.0), true, false),
            (egui::pos2(271.0, 8.5), false, false),
        ] {
            let context = egui::Context::default();
            let mut clicked = false;
            for pressed in [None, Some(true), Some(false)] {
                let mut events = vec![egui::Event::PointerMoved(point)];
                if let Some(pressed) = pressed {
                    events.push(egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    });
                }
                let _ = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(280.0, 160.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        clicked |= ui
                            .add(
                                PileProgress::new(
                                    Path::new("/public/project.pile"),
                                    Progress::default(),
                                )
                                .refreshable(refreshable),
                            )
                            .clicked();
                    },
                );
            }
            assert_eq!(clicked, expected);
        }
    }
}
