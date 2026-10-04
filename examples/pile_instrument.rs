//! Synthetic presentation fixture; never constructs a PileCell or opens a file.
//! Capture using `pile_instrument --headless --theme light --out-dir ...`
//! and repeat with `--theme dark`. The actual faces are 180, 320 and 640 points.

#[cfg(unix)]
use lissajous::prelude::*;

#[cfg(unix)]
#[notebook]
fn main(nb: &mut NotebookCtx) {
    use lissajous::widgets::triblespace::pile::{Phase, PileProgress, Progress};
    use std::path::Path;

    for width in [180.0, 320.0, 640.0] {
        nb.view(move |ctx| {
            let start = ctx.next_widget_position();
            ctx.ui_mut().scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(start, egui::vec2(width, 0.0)))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_width(width);
                    let total = Some(233_305_898_752);
                    let part = Some(84_325_103_000);
                    let error =
                        "Invalid record at byte 84325103000; the last snapshot remains available.";
                    let cases = [
                        (Phase::Opening, None, None, None),
                        (
                            Phase::Replay,
                            Some(100_000_000_000),
                            Some(200_000_000_000),
                            None,
                        ),
                        (Phase::Replay, part, total, None),
                        (Phase::Replay, total, total, None),
                        (Phase::Snapshot, total, total, None),
                        (Phase::Ready, total, total, None),
                        (Phase::Ready, total, Some(241_305_898_752), None),
                        (Phase::Replay, total, None, None),
                        (Phase::Ready, Some(0), Some(0), None),
                        (Phase::Failed, part, total, Some(error)),
                        (
                            Phase::Failed,
                            None,
                            None,
                            Some("Permission denied while opening this source."),
                        ),
                    ];
                    for (phase, replayed, observed, error) in cases {
                        let response = ui.add(
                            PileProgress::new(
                                Path::new("/public/observations/delta-team/project.pile"),
                                Progress {
                                    phase,
                                    replayed,
                                    observed,
                                    ..Default::default()
                                },
                            )
                            .error(error),
                        );
                        assert!((response.rect.width() - width).abs() < 1.0);
                        if error.is_none() {
                            assert_eq!(response.rect.height(), 28.0);
                        }
                        ui.add_space(8.0);
                    }
                    ui.add(PileProgress::new(
                        Path::new("/public/observations/coast-team/project.pile"),
                        Progress {
                            phase: Phase::Ready,
                            replayed: total,
                            observed: total,
                            ..Default::default()
                        },
                    ));
                },
            );
        });
    }
    nb.settled();
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The native pile instrument is currently exposed on Unix.");
}
