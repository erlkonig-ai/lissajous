//! The two index structures the force law needs, and nothing else.
//!
//! Both exist for the same reason: the lifted kernel computed every force by
//! scanning everything. Repulsion scanned all `n` other nodes; attraction
//! scanned all `e` edges looking for the ones touching `i`. That is `n² + n·e`
//! per step, which is fine at the few hundred nodes the wiki had and is
//! `1.6e9` operations per step at the sizes this module now has to reach.
//!
//! Neither structure changes the force being computed. [`Csr`] is a
//! reindexing of the same edge sum — every term that was found by scanning is
//! still there, found by lookup instead. [`QuadTree`] is exact when `theta` is
//! zero (the acceptance test can never fire, so every leaf is visited) and
//! approximates only the *far* field above it.

/// Adjacency in compressed-sparse-row form, plus the degree vector the
/// symmetric attraction weight needs.
///
/// Edges are stored twice — once from each endpoint — because the force is
/// symmetric and both endpoints need to find it. Out-of-range and self-edges
/// are dropped at build time rather than checked per step: a view over an open
/// world may name an endpoint it did not include, and skipping such a record is
/// the correct reading of it, not an error.
#[derive(Clone, Debug, Default)]
pub(crate) struct Csr {
    /// `offsets[i]..offsets[i + 1]` is node `i`'s slice of `targets`.
    offsets: Vec<u32>,
    targets: Vec<u32>,
    /// `1.0 + incident edge count`, matching the lifted kernel's `degrees`
    /// array. The base of one keeps an isolated node at `deg = 1` so
    /// `sqrt(deg_i * deg_j)` never divides by zero.
    degree: Vec<f32>,
}

impl Csr {
    pub(crate) fn build(nodes: usize, edges: &[(u32, u32)]) -> Self {
        let mut counts = vec![0u32; nodes];
        let keep = |&(a, b): &(u32, u32)| (a as usize) < nodes && (b as usize) < nodes && a != b;
        for &(a, b) in edges.iter().filter(|edge| keep(edge)) {
            counts[a as usize] += 1;
            counts[b as usize] += 1;
        }

        let mut offsets = Vec::with_capacity(nodes + 1);
        let mut running = 0u32;
        for count in &counts {
            offsets.push(running);
            running += count;
        }
        offsets.push(running);

        let mut cursor = offsets.clone();
        let mut targets = vec![0u32; running as usize];
        for &(a, b) in edges.iter().filter(|edge| keep(edge)) {
            targets[cursor[a as usize] as usize] = b;
            cursor[a as usize] += 1;
            targets[cursor[b as usize] as usize] = a;
            cursor[b as usize] += 1;
        }

        let degree = counts.iter().map(|count| 1.0 + *count as f32).collect();
        Csr {
            offsets,
            targets,
            degree,
        }
    }

    pub(crate) fn neighbours(&self, node: usize) -> &[u32] {
        let start = self.offsets[node] as usize;
        let end = self.offsets[node + 1] as usize;
        &self.targets[start..end]
    }

    pub(crate) fn degree(&self, node: usize) -> f32 {
        self.degree[node]
    }

    /// Every node's `1 + incident edges`, index-parallel to the positions.
    pub(crate) fn degrees(&self) -> &[f32] {
        &self.degree
    }

    /// Undirected edge count, recovered from the doubled adjacency.
    pub(crate) fn edge_count(&self) -> usize {
        self.targets.len() / 2
    }
}

/// A cell of the Barnes–Hut tree.
///
/// Leaves carry a range into the permutation array rather than a child list, so
/// a bucket of coincident points is an ordinary leaf instead of an infinite
/// subdivision. That matters: coincident points are not a pathological fixture
/// here, they are what a lattice seeded by rank produces before the first step.
#[derive(Clone, Copy, Debug)]
struct Cell {
    centre_of_mass: [f32; 2],
    /// Geometric centre of the square this cell covers, for range queries.
    centre: [f32; 2],
    /// Point count when every node repels equally, or the sum of the members'
    /// weights when the tree was built with weights.
    mass: f32,
    /// Half the side length of the square this cell covers.
    half: f32,
    children: [u32; 4],
    first: u32,
    count: u32,
    leaf: bool,
}

const NO_CHILD: u32 = u32::MAX;
/// Subdivision stops here. At depth 24 a cell is 2⁻²⁴ of the root square, which
/// is below f32's ability to tell two positions apart in any layout we draw, so
/// deeper recursion would separate nothing and could not terminate on
/// coincident input.
const MAX_DEPTH: u32 = 24;
/// A leaf holds at most this many points before subdividing.
const LEAF_CAPACITY: u32 = 8;

/// A Barnes–Hut quadtree over the current positions.
///
/// Rebuilt every step. That is deliberate: an incrementally-maintained tree
/// would need to be correct under motion, and the build is `O(n log n)` with a
/// tiny constant against a traversal that dominates it.
#[derive(Clone, Debug, Default)]
pub(crate) struct QuadTree {
    cells: Vec<Cell>,
    /// Point indices, permuted so each leaf's members are contiguous.
    order: Vec<u32>,
    scratch: Vec<u32>,
}

impl QuadTree {
    /// Build over `pos`. With `weights`, a cell's mass is the sum of its
    /// members' weights and its centre of mass is weighted by them; without,
    /// every node weighs one, which is the lifted law.
    pub(crate) fn build(&mut self, pos: &[[f32; 2]], weights: Option<&[f32]>) {
        self.cells.clear();
        self.order.clear();
        if pos.is_empty() {
            return;
        }

        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for point in pos {
            for axis in 0..2 {
                if point[axis] < min[axis] {
                    min[axis] = point[axis];
                }
                if point[axis] > max[axis] {
                    max[axis] = point[axis];
                }
            }
        }
        // A non-finite coordinate would poison the whole tree. The integrator
        // keeps positions finite, but the tree is also handed caller-seeded
        // positions, and an open world hands us what it has.
        if !min.iter().all(|v| v.is_finite()) || !max.iter().all(|v| v.is_finite()) {
            min = [0.0, 0.0];
            max = [0.0, 0.0];
        }

        let centre = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
        let half = ((max[0] - min[0]).max(max[1] - min[1]) * 0.5).max(1.0);

        self.order.extend(0..pos.len() as u32);
        self.scratch.resize(pos.len(), 0);
        self.cells.push(Cell {
            centre_of_mass: [0.0, 0.0],
            centre,
            mass: 0.0,
            half,
            children: [NO_CHILD; 4],
            first: 0,
            count: pos.len() as u32,
            leaf: true,
        });
        self.subdivide(0, centre, pos, weights, 0);
    }

    /// Split `cell` until it is small enough to be a leaf, accumulating the
    /// centre of mass on the way back up.
    fn subdivide(
        &mut self,
        cell: usize,
        centre: [f32; 2],
        pos: &[[f32; 2]],
        weights: Option<&[f32]>,
        depth: u32,
    ) {
        let first = self.cells[cell].first as usize;
        let count = self.cells[cell].count as usize;
        let half = self.cells[cell].half;

        if count as u32 <= LEAF_CAPACITY || depth >= MAX_DEPTH {
            if let Some(weights) = weights {
                let mut sum = [0.0f32; 2];
                let mut mass = 0.0f32;
                for &index in &self.order[first..first + count] {
                    let weight = weights[index as usize];
                    sum[0] += pos[index as usize][0] * weight;
                    sum[1] += pos[index as usize][1] * weight;
                    mass += weight;
                }
                let cell = &mut self.cells[cell];
                cell.leaf = true;
                cell.mass = mass;
                cell.centre_of_mass = [sum[0] / mass, sum[1] / mass];
                return;
            }
            let mut sum = [0.0f32; 2];
            for &index in &self.order[first..first + count] {
                sum[0] += pos[index as usize][0];
                sum[1] += pos[index as usize][1];
            }
            let cell = &mut self.cells[cell];
            cell.leaf = true;
            cell.mass = count as f32;
            cell.centre_of_mass = [sum[0] / count as f32, sum[1] / count as f32];
            return;
        }

        // Four-way partition of this cell's slice, quadrant order
        // (-x,-y), (+x,-y), (-x,+y), (+x,+y).
        let mut buckets = [0usize; 4];
        for &index in &self.order[first..first + count] {
            buckets[quadrant(pos[index as usize], centre)] += 1;
        }
        let mut starts = [0usize; 4];
        let mut running = 0usize;
        for quad in 0..4 {
            starts[quad] = running;
            running += buckets[quad];
        }
        let mut cursor = starts;
        for slot in 0..count {
            let index = self.order[first + slot];
            let quad = quadrant(pos[index as usize], centre);
            self.scratch[cursor[quad]] = index;
            cursor[quad] += 1;
        }
        self.order[first..first + count].copy_from_slice(&self.scratch[..count]);

        let quarter = half * 0.5;
        let mut mass = 0.0f32;
        let mut weighted = [0.0f32; 2];
        for quad in 0..4 {
            if buckets[quad] == 0 {
                continue;
            }
            let child = self.cells.len() as u32;
            let child_centre = [
                centre[0] + if quad & 1 == 0 { -quarter } else { quarter },
                centre[1] + if quad & 2 == 0 { -quarter } else { quarter },
            ];
            self.cells.push(Cell {
                centre_of_mass: [0.0, 0.0],
                centre: child_centre,
                mass: 0.0,
                half: quarter,
                children: [NO_CHILD; 4],
                first: (first + starts[quad]) as u32,
                count: buckets[quad] as u32,
                leaf: true,
            });
            self.cells[cell].children[quad] = child;
            self.subdivide(child as usize, child_centre, pos, weights, depth + 1);
            let child = &self.cells[child as usize];
            mass += child.mass;
            weighted[0] += child.centre_of_mass[0] * child.mass;
            weighted[1] += child.centre_of_mass[1] * child.mass;
        }

        let cell = &mut self.cells[cell];
        cell.leaf = false;
        cell.mass = mass;
        cell.centre_of_mass = [weighted[0] / mass, weighted[1] / mass];
    }

    /// Sum the repulsion on `node` from every other node.
    ///
    /// `theta` is the Barnes–Hut acceptance ratio: a cell is summarised by its
    /// centre of mass when its width over its distance falls below it. At
    /// `theta = 0` the test can never pass, every leaf is visited, and the
    /// result is the exact all-pairs sum the lifted kernel computed — which is
    /// how the migration is proved rather than assumed.
    ///
    /// `falloff` is the exponent `p` in `repulsion / r^p`: `2` is the lifted
    /// law, `1` is the ForceAtlas2 law, whose field inside a uniform disc grows
    /// linearly with radius (Gauss's law in two dimensions) and so balances a
    /// linear anchor everywhere at once. `weights`, when given, must be the
    /// ones the tree was built with; each other node then repels in proportion
    /// to its weight.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn repulsion(
        &self,
        node: usize,
        pos: &[[f32; 2]],
        repulsion: f32,
        theta: f32,
        falloff: f32,
        weights: Option<&[f32]>,
        stack: &mut Vec<u32>,
    ) -> [f32; 2] {
        let mut force = [0.0f32; 2];
        if self.cells.is_empty() {
            return force;
        }
        let point = pos[node];
        let theta_sq = theta * theta;

        stack.clear();
        stack.push(0);
        while let Some(index) = stack.pop() {
            let cell = &self.cells[index as usize];
            if cell.mass == 0.0 {
                continue;
            }
            if !cell.leaf {
                let dx = point[0] - cell.centre_of_mass[0];
                let dy = point[1] - cell.centre_of_mass[1];
                let dist_sq = dx * dx + dy * dy;
                let width = cell.half * 2.0;
                if width * width < theta_sq * dist_sq {
                    // Monopole. The per-pair `max(1.0)` floor of the original
                    // law is deliberately not re-applied here: a cell only
                    // qualifies when it is far, where the floor never binds.
                    let clamped = dist_sq.max(1.0);
                    let dist = clamped.sqrt().max(0.001);
                    let magnitude = law(repulsion, clamped, dist, falloff) * cell.mass;
                    force[0] += dx / dist * magnitude;
                    force[1] += dy / dist * magnitude;
                    continue;
                }
                stack.extend(cell.children.iter().copied().filter(|c| *c != NO_CHILD));
                continue;
            }
            let first = cell.first as usize;
            for &other in &self.order[first..first + cell.count as usize] {
                let other = other as usize;
                if other == node {
                    continue;
                }
                let target = pos[other];
                let mut dx = point[0] - target[0];
                let mut dy = point[1] - target[1];
                if dx == 0.0 && dy == 0.0 {
                    // Two nodes at one point. The lifted law computes
                    // `dx / dist * f` here, which is exactly zero, so they stay
                    // welded together forever. A ring seed hides this because
                    // no two nodes ever share a point; seeding a lattice by
                    // rank does not, which is how it was found.
                    //
                    // The substituted direction is a function of the unordered
                    // pair, negated for the lower index, so the two nodes push
                    // apart with equal and opposite force and the step stays
                    // momentum-conserving. It is derived from the indices
                    // rather than drawn from a generator: this makes seeding
                    // *correct*, and deliberately does not make the layout
                    // reproducible, which is not a property being promised.
                    let (low, high) = (node.min(other), node.max(other));
                    let angle = pair_angle(low as u64, high as u64);
                    let sign = if node == low { -1.0 } else { 1.0 };
                    dx = angle.cos() * sign;
                    dy = angle.sin() * sign;
                }
                let dist_sq = (dx * dx + dy * dy).max(1.0);
                let dist = dist_sq.sqrt().max(0.001);
                let mut magnitude = law(repulsion, dist_sq, dist, falloff);
                if let Some(weights) = weights {
                    magnitude *= weights[other];
                }
                force[0] += dx / dist * magnitude;
                force[1] += dy / dist * magnitude;
            }
        }
        force
    }

    /// Push `node` out of every other node's disc.
    ///
    /// Node `j`'s disc has radius `radius * drawn_scale(degrees[j])`, the same
    /// degree scaling the wiki draws its marks with, so a hub keeps the room
    /// its larger mark takes. The push is a linear spring on the overlap,
    /// `stiffness * (r_i + r_j - d)`, equal and opposite for the pair, so it
    /// conserves momentum the way the repulsion does. Exact: every cell that
    /// can hold an overlapping disc is opened, nothing is summarised.
    pub(crate) fn collision(
        &self,
        node: usize,
        pos: &[[f32; 2]],
        degrees: &[f32],
        radius: f32,
        stiffness: f32,
        stack: &mut Vec<u32>,
    ) -> [f32; 2] {
        let mut force = [0.0f32; 2];
        if self.cells.is_empty() || radius <= 0.0 {
            return force;
        }
        let point = pos[node];
        let own = radius * drawn_scale(degrees[node]);
        let reach = own + radius * DRAWN_SCALE_MAX;

        stack.clear();
        stack.push(0);
        while let Some(index) = stack.pop() {
            let cell = &self.cells[index as usize];
            if cell.mass == 0.0 {
                continue;
            }
            let span = cell.half + reach;
            if (point[0] - cell.centre[0]).abs() > span || (point[1] - cell.centre[1]).abs() > span
            {
                continue;
            }
            if !cell.leaf {
                stack.extend(cell.children.iter().copied().filter(|c| *c != NO_CHILD));
                continue;
            }
            let first = cell.first as usize;
            for &other in &self.order[first..first + cell.count as usize] {
                let other = other as usize;
                if other == node {
                    continue;
                }
                let limit = own + radius * drawn_scale(degrees[other]);
                let mut dx = point[0] - pos[other][0];
                let mut dy = point[1] - pos[other][1];
                let dist_sq = dx * dx + dy * dy;
                if dist_sq >= limit * limit {
                    continue;
                }
                let mut dist = dist_sq.sqrt();
                if dist == 0.0 {
                    let (low, high) = (node.min(other), node.max(other));
                    let angle = pair_angle(low as u64, high as u64);
                    let sign = if node == low { -1.0 } else { 1.0 };
                    dx = angle.cos() * sign;
                    dy = angle.sin() * sign;
                    dist = 1.0;
                }
                let push = stiffness * (limit - dist_sq.sqrt());
                force[0] += dx / dist * push;
                force[1] += dy / dist * push;
            }
        }
        force
    }
}

/// The largest [`drawn_scale`] ever returns.
pub(crate) const DRAWN_SCALE_MAX: f32 = 3.0;

/// How much larger than a bare mark the wiki draws a node of `degree`
/// (`1 + incident edges`): logarithmic in degree, capped at three.
pub(crate) fn drawn_scale(degree: f32) -> f32 {
    (1.0 + degree.ln() * 0.4).min(DRAWN_SCALE_MAX)
}

/// `repulsion / r^falloff`, with the lifted law's exact arithmetic at `2`.
#[inline]
fn law(repulsion: f32, dist_sq: f32, dist: f32, falloff: f32) -> f32 {
    if falloff == 2.0 {
        repulsion / dist_sq
    } else if falloff == 1.0 {
        repulsion / dist
    } else {
        repulsion / dist.powf(falloff)
    }
}

/// A direction for a coincident pair, mixed from the two indices.
fn pair_angle(low: u64, high: u64) -> f32 {
    let mut z = low
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(high.wrapping_mul(0xbf58_476d_1ce4_e5b9));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 40) as f32 / 16_777_216.0 * std::f32::consts::TAU
}

fn quadrant(point: [f32; 2], centre: [f32; 2]) -> usize {
    usize::from(point[0] >= centre[0]) | (usize::from(point[1] >= centre[1]) << 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_skips_records_it_cannot_place() {
        // An open-world view may name an endpoint it did not include, and a
        // self-edge is a record the force law has no term for. Both are
        // skipped; neither is an error.
        let csr = Csr::build(3, &[(0, 1), (1, 9), (2, 2)]);
        assert_eq!(csr.neighbours(0), &[1]);
        assert_eq!(csr.neighbours(1), &[0]);
        assert!(csr.neighbours(2).is_empty());
        assert_eq!(csr.edge_count(), 1);
    }

    #[test]
    fn csr_degree_matches_the_lifted_kernel() {
        // The kernel's `degrees` array was `1.0 + incident edges`, and the
        // symmetric attraction weight divides by its square root.
        let csr = Csr::build(3, &[(0, 1), (0, 2)]);
        assert_eq!(csr.degree(0), 3.0);
        assert_eq!(csr.degree(1), 2.0);
        assert_eq!(csr.degree(2), 2.0);
    }

    #[test]
    fn a_tree_over_coincident_points_terminates() {
        // Every node at one point cannot be separated by subdivision. The
        // depth cap is what makes this a leaf bucket rather than a hang.
        let pos = vec![[3.0f32, -2.0]; 64];
        let mut tree = QuadTree::default();
        tree.build(&pos, None);
        let mut stack = Vec::new();
        let force = tree.repulsion(0, &pos, 140_000.0, 0.7, 2.0, None, &mut stack);
        assert!(force[0].is_finite() && force[1].is_finite());
    }
}
