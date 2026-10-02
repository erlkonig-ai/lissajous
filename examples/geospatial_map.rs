//! Tile-free, synthetic map tour. Geometry is not evidence from a real station.
use eframe::egui::Color32;
use lissajous::prelude::*;

#[derive(Default)]
struct MapState {
    camera: MapCamera,
    selected: Option<u64>,
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.state("geospatial-map", MapState::default(), |ui, state| {
        ui.heading("Located observations");
        ui.label("Synthetic geometry · pan to inspect · wheel to zoom · click to select");
        let route = [
            GeoPosition([4.42, 52.22]),
            GeoPosition([4.45, 52.25]),
            GeoPosition([4.51, 52.24]),
        ];
        let cyan = Color32::from_rgb(45, 185, 185);
        let orange = Color32::from_rgb(230, 145, 55);
        let features = [
            MapFeature {
                id: 101,
                geometry: MapGeometry::Point(route[0]),
                label: "Synthetic station A",
                color: cyan,
            },
            MapFeature {
                id: 202,
                geometry: MapGeometry::Point(route[2]),
                label: "Synthetic station B",
                color: cyan,
            },
            MapFeature {
                id: 303,
                geometry: MapGeometry::Path(&route),
                label: "Synthetic reach",
                color: orange,
            },
        ];
        let legend = [
            MapLegendEntry {
                label: "station",
                color: cyan,
            },
            MapLegendEntry {
                label: "reach",
                color: orange,
            },
        ];
        let result = MapView::new(&features, &mut state.camera, &mut state.selected)
            .legend(&legend)
            .show(ui);
        if let Some(id) = result.selected {
            ui.label(format!("Caller entity: {id}"));
        }
    });
}
