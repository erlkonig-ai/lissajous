#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = "..", features = ["plots"] }
//! egui = "0.34"
//! egui_plot = "0.35"
//! ```
//!
//! Live-dashboard widget tour: `StreamLane`, `BudgetGauge`,
//! `MetricStrip`, and `EventFeed`, all fed with synthetic data.

use std::time::Duration;

use lissajous::notebook;
use lissajous::widgets;
use lissajous::widgets::{BudgetGauge, EventFeed, MetricStrip, StreamLane};
use lissajous::NotebookCtx;

const LANE_TAGS: [&str; 4] = ["sense", "plan", "act", "note"];

#[derive(Default)]
struct LaneState {
    lane: StreamLane<&'static str>,
    emitted: usize,
}

#[derive(Default)]
struct FeedState {
    feed: EventFeed,
    emitted: usize,
}

struct StripState {
    strip: MetricStrip,
}

/// Synthetic "frame time" in ms, oscillating across the 80 ms budget.
fn synthetic_ms(t: f64) -> f64 {
    60.0 + 25.0 * (t * 0.6).sin() + 8.0 * (t * 2.9).sin()
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let padding = lissajous::cards::DEFAULT_CARD_PADDING;

    nb.view(move |ctx| {
        widgets::markdown(
            ctx,
            "# Dashboard widgets\n\nA tour of the live-dashboard widgets with synthetic data: \
             `StreamLane` (tagged text runs), `BudgetGauge` (scalar vs budget), `MetricStrip` \
             (rolling time-series), and `EventFeed` (newest-first rows).",
        );
    });

    // ── StreamLane ───────────────────────────────────────────────────
    nb.state("lane", LaneState::default, move |ctx, state| {
        ctx.with_padding(padding, |ctx| {
            ctx.heading("StreamLane");
            let t = ctx.input(|i| i.time);
            let due = (t * 4.0) as usize;
            while state.emitted < due {
                let n = state.emitted;
                let tag = LANE_TAGS[n % LANE_TAGS.len()];
                state.lane.push(
                    tag,
                    format!(
                        "[{tag}] event {n}: value={:.2}\n",
                        synthetic_ms(n as f64 * 0.25)
                    ),
                );
                state.emitted += 1;
            }
            state.lane.show(ctx);
            ctx.ctx().request_repaint_after(Duration::from_millis(100));
        });
    });

    // ── BudgetGauge ──────────────────────────────────────────────────
    nb.view(move |ctx| {
        ctx.with_padding(padding, |ctx| {
            ctx.heading("BudgetGauge");
            let t = ctx.input(|i| i.time);
            ctx.add(
                BudgetGauge::new(synthetic_ms(t), 80.0)
                    .label("frame")
                    .suffix(" ms"),
            );
            ctx.ctx().request_repaint_after(Duration::from_millis(50));
        });
    });

    // ── MetricStrip ──────────────────────────────────────────────────
    nb.state(
        "strip",
        || StripState {
            strip: MetricStrip::new("frame time", 600)
                .suffix(" ms")
                .percentile_band(true),
        },
        move |ctx, state| {
            ctx.with_padding(padding, |ctx| {
                ctx.heading("MetricStrip");
                let t = ctx.input(|i| i.time);
                state.strip.push(t, synthetic_ms(t));
                state.strip.show(ctx);
                ctx.ctx().request_repaint_after(Duration::from_millis(50));
            });
        },
    );

    // ── EventFeed ────────────────────────────────────────────────────
    nb.state("feed", FeedState::default, move |ctx, state| {
        ctx.with_padding(padding, |ctx| {
            ctx.heading("EventFeed");
            let t = ctx.input(|i| i.time);
            let due = (t / 1.2) as usize;
            while state.emitted < due {
                let n = state.emitted;
                let category = LANE_TAGS[n % LANE_TAGS.len()];
                if n % 3 == 0 {
                    state.feed.push_with_detail(
                        category,
                        format!("event {n} finished"),
                        format!(
                            "synthetic detail for event {n}:\nvalue = {:.2} ms\nphase = {}",
                            synthetic_ms(n as f64),
                            n % 7
                        ),
                    );
                } else {
                    state.feed.push(category, format!("event {n} finished"));
                }
                state.emitted += 1;
            }
            state.feed.show(ctx);
            ctx.ctx().request_repaint_after(Duration::from_millis(200));
        });
    });
}
