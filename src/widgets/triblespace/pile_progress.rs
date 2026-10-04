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
/// The tick measures replayed bytes, not snapshot readiness. This is a
/// hover-only instrument; the resource publishes automatically. Errors stay
/// visible below the rail, and exact path/byte information stays on hover.
pub struct PileProgress<'a> {
    path: &'a Path,
    progress: Progress,
    error: Option<&'a str>,
}
impl<'a> PileProgress<'a> {
    pub fn new(path: &'a Path, progress: Progress) -> Self {
        Self {
            path,
            progress,
            error: None,
        }
    }

    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }
}

impl egui::Widget for PileProgress<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        use egui::{pos2, vec2, FontId, Sense, Stroke, StrokeKind};
        let (rail, response) =
            ui.allocate_exact_size(vec2(ui.available_width().max(1.0), 28.0), Sense::hover());
        let painter = ui.painter_at(rail.intersect(ui.clip_rect()));
        let text = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let failed = self.progress.phase == Phase::Failed;
        let stroke = if failed {
            ui.visuals().error_fg_color
        } else {
            weak
        };
        painter.rect_stroke(rail, 2.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
        let inner = rail.shrink(2.0);
        let right = inner.right() - 4.0;
        let active = self.progress.phase != Phase::Ready;
        let amount =
            painter.layout_no_wrap(rail_label(self.progress), FontId::monospace(10.0), text);
        let amount_rect = egui::Rect::from_min_size(
            pos2(
                right - amount.size().x,
                rail.center().y - amount.size().y * 0.5 - if active { 5.0 } else { 0.0 },
            ),
            amount.size(),
        );
        let phase = active.then(|| {
            let galley = painter.layout_no_wrap(
                phase_label(self.progress.phase).to_owned(),
                FontId::monospace(8.0),
                if failed { stroke } else { weak },
            );
            let rect = egui::Rect::from_min_size(
                pos2(
                    right - galley.size().x,
                    rail.center().y + 6.0 - galley.size().y * 0.5,
                ),
                galley.size(),
            );
            (rect, galley)
        });
        let path_font = FontId::monospace(10.0);
        let path_left = inner.left() + 4.0;
        let path_right = phase.as_ref().map_or(amount_rect.left(), |(rect, _)| {
            rect.left().min(amount_rect.left())
        }) - 8.0;
        let label = middle_elide(
            &self.path.to_string_lossy(),
            (path_right - path_left).max(0.0),
            |label| {
                painter
                    .layout_no_wrap(label.to_owned(), path_font.clone(), weak)
                    .size()
                    .x
            },
        );
        let path = painter.layout_no_wrap(label, path_font, weak);
        let path_rect = egui::Rect::from_min_size(
            pos2(path_left, rail.center().y - path.size().y * 0.5),
            path.size(),
        );
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
        if let Some(x) = watermark {
            let tick = Stroke::new(2.0, if failed { stroke } else { text });
            let mut labels = vec![path_rect, amount_rect];
            labels.extend(phase.as_ref().map(|(rect, _)| *rect));
            for (top, bottom) in tick_spans(inner, x, &labels) {
                painter.line_segment([pos2(x, top), pos2(x, bottom)], tick);
            }
        }
        let content = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
        content.galley(path_rect.min, path, weak);
        content.galley(amount_rect.min, amount, text);
        if let Some((rect, galley)) = phase {
            content.galley(rect.min, galley, if failed { stroke } else { weak });
        }
        // Preserve the exact configured path, without filesystem resolution.
        // Debug escapes non-UTF8 path bytes instead of silently replacing them.
        let full_path = self
            .path
            .to_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{:?}", self.path));
        let detail = format!(
            "{full_path}\n{}\nReplayed: {}\nObserved: {}",
            phase_label(self.progress.phase),
            exact_bytes(self.progress.replayed),
            exact_bytes(self.progress.observed)
        );
        ui.interact(rail, response.id.with("bytes"), Sense::hover())
            .on_hover_text(detail);
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

fn tick_spans(inner: egui::Rect, x: f32, labels: &[egui::Rect]) -> Vec<(f32, f32)> {
    let mut blocked: Vec<_> = labels
        .iter()
        .map(|rect| rect.expand(1.0))
        .filter(|rect| x >= rect.left() && x <= rect.right())
        .collect();
    blocked.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let mut spans = Vec::new();
    let mut top = inner.top();
    for rect in blocked {
        let bottom = rect.top().min(inner.bottom());
        if top < bottom {
            spans.push((top, bottom));
        }
        top = top.max(rect.bottom());
    }
    if top < inner.bottom() {
        spans.push((top, inner.bottom()));
    }
    spans
}

fn middle_elide(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    if measure("…") > width {
        return String::new();
    }
    if let Some((parents, basename)) = text.rsplit_once('/') {
        if let Some(parent) = parents
            .rsplit('/')
            .next()
            .filter(|parent| !parent.is_empty())
        {
            // Keep the parent's beginning when the whole parent no longer
            // fits, so same-basename sources can still be distinguished.
            if measure(&format!("{parent}/{basename}")) > width {
                let mut prefix = String::new();
                let mut shortened = None;
                for ch in parent.chars() {
                    prefix.push(ch);
                    let candidate = format!("{prefix}…/{basename}");
                    if measure(&candidate) > width {
                        break;
                    }
                    shortened = Some(candidate);
                }
                if let Some(shortened) = shortened {
                    return shortened;
                }
            }
        }
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
        "{}/{} {suffix}",
        value(progress.replayed),
        value(progress.observed)
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
                    assert_eq!(response.rect.height(), 28.0);
                    assert!(ui.min_rect().width() <= width);
                    assert!(ui.min_rect().height() <= 32.0);
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
        assert_eq!(rail_label(Progress::default()), "?/? B");
        let mut progress = Progress {
            replayed: Some(1_000),
            observed: Some(1_000),
            phase: Phase::Replay,
            ..Default::default()
        };
        assert_eq!(rail_label(progress), "1.0/1.0 kB");
        progress.phase = Phase::Snapshot;
        assert_eq!(rail_label(progress), "1.0/1.0 kB");
        progress.phase = Phase::Ready;
        assert_eq!(rail_label(progress), "1.0/1.0 kB");
        progress.observed = Some(2_000);
        assert_eq!(progress.pending_bytes(), Some(1_000));
        assert_eq!(progress.replay_fraction(), Some(0.5));
        assert_eq!(rail_label(progress), "1.0/2.0 kB");
        progress.observed = None;
        assert_eq!(progress.replay_fraction(), None);
        assert_eq!(rail_label(progress), "1.0/? kB");
        progress.observed = Some(0);
        assert_eq!(progress.replay_fraction(), None);
        progress.replayed = Some(0);
        assert_eq!(rail_label(progress), "0/0 B");
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
        assert_eq!(
            middle_elide("/public/delta-team/project.pile", 19.0, measure),
            "delta…/project.pile"
        );
        assert_eq!(
            middle_elide("/public/coast-team/project.pile", 19.0, measure),
            "coast…/project.pile"
        );
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
    fn errors_remain_visible_without_an_action() {
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
                    .error(Some(error)),
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
    fn following_layout_and_errors_stay_below_the_rail() {
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
                    let first = ui.add(PileProgress::new(
                        Path::new("/public/source.pile"),
                        Progress::default(),
                    ));
                    assert!(ui.next_widget_position().y >= first.rect.bottom());
                    let start = ui.next_widget_position();
                    face_bottom = start.y + 28.0;
                    let failed = ui.add(
                        PileProgress::new(
                            Path::new("/public/source.pile"),
                            Progress {
                                phase: Phase::Failed,
                                ..Default::default()
                            },
                        )
                        .error(Some(error)),
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

    #[test]
    fn the_entire_rail_is_noninteractive() {
        for point in [
            egui::pos2(16.0, 14.0),
            egui::pos2(140.0, 14.0),
            egui::pos2(271.0, 14.0),
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
                            .add(PileProgress::new(
                                Path::new("/public/project.pile"),
                                Progress::default(),
                            ))
                            .clicked();
                    },
                );
            }
            assert!(!clicked);
        }
    }

    #[test]
    fn path_and_bytes_are_inside_one_rail_without_ready_or_refresh_decoration() {
        for phase in [
            Phase::Opening,
            Phase::Replay,
            Phase::Snapshot,
            Phase::Ready,
            Phase::Failed,
        ] {
            for width in [180.0, 320.0, 640.0] {
                let context = egui::Context::default();
                let mut rail = egui::Rect::NOTHING;
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 200.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        rail = ui
                            .add(PileProgress::new(
                                Path::new("/public/delta-team/project.pile"),
                                Progress {
                                    phase,
                                    replayed: Some(100_000_000_000),
                                    observed: Some(200_000_000_000),
                                    ..Default::default()
                                },
                            ))
                            .rect;
                    },
                );
                let labels: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some((
                            text.galley.job.text.as_str(),
                            egui::Rect::from_min_size(text.pos, text.galley.size()),
                        )),
                        _ => None,
                    })
                    .collect();
                assert!(labels.iter().all(|(text, bounds)| !text.contains("ready")
                    && !text.contains('↻')
                    && rail.contains_rect(*bounds)));
                let (_, path) = labels
                    .iter()
                    .find(|(text, _)| text.contains('/'))
                    .expect("visible path");
                let (_, bytes) = labels
                    .iter()
                    .find(|(text, _)| *text == "100.0/200.0 GB")
                    .expect("visible byte counts");
                assert!(path.right() < bytes.left());
                assert_eq!(
                    labels.iter().any(|(text, _)| *text == phase_label(phase)),
                    phase != Phase::Ready
                );
            }
        }
    }

    #[test]
    fn tick_keeps_its_position_without_striking_either_label() {
        use egui::{pos2, Rect};
        let inner = Rect::from_min_max(pos2(2.0, 2.0), pos2(178.0, 26.0));
        let path = Rect::from_min_max(pos2(6.0, 9.0), pos2(80.0, 19.0));
        let bytes = Rect::from_min_max(pos2(88.0, 4.0), pos2(174.0, 14.0));
        let phase = Rect::from_min_max(pos2(136.0, 16.0), pos2(174.0, 24.0));
        for x in [2.0, 50.0, 90.0, 150.0, 178.0] {
            let labels = [path, bytes, phase];
            let spans = tick_spans(inner, x, &labels);
            assert!(!spans.is_empty());
            for (top, bottom) in spans {
                assert!(top >= inner.top() && top < bottom && bottom <= inner.bottom());
                assert!(labels.iter().all(|label| x < label.left()
                    || x > label.right()
                    || bottom <= label.top()
                    || top >= label.bottom()));
            }
        }
    }
}
