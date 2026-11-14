//! The force-directed layout, lifted from the wiki viewer.
//!
//! This is not a reimplementation. The force law, its five constants, the
//! integration step and the two momentum corrections are the ones
//! `faculties/src/widgets/wiki.rs` ran on the GPU, moved down here so the peer
//! mesh and the collection lattice can use the same physics the wiki graph
//! already did. [`LayoutParams::default`] *is* the wiki's tuned law, so the
//! existing production user is preserved by construction rather than by care,
//! and a test pins every constant so a later re-tune cannot reach it silently.
//!
//! Three things were added, each because a measurement demanded it rather than
//! because a cleaner shape suggested it:
//!
//! * **A Barnes–Hut far field.** The lifted kernel summed repulsion over every
//!   other node. At `theta = 0` this module still does, exactly; above it the
//!   far field is summarised. See [`crate::graph::tree`].
//! * **CSR adjacency.** The lifted kernel found node `i`'s edges by scanning
//!   every edge. The sum is identical; only the lookup changed.
//! * **A direction for coincident nodes.** The lifted law computes exactly zero
//!   force between two nodes at the same point — `dx / dist * f` with `dx = 0`
//!   — so they never separate. A ring seed hides it. Seeding a lattice by rank
//!   does not, which is how it was found.
//!
//! Nothing here touches `egui`, which is why it can be tested without a GPU,
//! without a window, and without a context.

use super::tree::{Csr, QuadTree};

/// Every constant the lifted kernel carried, plus what the new views need.
///
/// The first five are the wiki's, and their values are load-bearing tuning
/// with a history recorded in the kernel they came from: `damping` is a
/// velocity *retention* factor, and at the 0.75 it once had, attract/repel
/// pairs orbited each other forever because the energy sink was too weak to
/// let them settle. `attraction` went 0.3 → 0.15 so edges ease together rather
/// than snapping taut and overshooting. Do not re-tune them casually.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutParams {
    /// Inverse-square repulsion coefficient between every pair of nodes.
    pub repulsion: f32,
    /// Spring constant, divided by `sqrt(deg_i * deg_j)` so both endpoints of
    /// an edge feel the same magnitude. The symmetry is not cosmetic: an
    /// asymmetric weight injects net linear and angular momentum every step,
    /// which is what made an earlier layout spin forever.
    pub attraction: f32,
    /// Velocity *retention* per step. Lower drains orbital energy faster.
    pub damping: f32,
    /// Per-step impulse cap, in world units.
    pub max_force: f32,
    /// Per-axis stiffness of the anchor spring. This is the lifted `gravity`
    /// term generalised: at the default, with no anchors set, every node is
    /// pulled toward the world origin at `0.001`, which is exactly what the
    /// kernel's `f -= p * gravity` did.
    pub anchor_k: [f32; 2],
    /// Barnes–Hut acceptance ratio. `0.0` means exact all-pairs.
    pub theta: f32,
    /// Initial ring radius is `seed_scale * sqrt(n)`.
    pub seed_scale: f32,
    /// Per-step multiplier on every node's heat.
    pub cool: f32,
    /// Heat never falls below this. Energy is the channel that makes change
    /// visible, and a layout that reaches exactly zero has lost it for good.
    pub heat_floor: f32,
    /// Mean speed below which [`LayoutStats::quiet`] is set.
    pub quiet_speed: f32,

    // The fields below are a candidate law, OFF at the default: each default
    // reproduces the lifted law exactly, so `LayoutParams::default()` is still
    // the wiki's law constant for constant. [`LayoutParams::atlas`] turns them
    // on together.
    /// Exponent `p` of the repulsion `repulsion / r^p`. `2.0` is the lifted
    /// law; `1.0` is ForceAtlas2's.
    pub repulsion_falloff: f32,
    /// Give every node a mass of `1 + incident edges` (ForceAtlas2's
    /// `deg + 1`). A node then repels in proportion to its mass, its springs
    /// are divided by its own mass, and the anchor and the collision disc act
    /// per unit mass. Read as forces, that is ForceAtlas2's
    /// `k_r (d_i + 1)(d_j + 1) / r` repulsion with gravity proportional to
    /// `d_i + 1`; read as accelerations, a hub is heavy rather than jittery.
    /// Statics are the same either way, because only the equilibrium matters
    /// and dividing a zero force by a mass leaves it zero.
    pub degree_mass: bool,
    /// Divide each spring by `sqrt(deg_i * deg_j)`, as the lifted law does.
    /// `false` gives every edge the same spring, ForceAtlas2's attraction.
    pub spring_degree_norm: bool,
    /// LinLog attraction: a spring's pull grows with `ln(1 + d)` instead of
    /// `d`, which tightens clusters and lengthens the edges between them.
    pub spring_log: bool,
    /// Radius, in world units, of a bare node's no-overlap disc; a node's own
    /// disc is this times the wiki's degree scaling of its mark. `0.0` is off.
    pub collide_radius: f32,
    /// Spring constant on the overlap of two discs.
    pub collide_k: f32,
}

impl Default for LayoutParams {
    /// The wiki's law, constant for constant.
    fn default() -> Self {
        LayoutParams {
            repulsion: 140_000.0,
            attraction: 0.15,
            damping: 0.45,
            max_force: 15.0,
            anchor_k: [0.001, 0.001],
            theta: 0.7,
            seed_scale: 40.0,
            cool: 0.985,
            heat_floor: 0.05,
            quiet_speed: 0.5,
            repulsion_falloff: 2.0,
            degree_mass: false,
            spring_degree_norm: true,
            spring_log: false,
            collide_radius: 0.0,
            collide_k: 0.0,
        }
    }
}

impl LayoutParams {
    /// A ForceAtlas2-shaped candidate for the wiki: degree-weighted `1/r`
    /// repulsion, a linear anchor per unit mass, plain springs, and a
    /// no-overlap disc the size of a mark.
    ///
    /// Why these, against the two defects of the lifted law on the live wiki:
    ///
    /// * **The ring.** Under `1/r²` repulsion a node outside the graph sees it
    ///   as one central charge, and a degree-zero node has nothing but the
    ///   linear anchor to hold it, so every unlinked note settles on one shell
    ///   at `r³ = repulsion · n / anchor_k`. Under `1/r` repulsion the field
    ///   inside a uniform disc of mass grows linearly with radius (Gauss's law
    ///   in two dimensions), which is exactly the shape of a linear anchor, so
    ///   the balance holds at EVERY radius inside the disc, not on one shell:
    ///   the equilibrium is a disc of uniform mass density
    ///   `anchor_k / (pi · repulsion)`, and an unlinked note is just one more
    ///   unit of mass filling it. Weighting both terms by the same mass is what
    ///   makes that true for every node at once.
    /// * **The overlap.** Uniform density spreads the core, and the collision
    ///   disc enforces a floor where springs would otherwise pull a cluster
    ///   tighter than its marks.
    ///
    /// Scale: area per unit mass is `pi · repulsion / anchor_k`, here 1608
    /// world units², a 40-unit spacing for a bare node, and a leaf pair rests
    /// at `sqrt(4 · repulsion / attraction)`, also 40, so edges and gaps read
    /// at the same size.
    pub fn atlas() -> Self {
        LayoutParams {
            repulsion: 10.24,
            attraction: 0.0256,
            anchor_k: [0.02, 0.02],
            repulsion_falloff: 1.0,
            degree_mass: true,
            spring_degree_norm: false,
            spring_log: false,
            collide_radius: super::marks::MARK,
            collide_k: 0.5,
            ..LayoutParams::default()
        }
    }
}

/// Where the anchor spring pulls each node, and how hard, per axis.
///
/// One term, three uses. The wiki leaves it unset and gets its gravity well at
/// the origin. The mesh points it at a ring so a handful of peers still read as
/// equals. The lattice pins `x` stiffly to the node's rank and leaves `y`
/// slack, so rank survives while a cross-lane spring is free to align
/// correspondents vertically.
#[derive(Clone, Debug, Default)]
pub struct Anchors {
    pub target: Vec<[f32; 2]>,
    pub k: Vec<[f32; 2]>,
}

impl Anchors {
    /// Anchors with uniform stiffness, one target per node.
    pub fn uniform(target: Vec<[f32; 2]>, k: [f32; 2]) -> Self {
        let k = vec![k; target.len()];
        Anchors { target, k }
    }
}

/// What the layout did, reported per step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutStats {
    pub steps: u64,
    pub kinetic_energy: f32,
    pub mean_speed: f32,
    pub max_speed: f32,
    /// `min_x, min_y, max_x, max_y`. Drives [`super::GraphViewport::fit`].
    pub bounds: [f32; 4],
    /// Nothing is moving fast enough to be worth repainting for.
    ///
    /// This is a **repaint gate, not a convergence claim**. Heat decays toward
    /// [`LayoutParams::heat_floor`], so a layout can go quiet while still
    /// creeping. A caller that needs "has this settled" must ask the physics
    /// with `cool = 1.0`, which is what the settling test does.
    pub quiet: bool,
}

impl Default for LayoutStats {
    fn default() -> Self {
        LayoutStats {
            steps: 0,
            kinetic_energy: 0.0,
            mean_speed: 0.0,
            max_speed: 0.0,
            bounds: [0.0; 4],
            quiet: false,
        }
    }
}

/// What [`ForceLayout::retarget`] changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetargetReport {
    pub added: usize,
    pub removed: usize,
    /// Nodes granted heat because the change reached them.
    pub kicked: usize,
}

/// Positions for `n` nodes under the lifted force law.
///
/// The seam is deliberately narrow: **this type owns positions by index, and
/// the view owns everything else at the same index.** There is no
/// `ForceLayout<T>`, no node payload, no key type — a view keeps its own
/// index-parallel arrays and reads [`ForceLayout::positions`] beside them. That
/// is what lets one layout serve three views whose node types have nothing in
/// common.
#[derive(Clone, Debug)]
pub struct ForceLayout {
    params: LayoutParams,
    pos: Vec<[f32; 2]>,
    vel: Vec<[f32; 2]>,
    heat: Vec<f32>,
    csr: Csr,
    anchors: Option<Anchors>,
    anchor_mean: [f32; 2],
    tree: QuadTree,
    stack: Vec<u32>,
    stats: LayoutStats,
}

impl ForceLayout {
    /// A layout over `nodes` nodes joined by `edges`, seeded on a ring of
    /// radius `seed_scale * sqrt(n)`.
    ///
    /// Edge endpoints outside `nodes` are skipped, and so are self-edges: the
    /// records a view draws come from an open world and may name what the view
    /// did not include.
    pub fn new(nodes: usize, edges: &[(u32, u32)], params: LayoutParams) -> Self {
        let mut layout = ForceLayout {
            params,
            pos: vec![[0.0, 0.0]; nodes],
            vel: vec![[0.0, 0.0]; nodes],
            heat: vec![1.0; nodes],
            csr: Csr::build(nodes, edges),
            anchors: None,
            anchor_mean: [0.0, 0.0],
            tree: QuadTree::default(),
            stack: Vec::new(),
            stats: LayoutStats::default(),
        };
        let radius = params.seed_scale * (nodes as f32).sqrt();
        layout.seed_ring(radius);
        layout
    }

    /// Place every node on a ring of `radius`, at rest.
    ///
    /// Seeding is not cosmetic. The lifted viewer opened on a ring of
    /// `200 + 5n`, which grows linearly in a layout whose settled area grows
    /// with the square root, so at a few thousand nodes it opened as a vast
    /// thin circle that the `max_force` cap — an absolute number, independent
    /// of `n` — could not close in any reasonable number of steps.
    pub fn seed_ring(&mut self, radius: f32) {
        let count = self.pos.len();
        let turns = count.max(1) as f32;
        for (index, point) in self.pos.iter_mut().enumerate() {
            let angle = index as f32 / turns * std::f32::consts::TAU;
            *point = [angle.cos() * radius, angle.sin() * radius];
        }
        self.vel.iter_mut().for_each(|v| *v = [0.0, 0.0]);
    }

    /// Place node `index` explicitly, at rest.
    pub fn seed(&mut self, index: usize, at: [f32; 2]) {
        if let Some(point) = self.pos.get_mut(index) {
            *point = at;
            self.vel[index] = [0.0, 0.0];
        }
    }

    /// Point the anchor spring somewhere other than the origin.
    ///
    /// `None` restores the lifted behaviour: every node pulled toward the world
    /// origin at [`LayoutParams::anchor_k`]. An `Anchors` shorter than the node
    /// count leaves the remaining nodes anchored at the origin, which is the
    /// open-world reading — a view that knows where some nodes belong should
    /// not have to invent a place for the rest.
    pub fn anchors(&mut self, anchors: Option<&Anchors>) {
        self.anchors = anchors.cloned();
        self.anchor_mean = match &self.anchors {
            None => [0.0, 0.0],
            Some(anchors) => {
                let count = self.pos.len().max(1) as f32;
                let mut sum = [0.0f32; 2];
                for index in 0..self.pos.len() {
                    let target = anchors.target.get(index).copied().unwrap_or([0.0, 0.0]);
                    sum[0] += target[0];
                    sum[1] += target[1];
                }
                [sum[0] / count, sum[1] / count]
            }
        };
    }

    pub fn params(&self) -> &LayoutParams {
        &self.params
    }

    pub fn set_params(&mut self, params: LayoutParams) {
        self.params = params;
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    pub fn positions(&self) -> &[[f32; 2]] {
        &self.pos
    }

    pub fn stats(&self) -> LayoutStats {
        self.stats
    }

    /// Incident edge count plus one, as the attraction weight uses it.
    pub fn degree(&self, node: usize) -> f32 {
        self.csr.degree(node)
    }

    pub fn edge_count(&self) -> usize {
        self.csr.edge_count()
    }

    /// Neighbours of `node`, each undirected edge appearing once per endpoint.
    pub fn neighbours(&self, node: usize) -> &[u32] {
        self.csr.neighbours(node)
    }

    /// Replace the graph in place, carrying what survived.
    ///
    /// `carry[new] = Some(old)` keeps that node's position **and velocity**, so
    /// a graph that gained one node does not teleport back to a circle and pay
    /// the whole settle again — which is what the lifted viewer did on every
    /// dataset revision, meaning one write by any peer reset the picture.
    ///
    /// The caller owns identity. This type never hashes, compares or stores a
    /// key: which new node is which old node is a question about the view's
    /// own vocabulary, and a derived id is opaque.
    pub fn retarget(
        &mut self,
        nodes: usize,
        edges: &[(u32, u32)],
        carry: &[Option<u32>],
    ) -> RetargetReport {
        let previous = self.pos.len();
        let old_pos = std::mem::take(&mut self.pos);
        let old_vel = std::mem::take(&mut self.vel);
        let old_heat = std::mem::take(&mut self.heat);

        self.csr = Csr::build(nodes, edges);
        self.pos = vec![[f32::NAN, f32::NAN]; nodes];
        self.vel = vec![[0.0, 0.0]; nodes];
        // Cold by default. A retarget that reheated everything would spend the
        // energy channel on the whole picture and say nothing about what
        // changed, which is the one thing it exists to say.
        self.heat = vec![self.params.heat_floor; nodes];

        let mut kept = 0usize;
        for index in 0..nodes {
            let Some(old) = carry.get(index).copied().flatten() else {
                continue;
            };
            let Some(point) = old_pos.get(old as usize) else {
                continue;
            };
            self.pos[index] = *point;
            self.vel[index] = old_vel[old as usize];
            self.heat[index] = old_heat[old as usize];
            kept += 1;
        }

        // A node with no carried position appears where its already-placed
        // neighbours already are, so it arrives inside the structure it belongs
        // to instead of flying in from a seed ring. With no placed neighbour it
        // falls back to the ring, which is the only honest answer.
        let fallback = self.params.seed_scale * (nodes.max(1) as f32).sqrt();
        let mut fresh = Vec::new();
        for index in 0..nodes {
            if self.pos[index][0].is_finite() {
                continue;
            }
            fresh.push(index as u32);
            let mut sum = [0.0f32; 2];
            let mut seen = 0.0f32;
            for &neighbour in self.csr.neighbours(index) {
                let point = self.pos[neighbour as usize];
                if point[0].is_finite() {
                    sum[0] += point[0];
                    sum[1] += point[1];
                    seen += 1.0;
                }
            }
            self.pos[index] = if seen > 0.0 {
                [sum[0] / seen, sum[1] / seen]
            } else {
                let angle = index as f32 / nodes.max(1) as f32 * std::f32::consts::TAU;
                [angle.cos() * fallback, angle.sin() * fallback]
            };
        }

        // Anchors are indexed by node, so a retarget invalidates them. The
        // mean is recomputed against the new count here so the centroid pin
        // stays honest until the caller sets the new ones.
        if let Some(anchors) = self.anchors.take() {
            self.anchors(Some(&anchors));
        }

        let added = nodes - kept;
        self.heat(&fresh, 2);
        let floor = self.params.heat_floor;
        let kicked = self.heat.iter().filter(|heat| **heat > floor).count();
        RetargetReport {
            added,
            removed: previous.saturating_sub(kept),
            kicked,
        }
    }

    /// Grant energy to `seeds` and, at half per hop, to their neighbourhood.
    ///
    /// Energy is a channel and it is as scarce as saturation: a uniformly
    /// boiling field says nothing, while a quiet one with a single shimmering
    /// chain says exactly where to look. Grants are raises, never
    /// replacements, so two overlapping grants do not cancel.
    pub fn heat(&mut self, seeds: &[u32], hops: u32) {
        let mut frontier: Vec<u32> = seeds
            .iter()
            .copied()
            .filter(|index| (*index as usize) < self.heat.len())
            .collect();
        let mut grant = 1.0f32;
        for index in &frontier {
            let heat = &mut self.heat[*index as usize];
            *heat = heat.max(grant);
        }
        let mut reached: Vec<bool> = vec![false; self.heat.len()];
        for index in &frontier {
            reached[*index as usize] = true;
        }
        for _ in 0..hops {
            grant *= 0.5;
            let mut next = Vec::new();
            for index in &frontier {
                for &neighbour in self.csr.neighbours(*index as usize) {
                    if reached[neighbour as usize] {
                        continue;
                    }
                    reached[neighbour as usize] = true;
                    let heat = &mut self.heat[neighbour as usize];
                    *heat = heat.max(grant);
                    next.push(neighbour);
                }
            }
            frontier = next;
        }
    }

    /// Heat of one node, for tests and for views that draw energy.
    pub fn node_heat(&self, node: usize) -> f32 {
        self.heat.get(node).copied().unwrap_or(0.0)
    }

    /// Advance one step.
    pub fn step(&mut self) -> LayoutStats {
        let count = self.pos.len();
        if count == 0 {
            return self.stats;
        }
        let params = self.params;
        let weights = params.degree_mass.then(|| self.csr.degrees());
        self.tree.build(&self.pos, weights);
        let mut collide_stack = Vec::new();

        // Two passes, because the lifted kernel is a Jacobi step: every node
        // read the SAME position array and wrote to a separate output. Folding
        // the integration into the force loop would let node `i + 1` see node
        // `i` already moved, which is a different (and differently-converging)
        // iteration wearing the same constants. This cost a real hour: the
        // migration test failed at step zero by a thousandth, which is far too
        // much for rounding and exactly right for half a graph being a step
        // ahead of the other half.
        let mut forces = Vec::with_capacity(count);
        for index in 0..count {
            let point = self.pos[index];

            let mut force = self.tree.repulsion(
                index,
                &self.pos,
                params.repulsion,
                params.theta,
                params.repulsion_falloff,
                weights,
                &mut self.stack,
            );

            // Attraction, symmetric in the two endpoints. Identical to the
            // lifted kernel's sum; found by lookup instead of by scanning
            // every edge for the ones that touch this node.
            let degree = self.csr.degree(index);
            // With `degree_mass` off, `local` IS the force and the sums land in
            // the same order the lifted kernel added them.
            let mut local = if params.degree_mass {
                [0.0f32; 2]
            } else {
                force
            };
            for &other in self.csr.neighbours(index) {
                let weight = if params.spring_degree_norm {
                    params.attraction / (degree * self.csr.degree(other as usize)).sqrt()
                } else {
                    params.attraction
                };
                let target = self.pos[other as usize];
                let (dx, dy) = (target[0] - point[0], target[1] - point[1]);
                let weight = if params.spring_log {
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > 0.0 {
                        weight * dist.ln_1p() / dist
                    } else {
                        0.0
                    }
                } else {
                    weight
                };
                local[0] += dx * weight;
                local[1] += dy * weight;
            }
            if params.degree_mass {
                // Repulsion from the tree already acts per unit of this
                // node's mass (it sums the OTHER nodes' masses only); the
                // springs are forces, divided here.
                force[0] += local[0] / degree;
                force[1] += local[1] / degree;
            } else {
                force = local;
            }
            if params.collide_radius > 0.0 {
                // Not divided by mass: the disc is a geometric floor, not a
                // physical force, and dividing would leave a hub's disc soft
                // exactly where its springs pull hardest.
                let push = self.tree.collision(
                    index,
                    &self.pos,
                    self.csr.degrees(),
                    params.collide_radius,
                    params.collide_k,
                    &mut collide_stack,
                );
                force[0] += push[0];
                force[1] += push[1];
            }

            let (target, stiffness) = match &self.anchors {
                None => ([0.0, 0.0], params.anchor_k),
                Some(anchors) => (
                    anchors.target.get(index).copied().unwrap_or([0.0, 0.0]),
                    anchors.k.get(index).copied().unwrap_or(params.anchor_k),
                ),
            };
            force[0] -= (point[0] - target[0]) * stiffness[0];
            force[1] -= (point[1] - target[1]) * stiffness[1];

            // Heat scales the *summed* force, so it changes the rate of
            // approach and can never move the equilibrium: at rest the force
            // is zero and any multiple of zero is zero.
            let heat = self.heat[index];
            force[0] *= heat;
            force[1] *= heat;

            let magnitude = (force[0] * force[0] + force[1] * force[1]).sqrt();
            if magnitude > params.max_force {
                let scale = params.max_force / magnitude;
                force[0] *= scale;
                force[1] *= scale;
            }

            forces.push(force);
        }

        let previous = self.pos.clone();
        for index in 0..count {
            let force = forces[index];
            let velocity = [
                (self.vel[index][0] + force[0]) * params.damping,
                (self.vel[index][1] + force[1]) * params.damping,
            ];
            self.vel[index] = velocity;
            self.pos[index] = [
                previous[index][0] + velocity[0],
                previous[index][1] + velocity[1],
            ];
        }

        self.correct(&previous);
        self.cool_and_measure()
    }

    /// The two momentum corrections, lifted verbatim in substance.
    ///
    /// Position: translate so the centroid sits at the anchors' mean. A pure
    /// translation is norm-preserving and never distorts the layout, and with
    /// no anchors the mean is the origin — exactly what the lifted viewer
    /// pinned to.
    ///
    /// Velocity: remove the net linear and net angular components. Both are
    /// no-ops at rest and only bleed off global drift and spin left by initial
    /// conditions or by the far-field approximation, which is not exactly
    /// equal-and-opposite the way the all-pairs sum is.
    ///
    /// Positions are deliberately *not* sheared by `omega x r`. That was a
    /// small-angle approximation of a rotation, not norm-preserving, and it
    /// wrote into the baseline for the next step's velocity estimate, closing a
    /// loop that could sustain the very rotation it meant to remove.
    fn correct(&mut self, previous: &[[f32; 2]]) {
        let count = self.pos.len();
        let mut centre = [0.0f32; 2];
        for point in &self.pos {
            centre[0] += point[0];
            centre[1] += point[1];
        }
        centre[0] /= count as f32;
        centre[1] /= count as f32;

        let mut angular = 0.0f32;
        let mut inertia = 0.0f32;
        for index in 0..count {
            let dx = self.pos[index][0] - centre[0];
            let dy = self.pos[index][1] - centre[1];
            let vx = self.pos[index][0] - previous[index][0];
            let vy = self.pos[index][1] - previous[index][1];
            angular += dx * vy - dy * vx;
            inertia += dx * dx + dy * dy;
        }
        let omega = if inertia > 1.0 {
            angular / inertia
        } else {
            0.0
        };

        let mut mean = [0.0f32; 2];
        for velocity in &self.vel {
            mean[0] += velocity[0];
            mean[1] += velocity[1];
        }
        mean[0] /= count as f32;
        mean[1] /= count as f32;

        for index in 0..count {
            let dx = self.pos[index][0] - centre[0];
            let dy = self.pos[index][1] - centre[1];
            self.pos[index] = [dx + self.anchor_mean[0], dy + self.anchor_mean[1]];
            self.vel[index] = [
                self.vel[index][0] - mean[0] + omega * dy,
                self.vel[index][1] - mean[1] - omega * dx,
            ];
        }
    }

    /// Cool the nodes that have stopped needing the energy, and report.
    ///
    /// Cooling is gated on the node's own speed, and that gate is the whole
    /// design rather than a refinement of it. An unconditional schedule reaches
    /// the floor in a few hundred steps whatever the layout is doing, so a
    /// layout that has not finished arriving is frozen where it stands —
    /// measured at 3382 nodes: seeded on the old ring of 17 110 world units it
    /// sat at a settled radius of 16 625, having barely closed at all, while
    /// the same law with the schedule off reached 10 479 and was still
    /// contracting toward about 8 400. The energies of the two schedules looked
    /// almost identical (704 against 683), which is exactly the trap: they
    /// matched because BOTH had frozen, and the radius was the number that
    /// said so.
    ///
    /// Gated on speed, a node keeps its energy for as long as it is using it
    /// and gives it up when it stops. A field that has settled goes quiet; a
    /// neighbourhood that was just granted heat keeps it until it arrives. The
    /// failure mode also inverts, which is the better half of the trade: an
    /// ungated schedule fails INVISIBLY, leaving a half-settled layout that
    /// looks finished, while this one fails VISIBLY, by continuing to move.
    fn cool_and_measure(&mut self) -> LayoutStats {
        let params = self.params;
        let mut energy = 0.0f32;
        let mut speed_sum = 0.0f32;
        let mut max_speed: f32 = 0.0;
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for index in 0..self.pos.len() {
            let velocity = self.vel[index];
            let speed_sq = velocity[0] * velocity[0] + velocity[1] * velocity[1];
            energy += speed_sq;
            let speed = speed_sq.sqrt();
            speed_sum += speed;
            max_speed = max_speed.max(speed);
            if speed < params.quiet_speed {
                let heat = &mut self.heat[index];
                *heat = (*heat * params.cool).max(params.heat_floor);
            }
            let point = self.pos[index];
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        }
        let count = self.pos.len() as f32;
        let mean_speed = speed_sum / count;
        self.stats = LayoutStats {
            steps: self.stats.steps + 1,
            kinetic_energy: 0.5 * energy,
            mean_speed,
            max_speed,
            bounds,
            quiet: mean_speed < params.quiet_speed,
        };
        self.stats
    }

    /// Step until `budget` milliseconds are spent, at least once.
    ///
    /// The widget owns the step rate, not the solver: a frame has a budget and
    /// the layout gets whatever is left of it. Capped so one enormous graph
    /// cannot run away with a frame it was only lent.
    pub fn step_budget(&mut self, budget: f32) -> LayoutStats {
        const MAX_STEPS: u32 = 64;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let start = std::time::Instant::now();
            let budget = std::time::Duration::from_secs_f32((budget / 1000.0).max(0.0));
            let mut stats = self.step();
            let mut taken = 1;
            while start.elapsed() < budget && taken < MAX_STEPS {
                stats = self.step();
                taken += 1;
            }
            stats
        }
        // `Instant::now` panics on wasm, and a graph widget is not worth a
        // clock dependency: spend a small fixed number of steps instead.
        #[cfg(target_arch = "wasm32")]
        {
            let _ = budget;
            let mut stats = self.step();
            for _ in 1..4 {
                stats = self.step();
            }
            stats
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transcription of the lifted GPU kernel, in index order.
    ///
    /// This is the oracle for the migration, and it is a transcription rather
    /// than a checked-in trajectory on purpose: a blob of numbers cannot be
    /// reviewed, and nobody can tell by reading it whether the code that
    /// produced it was the code that shipped. Every line below has a line above
    /// it in `force_step_kernel` and in `WikiGraph::step`, so the comparison is
    /// readable, and it runs on the CPU so no test needs a GPU to make it.
    fn lifted_step(pos: &mut [[f32; 2]], vel: &mut [[f32; 2]], edges: &[(u32, u32)]) {
        let repulsion = 140_000.0f32;
        let attraction = 0.15f32;
        let damping = 0.45f32;
        let max_force = 15.0f32;
        let gravity = 0.001f32;

        let count = pos.len();
        let mut degrees = vec![1.0f32; count];
        for &(a, b) in edges {
            degrees[a as usize] += 1.0;
            degrees[b as usize] += 1.0;
        }

        let previous: Vec<[f32; 2]> = pos.to_vec();
        let mut next = vec![[0.0f32; 2]; count];
        let mut velocities = vec![[0.0f32; 2]; count];
        for i in 0..count {
            let (px, py) = (pos[i][0], pos[i][1]);
            let mut fx = 0.0f32;
            let mut fy = 0.0f32;
            for j in 0..count {
                if j == i {
                    continue;
                }
                let dx = px - pos[j][0];
                let dy = py - pos[j][1];
                let dist_sq = (dx * dx + dy * dy).max(1.0);
                let dist = dist_sq.sqrt().max(0.001);
                let f = repulsion / dist_sq;
                fx += (dx / dist) * f;
                fy += (dy / dist) * f;
            }
            let deg_i = degrees[i];
            for &(ea, eb) in edges {
                if ea as usize == i {
                    let w = attraction / (deg_i * degrees[eb as usize]).sqrt();
                    fx += (pos[eb as usize][0] - px) * w;
                    fy += (pos[eb as usize][1] - py) * w;
                }
                if eb as usize == i {
                    let w = attraction / (deg_i * degrees[ea as usize]).sqrt();
                    fx += (pos[ea as usize][0] - px) * w;
                    fy += (pos[ea as usize][1] - py) * w;
                }
            }
            fx -= px * gravity;
            fy -= py * gravity;
            let magnitude = (fx * fx + fy * fy).sqrt();
            if magnitude > max_force {
                let scale = max_force / magnitude;
                fx *= scale;
                fy *= scale;
            }
            let vx = (vel[i][0] + fx) * damping;
            let vy = (vel[i][1] + fy) * damping;
            velocities[i] = [vx, vy];
            next[i] = [px + vx, py + vy];
        }

        // The CPU-side corrections from `WikiGraph::step`.
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        for point in &next {
            cx += point[0];
            cy += point[1];
        }
        cx /= count as f32;
        cy /= count as f32;
        let (mut angular, mut inertia) = (0.0f32, 0.0f32);
        for i in 0..count {
            let dx = next[i][0] - cx;
            let dy = next[i][1] - cy;
            let vx = next[i][0] - previous[i][0];
            let vy = next[i][1] - previous[i][1];
            angular += dx * vy - dy * vx;
            inertia += dx * dx + dy * dy;
        }
        let omega = if inertia > 1.0 {
            angular / inertia
        } else {
            0.0
        };
        let (mut mvx, mut mvy) = (0.0f32, 0.0f32);
        for velocity in &velocities {
            mvx += velocity[0];
            mvy += velocity[1];
        }
        mvx /= count as f32;
        mvy /= count as f32;
        for i in 0..count {
            let dx = next[i][0] - cx;
            let dy = next[i][1] - cy;
            pos[i] = [dx, dy];
            vel[i] = [
                velocities[i][0] - mvx + omega * dy,
                velocities[i][1] - mvy - omega * dx,
            ];
        }
    }

    /// The params the wiki ran under, for the identity proof: exact all-pairs,
    /// and no heat schedule, because the schedule is a later deliberate change
    /// and not part of the law being preserved.
    fn lifted_params() -> LayoutParams {
        LayoutParams {
            theta: 0.0,
            cool: 1.0,
            ..LayoutParams::default()
        }
    }

    fn ring(count: usize, radius: f32) -> Vec<[f32; 2]> {
        (0..count)
            .map(|index| {
                let angle = index as f32 / count.max(1) as f32 * std::f32::consts::TAU;
                [angle.cos() * radius, angle.sin() * radius]
            })
            .collect()
    }

    /// A `side x side` grid: structure worth revealing, and the shape these
    /// views actually carry. Measured convergence at `side = 14` (196 nodes,
    /// 364 edges), in mean speed per solver step: 500 steps 1.185, 1000 1.020,
    /// 2000 0.898, 2500 0.159, 3000 0.041, 4000 0.003.
    fn grid(side: u32) -> Vec<(u32, u32)> {
        let mut edges = Vec::new();
        for row in 0..side {
            for column in 0..side {
                let index = row * side + column;
                if column + 1 < side {
                    edges.push((index, index + 1));
                }
                if row + 1 < side {
                    edges.push((index, index + side));
                }
            }
        }
        edges
    }

    fn settle(layout: &mut ForceLayout, steps: usize) -> LayoutStats {
        let mut stats = layout.stats();
        for _ in 0..steps {
            stats = layout.step();
        }
        stats
    }

    #[test]
    fn default_params_are_the_wiki_constants() {
        // These five are the lifted law. A later re-tune is welcome to argue
        // for a change; it is not welcome to reach the existing production user
        // by accident, which is what this assertion prevents.
        let params = LayoutParams::default();
        assert_eq!(params.repulsion, 140_000.0);
        assert_eq!(params.attraction, 0.15);
        assert_eq!(params.damping, 0.45);
        assert_eq!(params.max_force, 15.0);
        assert_eq!(params.anchor_k, [0.001, 0.001]);
    }

    #[test]
    fn golden_trajectory_matches_the_pre_lift_kernel() {
        // Bit-exactness is not available and is not claimed: the tree visits
        // leaves in spatial order while the kernel summed in index order, and
        // float addition is not associative. What is claimed is that the law,
        // the integration and both corrections are the same ones.
        let edges = vec![
            (0u32, 1u32),
            (1, 2),
            (2, 3),
            (3, 0),
            (0, 5),
            (5, 7),
            (7, 11),
            (4, 9),
        ];
        let mut layout = ForceLayout::new(12, &edges, lifted_params());
        layout.seed_ring(200.0 + 12.0 * 5.0);

        let mut pos = ring(12, 200.0 + 12.0 * 5.0);
        let mut vel = vec![[0.0f32; 2]; 12];

        for step in 0..100 {
            layout.step();
            lifted_step(&mut pos, &mut vel, &edges);
            for index in 0..12 {
                let ours = layout.positions()[index];
                let theirs = pos[index];
                let error = (ours[0] - theirs[0]).hypot(ours[1] - theirs[1]);
                let scale = theirs[0].hypot(theirs[1]).max(1.0);
                assert!(
                    error / scale < 1e-3,
                    "step {step} node {index}: {ours:?} vs {theirs:?}"
                );
            }
        }
    }

    #[test]
    fn theta_zero_matches_all_pairs() {
        // The far-field approximation is the one substantive change to the
        // force law, so the code path that proves the migration is also the
        // code path that scales: `theta = 0` is exact all-pairs.
        let count = 200;
        let mut exact = ForceLayout::new(count, &[], lifted_params());
        let mut approximate = ForceLayout::new(count, &[], LayoutParams::default());
        approximate.set_params(LayoutParams {
            cool: 1.0,
            ..LayoutParams::default()
        });
        for index in 0..count {
            let angle = index as f32 * 2.399963;
            let radius = 30.0 * (index as f32).sqrt();
            let at = [angle.cos() * radius, angle.sin() * radius];
            exact.seed(index, at);
            approximate.seed(index, at);
        }
        exact.step();
        approximate.step();

        let mut worst = 0.0f32;
        for index in 0..count {
            let a = exact.positions()[index];
            let b = approximate.positions()[index];
            worst = worst.max((a[0] - b[0]).hypot(a[1] - b[1]));
        }
        // theta = 0.7 is an approximation and is allowed to differ; what must
        // hold is that it differs by a fraction of a mark, not a screenful.
        assert!(worst < 2.0, "far field diverged by {worst}");
    }

    #[test]
    fn energy_decreases_from_a_settled_start() {
        // Deliberately NOT per-step monotone. A layout released from a seed
        // ring accelerates first, and a monotone assertion here would be a test
        // that fails for being right about the physics.
        let edges: Vec<(u32, u32)> = (0..99u32).map(|index| (index, index + 1)).collect();
        let mut layout = ForceLayout::new(100, &edges, lifted_params());
        let start = settle(&mut layout, 1_500);
        let end = settle(&mut layout, 200);
        assert!(
            end.kinetic_energy < start.kinetic_energy,
            "energy rose from {} to {}",
            start.kinetic_energy,
            end.kinetic_energy
        );
    }

    #[test]
    fn settles_within_a_step_budget() {
        // Run with `cool = 1.0`, so this measures the physics rather than the
        // heat schedule. A decaying schedule would make this pass by freezing,
        // which would be a test that lies about the one thing it checks.
        //
        // Settling is judged on the MEAN speed, not the max: a single node can
        // spike long after the field is at rest, and a max-based assertion
        // would never fire.
        //
        // The budget is a measurement, not a guess. On a 196-node grid the mean
        // speed passes below 0.5 between step 2000 and 2500 and reaches 0.003
        // by step 4000; a 200-node path settles by step 1000; a 200-node
        // circulant with long chords is far harder and needs about 5000. The
        // number below is the grid's, with headroom.
        let mut layout = ForceLayout::new(196, &grid(14), lifted_params());
        let stats = settle(&mut layout, 3_000);
        assert!(
            stats.mean_speed < layout.params().quiet_speed,
            "still moving at {} after 3000 steps",
            stats.mean_speed
        );
    }

    #[test]
    fn no_overlap_beyond_tolerance() {
        // The 5th percentile, not the minimum. A settled layout was measured
        // with a closest pair around ten world units against a mark reaching
        // eighteen screen points, and an open world can hand you two nodes
        // identical in everything that places them.
        let edges: Vec<(u32, u32)> = (0..119u32)
            .map(|index| (index, (index + 1) % 120))
            .collect();
        let mut layout = ForceLayout::new(120, &edges, lifted_params());
        settle(&mut layout, 1_500);

        let positions = layout.positions();
        let mut distances = Vec::new();
        for a in 0..positions.len() {
            for b in (a + 1)..positions.len() {
                distances.push(
                    (positions[a][0] - positions[b][0]).hypot(positions[a][1] - positions[b][1]),
                );
            }
        }
        distances.sort_by(f32::total_cmp);
        let fifth = distances[distances.len() / 20];
        assert!(
            fifth > 2.0 * super::super::marks::MARK,
            "5th percentile pair distance {fifth}"
        );
    }

    #[test]
    fn coincident_nodes_separate() {
        // Written red first, and it failed against the lifted kernel: at
        // dx = dy = 0 the law computes `(0.0 / 1.0) * 140000`, which is zero
        // force, forever. A ring seed hides it because no two nodes share a
        // point. Seeding a lattice by rank does not.
        let mut layout = ForceLayout::new(2, &[], lifted_params());
        layout.seed(0, [40.0, -15.0]);
        layout.seed(1, [40.0, -15.0]);
        settle(&mut layout, 100);
        let a = layout.positions()[0];
        let b = layout.positions()[1];
        assert!(
            (a[0] - b[0]).hypot(a[1] - b[1]) > 1.0,
            "coincident nodes stayed welded at {a:?} / {b:?}"
        );
    }

    #[test]
    fn no_nan_ever() {
        // Every pathology at once: coincident nodes, a self-edge, an isolated
        // node, and an edge naming a node outside the graph. The last is the
        // open-world case and must be SKIPPED, never a panic.
        let edges = vec![(0u32, 0u32), (1, 2), (2, 1), (3, 99), (0, 1)];
        let mut layout = ForceLayout::new(5, &edges, lifted_params());
        for index in 0..4 {
            layout.seed(index, [0.0, 0.0]);
        }
        layout.seed(4, [1e6, -1e6]);
        for step in 0..500 {
            layout.step();
            for (index, point) in layout.positions().iter().enumerate() {
                assert!(
                    point[0].is_finite() && point[1].is_finite(),
                    "step {step} node {index} went non-finite: {point:?}"
                );
            }
        }
    }

    #[test]
    fn momentum_is_conserved() {
        // This is the test that guards the Barnes-Hut far field, whose monopole
        // sum is NOT exactly equal and opposite the way the all-pairs sum is.
        // The centroid is pinned to the anchors' mean, so with anchors at the
        // origin it must stay at the origin however the field approximates.
        let edges: Vec<(u32, u32)> = (0..299u32)
            .map(|index| (index, (index * 13 + 5) % 300))
            .collect();
        let mut layout = ForceLayout::new(300, &edges, LayoutParams::default());
        for _ in 0..500 {
            layout.step();
        }
        let positions = layout.positions();
        let mut centre = [0.0f32; 2];
        for point in positions {
            centre[0] += point[0];
            centre[1] += point[1];
        }
        centre[0] /= positions.len() as f32;
        centre[1] /= positions.len() as f32;
        assert!(
            centre[0].abs() < 1e-2 && centre[1].abs() < 1e-2,
            "centroid drifted to {centre:?}"
        );
    }

    #[test]
    fn momentum_is_conserved_around_offset_anchors() {
        // Generalised: with anchors away from the origin the centroid must sit
        // at the anchors' mean, not the origin. A pin hard-coded to zero would
        // fight the lattice's lanes every step.
        let target: Vec<[f32; 2]> = (0..64).map(|index| [400.0 + index as f32, 900.0]).collect();
        let mean = [target.iter().map(|t| t[0]).sum::<f32>() / 64.0, 900.0f32];
        let mut layout = ForceLayout::new(64, &[], LayoutParams::default());
        layout.anchors(Some(&Anchors::uniform(target, [0.02, 0.02])));
        for _ in 0..400 {
            layout.step();
        }
        let positions = layout.positions();
        let mut centre = [0.0f32; 2];
        for point in positions {
            centre[0] += point[0];
            centre[1] += point[1];
        }
        centre[0] /= positions.len() as f32;
        centre[1] /= positions.len() as f32;
        assert!(
            (centre[0] - mean[0]).abs() < 1.0,
            "x centroid {} vs {}",
            centre[0],
            mean[0]
        );
        assert!(
            (centre[1] - mean[1]).abs() < 1.0,
            "y centroid {} vs {}",
            centre[1],
            mean[1]
        );
    }

    #[test]
    fn retarget_keeps_surviving_positions() {
        let edges = vec![(0u32, 1u32), (1, 2), (2, 3)];
        let mut layout = ForceLayout::new(4, &edges, lifted_params());
        settle(&mut layout, 300);
        let before: Vec<[f32; 2]> = layout.positions().to_vec();

        // Node 4 is new; 0..3 carry across unchanged.
        let carry = vec![Some(0), Some(1), Some(2), Some(3), None];
        let report = layout.retarget(5, &[(0, 1), (1, 2), (2, 3), (3, 4)], &carry);
        assert_eq!(report.added, 1);
        assert_eq!(report.removed, 0);
        for index in 0..4 {
            let now = layout.positions()[index];
            assert_eq!(now, before[index], "carried node {index} moved on retarget");
        }
        assert!(layout.positions()[4][0].is_finite());
    }

    #[test]
    fn retarget_heats_only_the_changed_neighbourhood() {
        // JP's wiggle, as a specification: the change is visible because the
        // energy went where the change was, and the rest of the field stayed
        // quiet enough to make that legible.
        //
        // The background has to be genuinely settled first, or the contrast is
        // measured against a field that never cooled — cooling is gated on a
        // node's own speed, so a fixture that is still moving at the moment of
        // the retarget has not given up its energy and there is nothing to
        // stand out against. That is the gate working, not a flaw in it.
        let mut edges = grid(14);
        let mut layout = ForceLayout::new(196, &edges, LayoutParams::default());
        settle(&mut layout, 5_000);
        assert!(
            layout.node_heat(0) <= layout.params().heat_floor + 1e-6,
            "the field never cooled, so the contrast would measure nothing"
        );

        // Node 196 arrives attached to node 98, which is row 7, column 0.
        edges.push((98, 196));
        let mut carry: Vec<Option<u32>> = (0..196).map(|index| Some(index as u32)).collect();
        carry.push(None);
        layout.retarget(197, &edges, &carry);

        let near = [196usize, 98, 99, 84, 112];
        let far: Vec<usize> = (0..196).filter(|index| !near.contains(index)).collect();
        fn mean(nodes: &[usize], of: &dyn Fn(usize) -> f32) -> f32 {
            nodes.iter().map(|index| of(*index)).sum::<f32>() / nodes.len() as f32
        }
        let heats =
            |layout: &ForceLayout, nodes: &[usize]| mean(nodes, &|index| layout.node_heat(index));
        let energies = |layout: &ForceLayout, nodes: &[usize]| {
            mean(nodes, &|index| {
                let velocity = layout.vel[index];
                velocity[0] * velocity[0] + velocity[1] * velocity[1]
            })
        };
        // A grant halves per hop, so the MEAN over a two-hop neighbourhood is
        // diluted by construction: one node at 1.0, one at 0.5 and three at
        // 0.25 average to 0.45 against a floor of 0.05, which is ninefold. The
        // contrast that matters is in the energy below, because kinetic energy
        // goes as the square of the force and therefore as the square of this.
        let (hot, cold) = (heats(&layout, &near), heats(&layout, &far));
        assert!(hot > 5.0 * cold, "heat contrast only {hot} vs {cold}");

        layout.step();
        let (near_energy, far_energy) = (energies(&layout, &near), energies(&layout, &far));
        assert!(
            near_energy > 10.0 * far_energy,
            "kinetic energy did not concentrate: {near_energy} vs {far_energy}"
        );

        // ... and it must fade, or the field never becomes legible again.
        settle(&mut layout, 2_000);
        let (hot, cold) = (heats(&layout, &near), heats(&layout, &far));
        assert!(hot < 1.5 * cold, "heat never faded: {hot} vs {cold}");
    }

    #[test]
    fn heat_never_reaches_zero() {
        // JP's constraint, as a number. A layout that reaches exactly zero has
        // lost the channel that makes change visible, permanently.
        let mut layout = ForceLayout::new(8, &[], LayoutParams::default());
        for _ in 0..10_000 {
            layout.step();
        }
        for index in 0..8 {
            assert!(
                layout.node_heat(index) >= layout.params().heat_floor,
                "node {index} froze"
            );
        }
    }

    #[test]
    fn heat_does_not_move_the_equilibrium() {
        // Heat scales the summed force, so it changes the rate of approach and
        // nothing else. If it could move where the layout ends up, it would be
        // a distortion wearing an animation's clothes.
        //
        // Stated as a comparison rather than as a tolerance on one run, because
        // a tolerance cannot tell "heat pushed it somewhere else" apart from
        // "it had not finished arriving". Two continuations of ONE settled
        // configuration, same step count, one hot and one cooling: if heat only
        // slows the approach, the cooled run cannot have gone further.
        let mut settled = ForceLayout::new(
            196,
            &grid(14),
            LayoutParams {
                cool: 1.0,
                ..LayoutParams::default()
            },
        );
        settle(&mut settled, 4_000);
        let start: Vec<[f32; 2]> = settled.positions().to_vec();

        let mut hot = settled.clone();
        let mut cooling = settled;
        cooling.set_params(LayoutParams {
            cool: 0.985,
            ..LayoutParams::default()
        });
        settle(&mut hot, 3_000);
        settle(&mut cooling, 3_000);
        assert!(
            cooling.node_heat(0) <= cooling.params().heat_floor + 1e-6,
            "the field did not actually cool"
        );

        let drift = |layout: &ForceLayout| {
            start
                .iter()
                .enumerate()
                .map(|(index, before)| {
                    let after = layout.positions()[index];
                    (after[0] - before[0]).hypot(after[1] - before[1])
                })
                .fold(0.0f32, f32::max)
        };
        let (hot, cold) = (drift(&hot), drift(&cooling));
        assert!(
            cold <= hot + 1e-3,
            "cooling moved the layout further than running hot: {cold} vs {hot}"
        );
    }

    #[test]
    fn clusters_separate_instead_of_collapsing_into_one_blob() {
        // The silent failure a bucketed repulsion has, and the reason this
        // module summarises the far field instead of cutting it off. With a
        // distance cutoff, two clusters further apart than the cutoff exert
        // nothing on each other while the springs between them pull unopposed,
        // so everything converges to one amorphous mass. The picture still
        // looks like a graph. It just no longer means anything, and revealing
        // latent structure is the entire reason a force layout is being paid
        // for.
        const SITES: usize = 6;
        const PER_SITE: usize = 30;
        let mut edges = Vec::new();
        for site in 0..SITES {
            let base = (site * PER_SITE) as u32;
            for member in 0..PER_SITE as u32 {
                edges.push((base + member, base + (member + 1) % PER_SITE as u32));
                edges.push((base + member, base + (member + 7) % PER_SITE as u32));
            }
            // One thin link to the next site: enough to make them one graph,
            // not enough to justify drawing them on top of each other.
            edges.push((base, ((site + 1) % SITES * PER_SITE) as u32));
        }
        let count = SITES * PER_SITE;
        let mut layout = ForceLayout::new(count, &edges, lifted_params());
        settle(&mut layout, 3_000);

        let centroid = |site: usize| {
            let mut centre = [0.0f32; 2];
            for member in 0..PER_SITE {
                let point = layout.positions()[site * PER_SITE + member];
                centre[0] += point[0];
                centre[1] += point[1];
            }
            [centre[0] / PER_SITE as f32, centre[1] / PER_SITE as f32]
        };
        let radius = |site: usize| {
            let centre = centroid(site);
            (0..PER_SITE)
                .map(|member| {
                    let point = layout.positions()[site * PER_SITE + member];
                    (point[0] - centre[0]).hypot(point[1] - centre[1])
                })
                .sum::<f32>()
                / PER_SITE as f32
        };

        let spread = (0..SITES).map(radius).sum::<f32>() / SITES as f32;
        let mut closest = f32::INFINITY;
        for site in 0..SITES {
            for other in (site + 1)..SITES {
                let (a, b) = (centroid(site), centroid(other));
                closest = closest.min((a[0] - b[0]).hypot(a[1] - b[1]));
            }
        }
        assert!(
            closest > spread,
            "sites collapsed into one blob: closest pair of centres {closest}, \
             mean site radius {spread}"
        );
    }

    #[test]
    fn an_isolated_node_does_not_escape() {
        // A degree-zero node has no spring holding it in. The anchor term is
        // the only thing between it and the horizon, which is why the wiki's
        // gravity became a per-node anchor rather than being dropped.
        let mut layout = ForceLayout::new(40, &[], LayoutParams::default());
        settle(&mut layout, 4_000);
        let escaped = layout
            .positions()
            .iter()
            .map(|point| point[0].hypot(point[1]))
            .fold(0.0f32, f32::max);
        assert!(
            escaped < 20_000.0,
            "a node reached {escaped} from the anchor"
        );
    }

    #[test]
    fn candidate_fields_are_off_at_the_default() {
        // The candidate law rides on new fields. Each one's default must be
        // the lifted law, or `default()` would stop being the wiki's law
        // without the pinned five noticing.
        let params = LayoutParams::default();
        assert_eq!(params.repulsion_falloff, 2.0);
        assert!(!params.degree_mass);
        assert!(params.spring_degree_norm);
        assert!(!params.spring_log);
        assert_eq!(params.collide_radius, 0.0);
    }

    /// Radius from the centroid of every node, and the 90th percentile of the
    /// linked ones.
    fn radii(layout: &ForceLayout) -> (Vec<f32>, f32) {
        let points = layout.positions();
        let n = points.len() as f32;
        let cx = points.iter().map(|p| p[0]).sum::<f32>() / n;
        let cy = points.iter().map(|p| p[1]).sum::<f32>() / n;
        let radii: Vec<f32> = points
            .iter()
            .map(|p| (p[0] - cx).hypot(p[1] - cy))
            .collect();
        let mut linked: Vec<f32> = (0..points.len())
            .filter(|i| layout.degree(*i) > 1.0)
            .map(|i| radii[i])
            .collect();
        linked.sort_by(f32::total_cmp);
        let p90 = linked[linked.len() * 9 / 10];
        (radii, p90)
    }

    #[test]
    fn atlas_mixes_unlinked_nodes_into_the_disc() {
        // The wiki's ring, reduced: 200 linked nodes and 100 unlinked. Under
        // the lifted law every unlinked node settles on one shell outside the
        // linked ones; under the candidate, Gauss's law in two dimensions
        // makes the equilibrium a uniform disc, so most of them sit inside.
        let edges = (0..200u32)
            .flat_map(|i| [(i, (i + 1) % 200), (i, (i * 7 + 3) % 200)])
            .collect::<Vec<_>>();
        let beyond = |params: LayoutParams| {
            let mut layout = ForceLayout::new(300, &edges, params);
            settle(&mut layout, 4_000);
            let (radii, p90) = radii(&layout);
            (200..300).filter(|i| radii[*i] > p90).count()
        };
        let lifted = beyond(lifted_params());
        let atlas = beyond(LayoutParams {
            cool: 1.0,
            ..LayoutParams::atlas()
        });
        assert!(lifted > 90, "the lifted law should ring: {lifted}/100");
        assert!(atlas < 40, "the candidate still rings: {atlas}/100");
    }

    #[test]
    fn weighted_far_field_matches_weighted_all_pairs() {
        // Barnes-Hut with degree mass and 1/r: at theta = 0 the tree must be
        // the exact weighted all-pairs sum.
        let count = 120;
        let pos: Vec<[f32; 2]> = (0..count)
            .map(|i| {
                let angle = i as f32 * 2.399963;
                let radius = 25.0 * (i as f32).sqrt();
                [angle.cos() * radius, angle.sin() * radius]
            })
            .collect();
        let weights: Vec<f32> = (0..count).map(|i| 1.0 + (i % 7) as f32).collect();
        let mut tree = QuadTree::default();
        tree.build(&pos, Some(&weights));
        let mut stack = Vec::new();
        for node in [0usize, 17, 64, 119] {
            let ours = tree.repulsion(node, &pos, 3.0, 0.0, 1.0, Some(&weights), &mut stack);
            let mut exact = [0.0f32; 2];
            for other in 0..count {
                if other == node {
                    continue;
                }
                let dx = pos[node][0] - pos[other][0];
                let dy = pos[node][1] - pos[other][1];
                let dist = (dx * dx + dy * dy).max(1.0).sqrt();
                let magnitude = 3.0 / dist * weights[other];
                exact[0] += dx / dist * magnitude;
                exact[1] += dy / dist * magnitude;
            }
            let error = (ours[0] - exact[0]).hypot(ours[1] - exact[1]);
            assert!(
                error < 1e-3 * exact[0].hypot(exact[1]).max(1.0),
                "node {node}: {ours:?} vs {exact:?}"
            );
            let approx = tree.repulsion(node, &pos, 3.0, 0.7, 1.0, Some(&weights), &mut stack);
            let error = (approx[0] - exact[0]).hypot(approx[1] - exact[1]);
            assert!(
                error < 0.05 * exact[0].hypot(exact[1]).max(1.0),
                "node {node} far field off by {error}"
            );
        }
    }

    #[test]
    fn collision_keeps_a_tight_cluster_apart() {
        // A complete graph of 30 pulls itself into a knot tighter than its
        // marks; the collision disc is what holds them a mark apart.
        let mut edges = Vec::new();
        for a in 0..30u32 {
            for b in (a + 1)..30 {
                edges.push((a, b));
            }
        }
        let closest = |params: LayoutParams| {
            let mut layout = ForceLayout::new(30, &edges, params);
            settle(&mut layout, 3_000);
            let points = layout.positions();
            let mut closest = f32::INFINITY;
            for a in 0..30 {
                for b in (a + 1)..30 {
                    closest = closest
                        .min((points[a][0] - points[b][0]).hypot(points[a][1] - points[b][1]));
                }
            }
            closest
        };
        // Repulsion all but off and springs firm, so without the disc the
        // 30 settle a few units apart.
        let base = LayoutParams {
            cool: 1.0,
            repulsion: 0.001,
            attraction: 0.02,
            ..LayoutParams::atlas()
        };
        let loose = closest(LayoutParams {
            collide_radius: 0.0,
            ..base
        });
        let held = closest(base);
        assert!(
            loose < 2.0 * super::super::marks::MARK,
            "fixture never overlapped: {loose}"
        );
        assert!(
            held > 1.6 * super::super::marks::MARK,
            "collision let marks overlap: {held}"
        );
    }

    #[test]
    fn an_out_of_range_edge_is_skipped_not_fatal() {
        // The open-world reading, asserted rather than assumed: a record naming
        // a node the view did not include is data the view cannot decode, and
        // skipping it is the correct answer.
        let layout = ForceLayout::new(3, &[(0, 1), (1, 77), (5, 6)], LayoutParams::default());
        assert_eq!(layout.edge_count(), 1);
    }

    #[test]
    fn seeding_by_area_beats_seeding_by_count() {
        // The lifted viewer seeded a ring of `200 + 5n`, which grows linearly
        // while a settled layout's radius grows with the square root, so the
        // opening ring was far larger than the answer. `max_force` is an
        // absolute cap independent of `n`, so at a few thousand nodes every
        // node is clamped for hundreds of steps and the ring closes at a fixed
        // crawl. Seeding by area is what makes the settle finish.
        let count = 600;
        let edges: Vec<(u32, u32)> = (0..count as u32 - 1).map(|i| (i, i + 1)).collect();
        let mut by_count = ForceLayout::new(count, &edges, lifted_params());
        by_count.seed_ring(200.0 + count as f32 * 5.0);
        let mut by_area = ForceLayout::new(count, &edges, lifted_params());

        let counted = settle(&mut by_count, 600);
        let area = settle(&mut by_area, 600);
        assert!(
            area.kinetic_energy < counted.kinetic_energy,
            "area seed {} did not beat count seed {}",
            area.kinetic_energy,
            counted.kinetic_energy
        );
    }
}
