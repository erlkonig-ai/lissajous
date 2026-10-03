//! Cell-owned instruments: private map state, useful public selection.
//! Tile-free synthetic geometry, not evidence from a real station.
//! Run with `cargo run --example geospatial_map`; see README's authoring guide.
use lissajous::prelude::*;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    // A visible public-value card is useful here, not required for every output.
    let selection = nb.state("selection", None::<u64>, |ctx, selected| {
        ctx.heading("Shared selection");
        ctx.label(format!("Selected entity: {selected:?}"));
        if ctx.button("Clear selection").clicked() {
            *selected = None;
            ctx.ctx().request_repaint();
        }
    });
    map::cell(nb, selection);
    selection_summary(nb, selection);
}

// This consumer could receive exactly the same input from a list selector.
// It neither knows about nor locks the map's camera or prepared geometry.
fn selection_summary(nb: &mut NotebookCtx, selection: StateId<Option<u64>>) {
    nb.state_with(
        "selection-summary",
        DerivedState::<Option<u64>, String>::default,
        move |ctx, label| {
            let selected = *selection.read(ctx); // Release the guard before drawing.
            let text = label.get(selected, |key| match key {
                Some(id) => format!("Ready to inspect entity {id}"),
                None => "Choose an entity with any selector".to_owned(),
            });
            ctx.heading("Independent consumer");
            ctx.label(text.as_str());
        },
    );
}

mod map {
    use eframe::egui::Color32;
    use lissajous::prelude::*;

    // Neither the type nor its handle escapes this instrument's module.
    struct MapState {
        camera: MapCamera,
        // Tiny prepared geometry stands in for a cell-local drawing cache.
        // A real dataset would be a shared source, not copied into this state.
        route: [GeoPosition; 3],
    }

    impl Default for MapState {
        fn default() -> Self {
            Self {
                camera: MapCamera::default(),
                route: [
                    GeoPosition([4.42, 52.22]),
                    GeoPosition([4.45, 52.25]),
                    GeoPosition([4.51, 52.24]),
                ],
            }
        }
    }

    pub(super) fn cell(nb: &mut NotebookCtx, selection: StateId<Option<u64>>) {
        nb.state_with("geospatial-map", MapState::default, move |ctx, state| {
            ctx.heading("Located observations");
            ctx.label("Synthetic geometry · pan · wheel to zoom · click to select");
            let cyan = Color32::from_rgb(45, 185, 185);
            let orange = Color32::from_rgb(230, 145, 55);
            let features = [
                MapFeature {
                    id: 101,
                    geometry: MapGeometry::Point(state.route[0]),
                    label: "Synthetic station A",
                    color: cyan,
                },
                MapFeature {
                    id: 202,
                    geometry: MapGeometry::Point(state.route[2]),
                    label: "Synthetic station B",
                    color: cyan,
                },
                MapFeature {
                    id: 303,
                    geometry: MapGeometry::Path(&state.route),
                    label: "Synthetic reach",
                    color: orange,
                },
            ];
            let legend = [
                MapLegendEntry { label: "station", color: cyan },
                MapLegendEntry { label: "reach", color: orange },
            ];
            // Read/copy/drop before drawing or taking the output's write lock.
            // The selection card never locks MapState, so there is no inverse
            // selection -> map lock path. Never reacquire this cell's own state.
            let before = *selection.read(ctx);
            let mut selected = before;
            MapView::new(&features, &mut state.camera, &mut selected)
                .legend(&legend)
                .show(ctx);
            if selected != before {
                *selection.read_mut(ctx) = selected;
                // The earlier selection card also needs to observe this edit.
                ctx.ctx().request_repaint();
            }
        });
    }
}
