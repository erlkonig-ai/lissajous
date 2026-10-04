//! Synthetic presentation fixture; never constructs a PileCell or opens a file.
//! Capture via the actual Lissajous renderer:
//! `pile_instrument --headless --theme light --out-dir /tmp/pile-face-light`
//! and repeat with `--theme dark`. Each card contains the same phases at a
//! different width, including the narrow footprint of a detached instrument.

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
                    let cases = [
                (Phase::Opening, None, None, None, false),
                (
                    Phase::Replay,
                    Some(100_000_000_000),
                    Some(200_000_000_000),
                    None,
                    true,
                ),
                (
                    Phase::Replay,
                    Some(84_325_103_000),
                    Some(233_305_898_752),
                    None,
                    true,
                ),
                (
                    Phase::Replay,
                    Some(233_305_898_752),
                    Some(233_305_898_752),
                    None,
                    true,
                ),
                (
                    Phase::Snapshot,
                    Some(233_305_898_752),
                    Some(233_305_898_752),
                    None,
                    true,
                ),
                (
                    Phase::Ready,
                    Some(233_305_898_752),
                    Some(233_305_898_752),
                    None,
                    true,
                ),
                (
                    Phase::Ready,
                    Some(233_305_898_752),
                    Some(241_305_898_752),
                    None,
                    true,
                ),
                (Phase::Replay, Some(233_305_898_752), None, None, true),
                (Phase::Ready, Some(0), Some(0), None, true),
                (
                    Phase::Failed,
                    Some(84_325_103_000),
                    Some(233_305_898_752),
                    Some(
                        "Invalid record at byte 84325103000; the last snapshot remains available.",
                    ),
                    true,
                ),
                (
                    Phase::Failed,
                    None,
                    None,
                    Some("Permission denied while opening this source."),
                    false,
                ),
            ];
                    for (phase, replayed, observed, error, refreshable) in cases {
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
                            .error(error)
                            .refreshable(refreshable),
                        );
                        assert!((response.rect.width() - width).abs() < 1.0);
                        ui.add_space(8.0);
                    }
                    ui.add(PileProgress::new(
                        Path::new("/public/observations/coast-team/project.pile"),
                        Progress {
                            phase: Phase::Ready,
                            replayed: Some(1_234_567),
                            observed: Some(1_234_567),
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
