#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = "..", features = ["polars"] }
//! egui = "0.33"
//! polars = "0.50.0"
//! parking_lot = "0.12.3"
//! ```

use egui::Margin;
use polars::prelude::*;
use lissajous::dataflow::ComputedState;
use lissajous::md;
use lissajous::notebook;
use lissajous::widgets::{
    data_export_tiny, data_summary_tiny, dataframe, dataframe_summary, load_auto,
};
use lissajous::NotebookCtx;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let padding = lissajous::cards::DEFAULT_CARD_PADDING;
    let summary_padding = padding;
    let dataframe_padding = Margin {
        left: padding.left,
        right: padding.right,
        top: padding.top,
        bottom: 0,
    };
    nb.view(|ui| {
        md!(
            ui,
            "# Polars
In this notebook we're going to use the `polars` crate to create a simple dataframe."
        );
    });
    let df_state = nb.state(
        "dataframe",
        ComputedState::<Option<DataFrame>>::default,
        move |ui, value| {
            ui.with_padding(summary_padding, |ctx| {
                md!(ctx, "*Overview*: row/column counts for quick context.");
                ctx.add_space(6.0);
                let df = load_auto(ctx, value, Option::is_none, || {
                    let df = CsvReadOptions::default()
                        .try_into_reader_with_file_path(Some("./assets/datasets/iris.csv".into()))
                        .unwrap()
                        .finish()
                        .unwrap();
                    Some(df)
                });
                if let Some(df) = df.as_ref() {
                    data_summary_tiny(ctx, df, df);
                }
            });
        },
    );

    let view_state = nb.state(
        "dataframe_view_response",
        || Ok(DataFrame::default()),
        move |ui, view_state| {
            let Some(state) = df_state.try_read(ui) else {
                return;
            };
            let Some(df) = state.value().as_ref() else {
                return;
            };
            ui.with_padding(dataframe_padding, |ctx| {
                md!(ctx, "*Table*: SQL query + sortable view.");
                ctx.add_space(6.0);
                *view_state = dataframe(ctx, df);
            });
        },
    );

    nb.view(move |ui| {
        let Some(state) = df_state.try_read(ui) else {
            return;
        };
        let Some(df) = state.value().as_ref() else {
            return;
        };
        let view_state = view_state.try_read(ui);
        ui.with_padding(summary_padding, |ctx| {
            md!(ctx, "*Summary*: per-column nulls and quick stats.");
            ctx.add_space(6.0);
            let active_df = view_state
                .as_ref()
                .and_then(|state| state.as_ref().ok())
                .unwrap_or(df);
            dataframe_summary(ctx, active_df);
        });
    });

    nb.view(move |ui| {
        let Some(state) = df_state.try_read(ui) else {
            return;
        };
        let Some(df) = state.value().as_ref() else {
            return;
        };
        let view_state = view_state.try_read(ui);
        ui.with_padding(summary_padding, |ctx| {
            md!(ctx, "*Export*: copy or save the dataframe as CSV.");
            ctx.add_space(6.0);
            let active_df = view_state
                .as_ref()
                .and_then(|state| state.as_ref().ok())
                .unwrap_or(df);
            data_export_tiny(ctx, active_df);
        });
    });
}
