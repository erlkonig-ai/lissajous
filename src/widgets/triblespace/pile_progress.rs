//! Latest-value loading telemetry and a reusable, data-only progress instrument.
//! No pile handles, request queues, paths, or notebook state live in the widget.
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

/// Generic instrument face: labels are supplied by its caller, so the same
/// primitive can show bytes, work units, or another bounded measurement. It is
/// an ordinary egui widget suitable for a Lissajous `nb.view`/retained state.
pub struct Instrument<'a> {
    pub title: &'a str,
    pub amount: &'a str,
    pub fraction: Option<f32>,
    pub accent: egui::Color32,
    pub stages: [&'a str; 3],
    pub note: &'a str,
}
impl egui::Widget for Instrument<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        use egui::{pos2, vec2, Align2, FontId, Sense, Stroke};
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 126.0), Sense::hover());
        let painter = ui.painter_at(rect.intersect(ui.clip_rect()));
        let area = rect.shrink2(vec2(18.0, 14.0));
        let text = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        painter.text(
            area.left_top(),
            Align2::LEFT_TOP,
            self.title,
            FontId::monospace(11.0),
            weak,
        );
        painter.text(
            pos2(area.left(), area.top() + 22.0),
            Align2::LEFT_TOP,
            self.amount,
            FontId::monospace(18.0),
            text,
        );
        let rail = egui::Rect::from_min_size(
            pos2(area.left(), area.top() + 53.0),
            vec2(area.width(), 3.0),
        );
        painter.rect_filled(rail, 1.5, ui.visuals().faint_bg_color);
        if let Some(fraction) = self.fraction.filter(|value| value.is_finite()) {
            let filled = egui::Rect::from_min_size(
                rail.min,
                vec2(rail.width() * fraction.clamp(0.0, 1.0), rail.height()),
            );
            painter.rect_filled(filled, 1.5, self.accent);
        } else {
            painter.line_segment(
                [rail.left_center(), rail.right_center()],
                Stroke::new(1.0, weak),
            );
        }
        for (index, label) in self.stages.iter().enumerate() {
            let x = area.left() + index as f32 * area.width() / 3.0;
            let column = egui::Rect::from_min_size(
                pos2(x, area.top() + 66.0),
                vec2(area.width() / 3.0, 17.0),
            );
            painter.with_clip_rect(column.intersect(rect)).text(
                column.min,
                Align2::LEFT_TOP,
                label,
                FontId::monospace(10.0),
                weak,
            );
        }
        painter.text(
            pos2(area.left(), area.top() + 87.0),
            Align2::LEFT_TOP,
            self.note,
            FontId::proportional(10.0),
            weak,
        );
        response
    }
}

fn bytes(value: u64) -> String {
    if value >= 1 << 30 {
        format!("{:.2} GiB", value as f64 / (1_u64 << 30) as f64)
    } else if value >= 1 << 20 {
        format!("{:.1} MiB", value as f64 / (1_u64 << 20) as f64)
    } else if value >= 1 << 10 {
        format!("{:.1} KiB", value as f64 / (1_u64 << 10) as f64)
    } else {
        format!("{value} B")
    }
}

pub fn show(ui: &mut egui::Ui, progress: Progress) {
    let amount = match (progress.replayed, progress.observed) {
        (Some(done), Some(total)) => format!("{} / {}", bytes(done), bytes(total)),
        _ => "Opening resource".into(),
    };
    let replay = match progress.pending_bytes() {
        Some(pending) if pending > 0 => format!("+{}", bytes(pending)),
        Some(_) if progress.replay_fraction().is_some() => "bytes replayed".into(),
        _ => "bytes pending".into(),
    };
    let stage = match progress.phase {
        Phase::Opening => "opening…",
        Phase::Replay => "replaying…",
        Phase::Snapshot => "snapshot…",
        Phase::Ready => "snapshot ready",
        Phase::Failed => "unavailable",
    };
    ui.add(Instrument {
        title: "PILE RESOURCE",
        amount: &amount,
        fraction: progress.replay_fraction(),
        accent: egui::Color32::from_rgb(53, 203, 214),
        stages: [&replay, stage, ""],
        note: "",
    });
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
        for width in [320.0, 640.0, 1100.0] {
            let context = egui::Context::default();
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 300.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui,
                        Progress {
                            replayed: Some(1 << 30),
                            observed: Some(2 << 30),
                            phase: Phase::Snapshot,
                            ..Default::default()
                        },
                    );
                    assert!(ui.min_rect().width() <= width);
                    assert!(ui.min_rect().height() <= 150.0);
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
}
