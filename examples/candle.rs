#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = ".." }
//! egui = "0.33"
//! ```

use lissajous::md;
use lissajous::notebook;
use lissajous::widgets;
use lissajous::NotebookCtx;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let padding = lissajous::cards::DEFAULT_CARD_PADDING;
    nb.view(|ctx| {
        md!(
            ctx,
            "# Candle
In this notebook we're going to use huggingfaces `candle` crate, to create a simple prompt based chatbot."
        );
    });
    let _prompt = nb.state("prompt", "", move |ctx, value| {
        ctx.with_padding(padding, |ctx| {
            ctx.horizontal(|ctx| {
                ctx.label("Prompt:");
                ctx.add(widgets::TextField::singleline(value));
                if ctx.button("Send").clicked() {
                    // send the prompt to the chatbot
                }
            });
        });
    });

    nb.view(move |ctx| {
        ctx.with_padding(padding, |_ctx| {});
    });
}
