//! Read-only geometry and real, initialized Rapier/Salva snapshots.
//! Nothing in this example advances a physics simulation.
//!
//! `cargo run --example physics_widgets`
//! `cargo run --example physics_widgets --features rapier,salva`

use egui::Color32;
use lissajous::widgets::{Bounds3, Label3, LegendEntry, PhysicsScene, PhysicsView};
use lissajous::{notebook, NotebookCtx};

fn geometry() -> PhysicsScene {
    let mut scene = PhysicsScene::default();
    let color = Color32::from_rgb(155, 175, 190);
    scene.wire_box(
        Bounds3 {
            min: [-0.8, -0.5, -0.5],
            max: [0.8, 0.5, 0.5],
        },
        color,
    );
    scene.labels.push(Label3::new(
        [-0.8, 0.55, -0.5],
        "Measured envelope / caller geometry",
        color,
    ));
    scene.legend.push(LegendEntry::new(
        "Generic wire-box geometry; not a simulation result",
        color,
    ));
    scene
}

#[cfg(feature = "rapier")]
fn rigid_snapshot() -> PhysicsScene {
    use rapier3d_f64::prelude::*;
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    colliders.insert(ColliderBuilder::cuboid(0.8, 0.5, 0.5).build());
    let body = bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(Vector::new(0.0, 0.8, 0.0))
            .rotation(Vector::new(0.0, 0.0, 0.25))
            .build(),
    );
    colliders.insert_with_parent(
        ColliderBuilder::capsule_y(0.2, 0.15).build(),
        body,
        &mut bodies,
    );
    colliders.insert(
        ColliderBuilder::ball(0.2)
            .translation(Vector::new(1.2, 0.0, 0.0))
            .build(),
    );
    let mut scene = lissajous::widgets::physics::rapier::scene(&bodies, &colliders);
    scene
        .warnings
        .push("Real initialized ColliderSet/RigidBodySet; no dynamics have been stepped.".into());
    scene
}

#[cfg(feature = "salva")]
fn fluid_snapshot() -> PhysicsScene {
    use salva3d_f64::{math::Vector, object::Fluid};
    let mut positions = Vec::new();
    for x in 0..10 {
        for y in 0..5 {
            for z in 0..5 {
                positions.push(Vector::new(
                    -0.65 + x as f64 * 0.14,
                    -0.3 + y as f64 * 0.14,
                    -0.3 + z as f64 * 0.14,
                ));
            }
        }
    }
    let fluid = Fluid::new(positions, 0.065, 1000.0, Default::default());
    let mut scene = lissajous::widgets::physics::salva::fluid_scene(&fluid);
    scene.warnings.push(
        "Actual initialized Fluid positions/radius; no fluid solve or equilibrium claim.".into(),
    );
    scene
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let scene = geometry();
    nb.state("geometry-camera", PhysicsView::default().height(340.0), move |ctx, camera| {
        ctx.heading("Physics scene viewer");
        ctx.label("The camera is interactive; snapshots are read-only. Axes and scale use the caller's declared units.");
        camera.show(ctx, &scene);
    });

    #[cfg(any(feature = "rapier", feature = "salva"))]
    {
        #[cfg(feature = "rapier")]
        let scene = rigid_snapshot();
        #[cfg(not(feature = "rapier"))]
        let scene = geometry();
        #[cfg(feature = "salva")]
        let scene = {
            let mut scene = scene;
            scene.extend(fluid_snapshot());
            scene
        };
        // Keep the same envelope regardless of which optional adapters are used.
        let view = PhysicsView::default().height(420.0).bounds(Bounds3 {
            min: [-1.0, -0.7, -0.7],
            max: [1.5, 1.3, 0.7],
        });
        nb.state("adapter-camera", view, move |ctx, camera| {
            ctx.heading("Initialized physics objects — not simulated motion");
            camera.show(ctx, &scene);
        });
    }
}
