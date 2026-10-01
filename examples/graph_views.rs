//! The mesh and the collection lattice, on fixtures that carry the absences.
//!
//! This exists for two reasons that unit tests cannot cover.
//!
//! **It is the absence fixture.** Non-resident descriptors, declared-but-never-
//! performed derivations and members reached only as somebody else's join input
//! are the point of the lattice view, and none of them has ever been seen on
//! live data — every pile we own reports "0 with no resident descriptor". A
//! clean render must not be allowed to imply the path works, so the path is
//! exercised here, deliberately, on data that is labelled as invented.
//!
//! **It renders.** The tests drive `egui` headlessly and ask the painter what
//! shapes came out, which is the right question for a settling layout — it has
//! no pixel identity to compare against, and pretending otherwise would be the
//! determinism this design rejects. But shape kinds are not a picture. This
//! notebook is the picture, and `--headless` turns it into PNGs.
//!
//! ```sh
//! cargo run --release --example graph_views -- --headless --out-dir /tmp/graph_views
//! ```

use lissajous::notebook;
use lissajous::widgets::{
    LatticeEdge, LatticeGraph, LatticeMark, LatticeNode, LatticePresence, MeshGraph, MeshLink,
    MeshNode, MeshNodeState,
};
use lissajous::NotebookCtx;

/// The colony as it is today: three peers, which is the size the ring anchor
/// exists to keep honest.
fn small_mesh() -> (Vec<MeshNode>, Vec<MeshLink>) {
    let nodes = vec![
        MeshNode {
            label: "sky".into(),
            state: MeshNodeState::Fresh,
            convergence: Some(1.0),
        },
        MeshNode {
            label: "stars".into(),
            state: MeshNodeState::Stale,
            // A measured ZERO, not a missing measurement. Without the datum
            // tick these two render identically, which is the one thing this
            // view exists to prevent.
            convergence: Some(0.0),
        },
        MeshNode {
            label: "mac".into(),
            state: MeshNodeState::Unreported,
            convergence: None,
        },
    ];
    let links = vec![
        MeshLink { from: 0, to: 1 },
        MeshLink { from: 1, to: 2 },
        MeshLink { from: 0, to: 2 },
    ];
    (nodes, links)
}

/// A thousand peers, clustered by site, with a scatter of exceptions.
///
/// The size JP asked the design to reach. What it is here to show is that the
/// level of detail decimates the NORM: the fresh majority folds as the zoom
/// drops while every exceptional peer keeps its full mark, so the handful of
/// nodes somebody opened the view for survive the densest part of the field.
fn large_mesh() -> (Vec<MeshNode>, Vec<MeshLink>) {
    const SITES: usize = 14;
    const PER_SITE: usize = 72;
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for site in 0..SITES {
        for member in 0..PER_SITE {
            let index = site * PER_SITE + member;
            // Exceptions are rare by definition, and a colony where that stops
            // being true is a colony whose picture is telling you so.
            let state = match (site, member) {
                (3, 5) | (3, 6) | (3, 7) => MeshNodeState::Stale,
                (9, 0) => MeshNodeState::Unreported,
                (11, 4) => MeshNodeState::Unknown,
                _ => MeshNodeState::Fresh,
            };
            nodes.push(MeshNode {
                label: format!("n{index:04}"),
                state,
                convergence: match state {
                    MeshNodeState::Fresh => Some(0.6 + (member % 5) as f32 * 0.08),
                    MeshNodeState::Stale => Some(0.0),
                    _ => None,
                },
            });
            if member > 0 {
                links.push(MeshLink {
                    from: index - 1,
                    to: index,
                });
            }
            if member % 9 == 0 {
                links.push(MeshLink {
                    from: site * PER_SITE,
                    to: index,
                });
            }
        }
        // One thin link between neighbouring sites, which is what makes the
        // clustering a latent structure rather than fourteen separate graphs.
        links.push(MeshLink {
            from: site * PER_SITE,
            to: ((site + 1) % SITES) * PER_SITE,
        });
    }
    (nodes, links)
}

/// A derivation forest carrying every absence the view can draw.
fn lattice() -> (Vec<LatticeNode>, Vec<LatticeEdge>) {
    let nodes = vec![
        LatticeNode {
            label: "facts".into(),
            mark: LatticeMark::Authored,
            coverage: Some(1.0),
            ..LatticeNode::default()
        },
        LatticeNode {
            label: "index".into(),
            coverage: Some(0.62),
            ..LatticeNode::default()
        },
        // A hole: named by a record, and its bytes are not here.
        LatticeNode {
            label: "summary".into(),
            presence: LatticePresence::Absent,
            coverage: None,
            ..LatticeNode::default()
        },
        // Reached only as somebody else's join input. Resident, real, and made
        // by a record this observation cannot see. Until it had a glyph it was
        // pixel identical to the fully-produced output beside it.
        LatticeNode {
            label: "joined".into(),
            produced: false,
            coverage: Some(0.0),
            ..LatticeNode::default()
        },
        // A measured zero on the track, beside a node with no measurement at
        // all: the datum tick is the only thing that tells them apart.
        LatticeNode {
            label: "rollup".into(),
            coverage: Some(0.0),
            ..LatticeNode::default()
        },
        LatticeNode {
            label: "unmeasured".into(),
            coverage: None,
            ..LatticeNode::default()
        },
    ];
    let edges = vec![
        LatticeEdge {
            from: 0,
            to: 1,
            endorsed: true,
        },
        // Declared and never performed: the descriptor says this derivation
        // should exist and no record endorses it.
        LatticeEdge {
            from: 1,
            to: 2,
            endorsed: false,
        },
        LatticeEdge {
            from: 1,
            to: 3,
            endorsed: true,
        },
        LatticeEdge {
            from: 3,
            to: 4,
            endorsed: true,
        },
        LatticeEdge {
            from: 0,
            to: 5,
            endorsed: false,
        },
    ];
    (nodes, edges)
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.view(|ctx| {
        ctx.section("Peer mesh, ring-sized", |ctx| {
            let (nodes, links) = small_mesh();
            let ui = ctx.ui_mut();
            ui.add(
                MeshGraph::new(&nodes, &links)
                    .height(300.0)
                    .id_salt("small"),
            );
            ui.small(
                "SOURCE FIXTURE. Filled square reported freshly; open square reported \
                 stale; dashed square was never heard from here. The tick at twelve \
                 o'clock says a measurement EXISTS — stars reported exactly zero, mac \
                 reported nothing, and without the tick those two tracks are the same \
                 picture.",
            );
        });
    });

    nb.view(|ctx| {
        ctx.section("Peer mesh, a thousand peers", |ctx| {
            let (nodes, links) = large_mesh();
            let ui = ctx.ui_mut();
            ui.add(
                MeshGraph::new(&nodes, &links)
                    .height(560.0)
                    .id_salt("large"),
            );
            ui.small(
                "SOURCE FIXTURE. 1008 peers across 14 sites, 5 of them exceptional. \
                 The level of detail decimates the norm and never the exception: the \
                 fresh majority folds to a dot as the zoom drops while every stale, \
                 unreported or unplaceable peer keeps its full mark over a knockout \
                 of the page ground. Drag to pan, pinch or command-scroll to zoom.",
            );
        });
    });

    nb.view(|ctx| {
        ctx.section("Collection lattice, with its holes", |ctx| {
            let (nodes, edges) = lattice();
            let ui = ctx.ui_mut();
            let drawn = LatticeGraph::new(&nodes, &edges)
                .id_salt("lattice")
                .show(ui);
            ui.small(format!(
                "SOURCE FIXTURE — every absence below is invented, because none of \
                 them has ever appeared on a pile we own. Square is authored, circle \
                 is computed, a circle with a gap at the bottom is a member nothing \
                 here produced. Dashed is a hole. A dashed edge is a derivation the \
                 model expects and no record endorses. DRAWN {} · FOLDED {}",
                drawn.drawn, drawn.folded
            ));
        });
    });
}
