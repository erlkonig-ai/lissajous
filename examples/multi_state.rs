#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = ".." }
//! egui = "0.33"
//! ```

use lissajous::cards::DEFAULT_CARD_PADDING;
use lissajous::prelude::*;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let left = nb.state("left", 2_i64, |ctx, value| {
        ctx.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Left counter");
            ctx.horizontal(|ctx| {
                if ctx.add(widgets::Button::new("-1")).clicked() {
                    *value -= 1;
                }
                if ctx.add(widgets::Button::new("+1")).clicked() {
                    *value += 1;
                }
            });
            widgets::markdown(ctx, &format!("Value: `{value}`"));
        });
    });

    let right = nb.state("right", 5_i64, |ctx, value| {
        ctx.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Right counter");
            ctx.horizontal(|ctx| {
                if ctx.add(widgets::Button::new("-1")).clicked() {
                    *value -= 1;
                }
                if ctx.add(widgets::Button::new("+1")).clicked() {
                    *value += 1;
                }
            });
            widgets::markdown(ctx, &format!("Value: `{value}`"));
        });
    });

    nb.view(move |ctx| {
        let left = left.read(ctx);
        let right = right.read(ctx);
        ctx.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Combined view (reads both states together)");
            widgets::markdown(
                ctx,
                &format!(
                    "Left: `{}`\nRight: `{}`\nSum: `{}`",
                    *left,
                    *right,
                    *left + *right
                ),
            );
        });
    });

    nb.view(move |ctx| {
        let mut left = left.read_mut(ctx);
        let mut right = right.read_mut(ctx);
        ctx.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Combined update (locks both states)");

            if ctx.add(widgets::Button::new("Add 10 to both")).clicked() {
                *left += 10;
                *right += 10;
            }
            if ctx.add(widgets::Button::new("Swap values")).clicked() {
                std::mem::swap(&mut *left, &mut *right);
            }

            widgets::markdown(ctx, &format!("Left: `{}`\nRight: `{}`", *left, *right));
        });
    });
}
