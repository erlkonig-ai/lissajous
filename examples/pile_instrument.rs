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
            ctx.set_width(width);
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
                ctx.add(
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
                ctx.add_space(8.0);
            }
            ctx.add(PileProgress::new(
                Path::new("/public/observations/coast-team/project.pile"),
                Progress {
                    phase: Phase::Ready,
                    replayed: Some(1_234_567),
                    observed: Some(1_234_567),
                    ..Default::default()
                },
            ));
        });
    }
    nb.settled();
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The native pile instrument is currently exposed on Unix.");
}
