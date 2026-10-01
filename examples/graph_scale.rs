//! Time the CPU force layout at the sizes the three views actually reach.
//!
//! This exists because the decision it feeds is otherwise made by instinct:
//! whether a GPU is warranted for a given view, or whether reaching for one is
//! laziness dressed as ambition. Run it, read the numbers, then decide.
//!
//! Every figure it prints is **milliseconds per solver step**, single core,
//! release build, on the named machine — not per frame, and not per token of
//! anything. A step is one force evaluation plus one integration plus the two
//! momentum corrections. A view that wants 4 ms of solver per frame gets
//! `4 / <figure>` steps of settling per frame at that size.
//!
//! ```text
//! cargo run --release --example graph_scale
//! ```

use lissajous::graph::{ForceLayout, LayoutParams};

fn ring_with_chords(nodes: usize) -> Vec<(u32, u32)> {
    let mut edges = Vec::with_capacity(nodes * 2);
    for index in 0..nodes as u32 {
        edges.push((index, (index + 1) % nodes as u32));
        if index % 3 == 0 {
            edges.push((index, (index + 37) % nodes as u32));
        }
    }
    edges
}

fn time(nodes: usize, theta: f32, steps: usize) -> f64 {
    let edges = ring_with_chords(nodes);
    let params = LayoutParams {
        theta,
        ..LayoutParams::default()
    };
    let mut layout = ForceLayout::new(nodes, &edges, params);
    // Warm past the seed transient so the tree is the shape it will spend its
    // life being, rather than one ring's worth of degenerate cells.
    for _ in 0..20 {
        layout.step();
    }
    // The MEDIAN of several reps, not one run. This box is shared with other
    // agents and a single rep swung ninefold between two runs of this very
    // example; a number taken once here is a measurement of the neighbours.
    let mut reps: Vec<f64> = Vec::new();
    for _ in 0..5 {
        let start = std::time::Instant::now();
        for _ in 0..steps {
            layout.step();
        }
        reps.push(start.elapsed().as_secs_f64() * 1000.0 / steps as f64);
    }
    reps.sort_by(f64::total_cmp);
    reps[reps.len() / 2]
}

/// Compare the two seeds at the wiki graph's live size.
///
/// Framing rule: these are kinetic energy, mean speed and world radius after a
/// fixed number of steps of the SAME force law, with only the opening radius
/// differing. Not wall clock, not a claim about any machine — a claim about how
/// far from rest each seed leaves the layout after the same amount of
/// simulation, on this topology.
///
/// Run at BOTH heat settings, because they answer different questions and the
/// difference between them is the point. With the schedule on, every node is at
/// the heat floor within a few hundred steps and both runs creep at the same
/// rate whatever their radius, so the comparison measures the schedule rather
/// than the seed.
fn seeds(nodes: usize, steps: usize, cool: f32) {
    let edges = ring_with_chords(nodes);
    let params = LayoutParams {
        cool,
        ..LayoutParams::default()
    };
    let counted_radius = 200.0 + nodes as f32 * 5.0;
    let area_radius = params.seed_scale * (nodes as f32).sqrt();

    let mut by_count = ForceLayout::new(nodes, &edges, params);
    by_count.seed_ring(counted_radius);
    let mut by_area = ForceLayout::new(nodes, &edges, params);

    let mut counted = by_count.stats();
    let mut area = by_area.stats();
    for _ in 0..steps {
        counted = by_count.step();
        area = by_area.step();
    }
    let radius = |stats: &lissajous::graph::LayoutStats| {
        ((stats.bounds[2] - stats.bounds[0]) + (stats.bounds[3] - stats.bounds[1])) * 0.25
    };
    println!(
        "{nodes} nodes, {steps} steps, cool = {cool}, identical law, only the seed differs:\n  \
         ring 200 + 5n   = {counted_radius:>6.0}: KE {:>9.0}  mean speed {:>6.3}  settled radius {:>7.0}\n  \
         ring 40*sqrt(n) = {area_radius:>6.0}: KE {:>9.0}  mean speed {:>6.3}  settled radius {:>7.0}",
        counted.kinetic_energy,
        counted.mean_speed,
        radius(&counted),
        area.kinetic_energy,
        area.mean_speed,
        radius(&area),
    );
}

fn main() {
    println!("ms per solver step, single core, this machine");
    // Below a few hundred nodes the tree build costs more than the pairs it
    // saves, and all-pairs wins — measured at 200 nodes, where it is several
    // times faster. It is not worth a special case: the whole step is well
    // under a millisecond at that size either way, and a branch on node count
    // would be two code paths to keep honest in exchange for nothing a reader
    // could perceive.
    println!("(all-pairs wins below a few hundred nodes; the tree build is the cost)");
    println!(
        "{:>8}  {:>10}  {:>12}  {:>10}",
        "nodes", "theta=0.7", "all-pairs", "speedup"
    );
    // The sizes are the real ones: a colony JP wants to reach, the wiki's live
    // graph, one collection's join lattice, and a whole derivation chain.
    for (nodes, steps, exact) in [
        (200usize, 200usize, true),
        (1_000, 100, true),
        (2_163, 50, true),
        (20_169, 10, false),
        (45_821, 5, false),
    ] {
        let approximate = time(nodes, 0.7, steps);
        if exact {
            let all_pairs = time(nodes, 0.0, steps.min(40));
            println!(
                "{nodes:>8}  {approximate:>10.2}  {all_pairs:>12.2}  {:>9.1}x",
                all_pairs / approximate
            );
        } else {
            println!("{nodes:>8}  {approximate:>10.2}  {:>12}  {:>10}", "-", "-");
        }
    }

    println!();
    // The physics, with the heat schedule out of the way.
    seeds(3_382, 1_200, 1.0);
    println!();
    // And as the widget actually runs it.
    seeds(3_382, 1_200, LayoutParams::default().cool);
}
