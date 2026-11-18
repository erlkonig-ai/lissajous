use eframe::egui::{vec2, Color32, Painter, Pos2, Response, Sense, TextStyle, Ui, Widget};

use crate::graph::{self, Anchors, Glyph, GraphViewport, LayoutParams, LayoutStats, Lod, Stroke2};
use crate::themes;

/// What kind of element a node stands for.
///
/// The collection algebra has exactly one meaningful split here, and it is not
/// "collection versus member": it is whether the element was **asserted** or
/// **computed**. A `COMMIT` is exogenous — no machine can recompute whether an
/// author meant to publish it — while a `MERGE` result, a `DERIVE` output and a
/// derived descriptor are all reproducible from their inputs. That is the
/// distinction a reader needs in order to know what would survive losing the
/// store's materialized work, so it is the one the shape carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatticeMark {
    /// Asserted by an author: a `COMMIT` member, or a named root collection.
    Authored,
    /// Reproducible from inputs: a `MERGE` result, a `DERIVE` output, or a
    /// derived collection descriptor.
    Computed,
}

/// Whether the element is resident in the observation being drawn.
///
/// Absence is a first-class value here because a store is an open world: a
/// record may name a descriptor, an input or an output whose bytes simply are
/// not here. Omitting such a node would quietly redraw the lattice as though it
/// were complete, which is the failure this widget exists to prevent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatticePresence {
    /// The element's bytes are resident in this observation.
    Present,
    /// The element is named or implied, but not resident. Drawn as an explicit
    /// hole — a dashed outline — never omitted.
    Absent,
}

/// Whether an admitted record put the element where it is.
///
/// A store admits a record when some capability proof names its signer. Until
/// one does, the record is *parked*: it is here, it is valid, and nothing it
/// says has been folded in. That is an absence with an owner — somebody must
/// issue a grant — and it is invisible in every other channel, because a
/// parked `COMMIT` names its payload exactly as an admitted one does.
///
/// The split that matters is therefore [`Unadmitted`](Self::Unadmitted)
/// against the rest, and that is the one the geometry carries. Admitted and
/// [`Unknown`](Self::Unknown) differ only in hue, deliberately: neither is a
/// condition anybody acts on, and the kit's rule forbids resting an actionable
/// fact on colour, not every fact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LatticeAdmission {
    /// An admitted record put this element here.
    Admitted,
    /// A record puts it here and no proof admits that record's signer yet.
    /// Drawn as an open mark: the bytes are here and nothing vouches for them.
    Unadmitted,
    /// Neither fact is in evidence.
    ///
    /// The default, so a caller that has not looked draws exactly what it drew
    /// before this channel existed. It is also the honest answer for an element
    /// whose collection could not be settled at all — a lineage that has not
    /// replicated is waiting on *bytes*, which is a different absence with a
    /// different owner, and drawing it as a missing grant would send a reader
    /// to ask the wrong person.
    #[default]
    Unknown,
}

/// One element of the lattice.
#[derive(Clone, Debug)]
pub struct LatticeNode {
    /// Short handle or name, drawn under the mark.
    pub label: String,
    /// Asserted or computed.
    pub mark: LatticeMark,
    /// Resident here, or a hole.
    pub presence: LatticePresence,
    /// Materialized share of the work this node stands for, in `0.0..=1.0`.
    ///
    /// For a collection node this is endorsed equations over expected ones: the
    /// derive backlog, drawn rather than described. `None` means the caller had
    /// nothing comparable to divide, and draws as an **empty track** — absence
    /// of evidence is not completeness, and the two must not look alike.
    pub coverage: Option<f32>,
    /// Was anything here seen to produce this element?
    ///
    /// `false` is a member reached only as somebody else's join input: it is
    /// resident, it is real, and the record that made it lives somewhere this
    /// observation cannot see. It draws as a circle with a gap at the bottom —
    /// nothing below it produced it.
    ///
    /// This was the one absence in the whole view carried by *text*: a count
    /// printed beside the picture, while the marks themselves were pixel
    /// identical to fully-produced outputs. A reader cannot point at a number
    /// and ask which ones.
    pub produced: bool,
    /// Has an admitted record put this element here?
    ///
    /// Defaults to [`LatticeAdmission::Unknown`], which draws as it always did.
    pub admission: LatticeAdmission,
    /// Is the label the element's real name, or a stand-in for one?
    ///
    /// `false` when the name exists but its bytes are not resident, so the
    /// label shown is a short handle. Drawn in dim ink with a leading middle
    /// dot — deliberately **not** a dashed underline, because underlines are
    /// reserved for links and a dashed one would read as a broken link rather
    /// than as a missing name.
    pub label_known: bool,
}

impl Default for LatticeNode {
    fn default() -> Self {
        LatticeNode {
            label: String::new(),
            mark: LatticeMark::Computed,
            presence: LatticePresence::Present,
            coverage: None,
            produced: true,
            admission: LatticeAdmission::Unknown,
            label_known: true,
        }
    }
}

/// A directed edge: `from` is below `to` in the order — a source, a merge
/// input, or the collection a derivation reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatticeEdge {
    /// Index of the lower element.
    pub from: usize,
    /// Index of the upper element.
    pub to: usize,
    /// `true` when a stored record endorses this step.
    ///
    /// `false` is the other half of "what is missing": the model says this
    /// equation should exist — a source member with no derivation, a pair the
    /// join never reached — and no record endorses it. It draws dashed, so an
    /// unfinished lattice looks unfinished.
    pub endorsed: bool,
}

/// One element of an attached collection, drawn in depth over the node it
/// indexes.
///
/// An attached collection is not a second lattice beside its parent: its
/// mapping takes nodes of the parent's lattice to attachments, one `MAP`
/// record each. So it is drawn as a layer standing over the parent, each
/// attachment directly above the node it indexes and offset along a depth
/// axis, rather than as more nodes in the plane competing with the order for
/// position. The layout never sees an overlay: rank and lane belong to the
/// lattice, and an attachment's place is a function of its anchor's.
///
/// Only an attachment some record put here is ever drawn. A parent node with
/// no `MAP` simply has nothing above it; inventing a hole for every node an
/// attached collection *could* index would draw work nobody declared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatticeOverlay {
    /// Index of the node in the lattice this attachment indexes.
    pub anchor: usize,
    /// Layer, from `1`: an attachment of the drawn lattice is depth one, an
    /// attachment of that attached collection is depth two, and so on. Depth
    /// zero is the lattice itself and draws nothing.
    pub depth: u8,
    /// The attachment's bytes are here, or it is a hole.
    pub presence: LatticePresence,
    /// Restated in hue exactly as a node's is.
    pub admission: LatticeAdmission,
}

/// What the pointer did to the lattice this frame.
pub struct LatticeResponse {
    /// The allocated region's own response.
    pub response: Response,
    /// Node under the pointer, if the pointer is near one.
    pub hovered: Option<usize>,
    /// Node clicked this frame, if any.
    pub clicked: Option<usize>,
    /// Marks drawn in full this frame.
    ///
    /// A view that folds and does not disclose it is a view you cannot trust
    /// the next time it looks sparse.
    pub drawn: usize,
    /// Nodes collapsed to a dot because the zoom made their mark unreadable.
    pub folded: usize,
    /// Overlay marks drawn this frame. An overlay whose anchor is out of range
    /// or off screen is not counted, so a caller can say what it skipped.
    pub overlaid: usize,
}

/// A collection lattice, drawn as a settling layout with ranked lanes.
///
/// # Why a simulation, when a partial order has a canonical picture
///
/// This widget used to argue that ranking is a pure function of the edge set,
/// that the same records therefore always draw the same picture, and that a
/// force-directed layout would discard exactly the structure that matters. The
/// first two are still true and the third is the part that has been fixed
/// rather than abandoned: **the ranking did not go away, it became a force.**
/// A node's rank is still its longest path from a minimal element, computed by
/// exactly the same [`levels`] function, and it is now a stiff spring pinning
/// the node's `x`. A source is still always to the left of everything it feeds.
///
/// What the simulation buys is the `y` axis, which the old layout had to guess
/// at by stacking siblings in index order. Left free, `y` is available to carry
/// something real: a cross-collection derivation edge is an ordinary spring,
/// and because `x` is pinned it can only pull in `y` — so a member and the
/// member it was derived into line up, and a member with no partner visibly
/// fails to. That is the derive backlog drawn instead of counted, and it is not
/// available to a layout that has already spent `y` on index order.
///
/// It also buys size. The old placement compressed a column to fit the
/// allocated height, so a collection with twenty thousand members drew twenty
/// thousand marks inside a few hundred points; a pan-and-zoom viewport over a
/// settling layout draws the same members at a size a reader can use.
///
/// # Encoding
///
/// Facts, each in its own channel, none of them colour alone:
///
/// * **Shape** — square is authored, circle is computed, and a circle with a
///   gap at the bottom is an element nothing here produced.
/// * **Stroke** — solid is resident and vouched for, an open outline is
///   resident with no proof admitting the record that put it here, dashed is a
///   hole. The three are ordered by how much of the element we actually have.
///   Dashed edges are equations the model expects and no record endorses.
/// * **Arc** — the track around a mark is materialized work over expected
///   work. An empty track is no evidence, not agreement.
/// * **Position** — rank is horizontal, and vertical alignment across lanes is
///   correspondence.
/// * **Motion** — energy is granted only where something changed.
///
/// Hue is a fourth channel and never the only one. It restates admission —
/// [`themes::admitted_mark`] against [`themes::unadmitted_mark`], the widest
/// colourblind-safe separation in the palette — and in doing so it separates
/// admitted from unknown, which nothing else does. That is allowed precisely
/// because it is the one distinction here nobody acts on; the distinction that
/// *is* actionable, a member waiting on a grant, rides on the stroke.
///
/// The accent is spent on one thing only: the ring around the selected node.
/// It is the same value as [`themes::unadmitted_mark`], which is why an
/// unadmitted mark is never told apart from a selected one by colour: one is a
/// mark, the other a ring at the track radius.
///
/// # Selection
///
/// Passing `selected` dims everything that is not on that node's chain, so a
/// derived collection can be walked back to the sources it was computed from.
/// The chain is the transitive closure in **both** directions: every element
/// that feeds the selection and everything the selection feeds. Unendorsed
/// edges are part of it, because the missing step is exactly what a reader
/// following a broken chain needs to see. It is computed when the selection
/// changes, not per frame.
///
/// ```ignore
/// let lattice = LatticeGraph::new(&nodes, &edges)
///     .selected(selected)
///     .show(ui);
/// if let Some(index) = lattice.clicked {
///     selected = Some(index);
/// }
/// ```
#[must_use = "You should show this widget with `.show(ui)` or `ui.add(widget)`"]
pub struct LatticeGraph<'a> {
    nodes: &'a [LatticeNode],
    edges: &'a [LatticeEdge],
    overlays: &'a [LatticeOverlay],
    selected: Option<usize>,
    height: Option<f32>,
    salt: &'static str,
}

impl<'a> LatticeGraph<'a> {
    /// A lattice of `nodes` ordered by `edges`.
    ///
    /// Edges whose endpoints fall outside `nodes` are skipped rather than
    /// panicking: the records are an open-world observation and may name an
    /// element the caller did not include.
    pub fn new(nodes: &'a [LatticeNode], edges: &'a [LatticeEdge]) -> Self {
        Self {
            nodes,
            edges,
            overlays: &[],
            selected: None,
            height: None,
            salt: "gorbie_lattice",
        }
    }

    /// Dim everything off this node's chain. Out-of-range indices select
    /// nothing, which keeps a stale selection from blanking the view.
    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.selected = selected;
        self
    }

    /// Draw attachments in depth over the nodes they index. See
    /// [`LatticeOverlay`]. Overlays never enter the layout, so adding or
    /// hiding a layer does not move a single node of the lattice.
    pub fn overlays(mut self, overlays: &'a [LatticeOverlay]) -> Self {
        self.overlays = overlays;
        self
    }

    /// Fix the drawing height. Width always follows the available space.
    ///
    /// Left alone, the height is computed from the shape of the ranking: one
    /// row pitch per node in the busiest column, bounded above so a collection
    /// with twenty thousand members asks for a viewport rather than a mile of
    /// page. A three-node lattice still takes three rows of space instead of
    /// reserving a screenful and drawing a sparse picture in it.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Distinguish two lattices drawn from the same `Ui`.
    pub fn id_salt(mut self, salt: &'static str) -> Self {
        self.salt = salt;
        self
    }

    /// Draw the lattice and report what the pointer did.
    pub fn show(self, ui: &mut Ui) -> LatticeResponse {
        let count = self.nodes.len();
        let ranks = levels(count, self.edges);
        let width = ui.available_width();

        // The key is the label, which is what the caller uses to tell two
        // elements apart on screen. It is a UI handle and never a lookup: two
        // elements that collide on one lose a carried position between frames
        // and nothing else.
        let keys: Vec<u64> = self
            .nodes
            .iter()
            .map(|node| graph::key_of(node.label.as_bytes()))
            .collect();
        let pairs: Vec<(u32, u32)> = self
            .edges
            .iter()
            .filter(|edge| edge.from != edge.to)
            .map(|edge| (edge.from as u32, edge.to as u32))
            .collect();

        // The layout is built BEFORE the region is allocated, because how tall
        // this widget should be is a question about the shape of the thing
        // being drawn, and answering it needs no rect.
        let layout_id = ui.id().with(self.salt);
        let (shared, changed) =
            graph::shared(ui, layout_id, &keys, &pairs, LayoutParams::default());
        let mut shared = shared.lock();
        let height = self
            .height
            .unwrap_or_else(|| auto_height(&ranks, shared.layout().stats(), width));
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        if !ui.is_rect_visible(rect) || self.nodes.is_empty() {
            drop(shared);
            return LatticeResponse {
                response,
                hovered: None,
                clicked: None,
                drawn: 0,
                folded: 0,
                overlaid: 0,
            };
        }
        let generation = shared.generation();
        let layout = shared.layout_mut();

        let view_id = layout_id.with("view");
        let mut view = GraphViewport::load(ui, view_id);
        if changed {
            // Rank is the `x` anchor and it is stiff; `y` is left slack so a
            // cross-lane correspondence spring has somewhere to pull. Seeding
            // on the anchor rather than on a ring means the picture arrives
            // already readable and settles into detail, rather than unwinding
            // from a circle every time a record lands.
            let target: Vec<[f32; 2]> = ranks
                .iter()
                .enumerate()
                .map(|(index, rank)| [rank_x(*rank), seed_y(index, count)])
                .collect();
            for (index, point) in target.iter().enumerate() {
                layout.seed(index, *point);
            }
            let anchors = Anchors {
                target: target.iter().map(|point| [point[0], 0.0]).collect(),
                k: vec![[RANK_K, LANE_K]; count],
            };
            layout.anchors(Some(&anchors));
            view.release();
        }
        let stats = layout.step_budget(STEP_BUDGET_MS);
        view.interact(ui, &response, rect);
        view.fit_unless_touched(rect, stats.bounds, LABEL_MARGIN);
        view.store(ui, view_id);
        if !stats.quiet {
            ui.ctx().request_repaint();
        }

        let positions = layout.positions();
        let screen: Vec<Pos2> = (0..count)
            .map(|index| view.to_screen(rect, positions[index]))
            .collect();
        drop(shared);

        let lit = lit_chain(ui, layout_id, generation, count, self.edges, self.selected);

        let visuals = ui.visuals();
        let ink = visuals.text_color();
        let hairline = visuals.widgets.noninteractive.bg_stroke.color;
        let dim = themes::blend(visuals.window_fill, ink, 0.32);
        let track = themes::blend(visuals.window_fill, hairline, 0.55);
        let accent = themes::button_light_on();
        let painter = ui.painter().with_clip_rect(rect);
        let font = TextStyle::Small.resolve(ui.style());
        let scale = view.mark_scale();
        // Rank pitch is the widest regular spacing in this layout; nodes
        // sharing a rank sit closer, which is what the fold is for.
        let lod = view.lod(RANK_PITCH * 0.5);
        let visible = rect.expand(graph::HIT * scale + 4.0);

        // Edges first, so marks always sit on top of them.
        for edge in self.edges {
            let (Some(from), Some(to)) = (screen.get(edge.from), screen.get(edge.to)) else {
                continue;
            };
            if edge.from == edge.to || !(visible.contains(*from) || visible.contains(*to)) {
                continue;
            }
            let on_chain = lit[edge.from] && lit[edge.to];
            let colour = if on_chain { hairline } else { dim };
            // An unendorsed edge is the exception this view exists to show, so
            // it keeps its weight while the ordinary ones thin out. Links
            // recede by thinning and never by lightening: see `draw_link`.
            let weight = if !edge.endorsed {
                1.0
            } else {
                match lod {
                    Lod::Full => 1.0,
                    Lod::Marks => 0.7,
                    _ => 0.45,
                }
            };
            graph::draw_link(
                &painter,
                *from,
                *to,
                scale,
                edge.endorsed,
                None,
                colour,
                weight,
            );
        }

        let mut labelled = 0usize;
        let mut drawn = 0usize;
        let mut folded = 0usize;
        for (index, node) in self.nodes.iter().enumerate() {
            let at = screen[index];
            if !visible.contains(at) {
                continue;
            }
            // Off the selected chain the node is dim and stays dim: a
            // selection that some hues ignored would not be a selection.
            let colour = if lit[index] { ink } else { dim };
            // The admission hue is for the MARK alone. The palette caps any
            // single value against either ground at 4.00:1, under the 4.5:1
            // text threshold -- so a label wearing it would be a label nobody
            // can read, and the track carries a different fact entirely.
            let mark = if lit[index] {
                mark_colour(node.admission, ink)
            } else {
                dim
            };

            // The level of detail decimates the norm, never the exception. A
            // hole, an unproduced member and a member waiting on a grant are
            // the three things anybody opened this view for, so they keep their
            // full mark at every zoom, on a small knockout of the page ground
            // so they stay legible where the field is densest.
            let exception = matches!(node.presence, LatticePresence::Absent)
                || !node.produced
                || matches!(node.admission, LatticeAdmission::Unadmitted);
            let detail = if exception { Lod::Full } else { lod };
            if matches!(detail, Lod::Dots | Lod::Aggregate) {
                painter.circle_filled(at, 1.5, mark);
                folded += 1;
                continue;
            }
            drawn += 1;
            if exception && !matches!(lod, Lod::Full) {
                painter.circle_filled(at, graph::MARK * scale + 2.0, visuals.panel_fill);
            }
            if matches!(detail, Lod::Full) {
                graph::draw_track(
                    &painter,
                    at,
                    scale,
                    node.coverage,
                    if lit[index] { track } else { dim },
                    colour,
                );
            }
            draw_mark(&painter, at, scale, node, mark);
            if self.selected == Some(index) {
                painter.circle_stroke(
                    at,
                    (graph::TRACK + 4.0) * scale,
                    eframe::egui::Stroke::new(1.5_f32, accent),
                );
            }

            // A label budget, not a label per node. Twelve-character handles at
            // the small text style are about seventy-eight points wide, so a
            // dense column overlaps its own names past nine ranks and the
            // reader loses the very thing they were reading. A node too small
            // to label is a node too small to click: zoom in.
            if !matches!(lod, Lod::Full) || labelled >= LABEL_BUDGET {
                continue;
            }
            labelled += 1;
            let colour = if node.label_known {
                colour
            } else {
                themes::blend(visuals.window_fill, colour, 0.55)
            };
            let text = if node.label_known {
                node.label.clone()
            } else {
                format!("\u{b7} {}", node.label)
            };
            let galley = painter.layout_no_wrap(text, font.clone(), colour);
            let left = (at.x - galley.size().x * 0.5).clamp(
                rect.left(),
                (rect.right() - galley.size().x).max(rect.left()),
            );
            painter.galley(
                eframe::egui::pos2(left, at.y + graph::TRACK * scale + 3.0),
                galley,
                colour,
            );
        }

        // Overlays last, so a layer stands over the lattice rather than under
        // it. Each sits up and to the right of its anchor, one step per depth,
        // joined to the layer below by a short stem: the stem is the MAP
        // record itself, so it is always solid -- an attachment with no record
        // is not drawn at all.
        let mut overlaid = 0usize;
        for overlay in self.overlays {
            let Some(&at) = screen.get(overlay.anchor) else {
                continue;
            };
            if overlay.depth == 0 || !visible.contains(at) {
                continue;
            }
            let exception = matches!(overlay.presence, LatticePresence::Absent)
                || matches!(overlay.admission, LatticeAdmission::Unadmitted);
            if matches!(lod, Lod::Dots | Lod::Aggregate) && !exception {
                continue;
            }
            let lit_here = lit[overlay.anchor];
            // Deeper layers recede into the page ground, so a stack of three
            // reads as a stack and not as three equal marks fighting.
            let recede = (0.22 * overlay.depth as f32).min(0.6);
            let base = if lit_here {
                mark_colour(overlay.admission, ink)
            } else {
                dim
            };
            let colour = themes::blend(base, visuals.window_fill, recede);
            let below = at + depth_offset(overlay.depth - 1, scale);
            let above = at + depth_offset(overlay.depth, scale);
            painter.line_segment(
                [below, above],
                eframe::egui::Stroke::new(1.0_f32, if lit_here { hairline } else { dim }),
            );
            let mark = LatticeNode {
                mark: LatticeMark::Computed,
                presence: overlay.presence,
                admission: overlay.admission,
                ..LatticeNode::default()
            };
            draw_mark(&painter, above, scale * OVERLAY_SCALE, &mark, colour);
            overlaid += 1;
        }

        let near = |at: Pos2| graph::pick(&screen, at, graph::HIT * scale.max(0.4));
        let hovered = response.hover_pos().and_then(near);
        let clicked = response
            .clicked()
            .then(|| response.interact_pointer_pos().and_then(near))
            .flatten();

        LatticeResponse {
            response,
            hovered,
            clicked,
            drawn,
            folded,
            overlaid,
        }
    }
}

impl Widget for LatticeGraph<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui).response
    }
}

/// World-space distance between two ranks.
const RANK_PITCH: f32 = 260.0;
/// Stiffness of the rank spring. Stiff: rank is the one thing the simulation is
/// not allowed to renegotiate, because it is the order itself.
///
/// The per-step impulse is capped at `max_force`, so a node's worst-case
/// departure from its rank is `max_force / RANK_K` world units. At these values
/// that is thirty against a pitch of two hundred and sixty, so a mark never
/// leaves its own column. The discrete spring is stable to roughly `k = 6.4` at
/// the shared damping, so this is nowhere near the edge.
const RANK_K: f32 = 0.5;
/// Stiffness of the lane spring. Slack, so a correspondence edge from another
/// lane can pull a node into line without fighting it.
const LANE_K: f32 = 0.0008;
/// Vertical distance between two marks sharing a column, before settling.
const PITCH: f32 = 46.0;
/// Space kept clear above a mark, and below it for the label.
const TOP: f32 = graph::TRACK + 4.0;
/// Space kept clear under the lowest mark, sized for its label.
const BOTTOM: f32 = graph::TRACK + 18.0;
/// Screen points kept clear around the marks for labels and end columns.
const LABEL_MARGIN: f32 = 40.0;
/// Solver time per frame.
const STEP_BUDGET_MS: f32 = 4.0;
/// Labels drawn per frame, at most.
const LABEL_BUDGET: usize = 512;
/// Rows a column may claim before the view stops growing and starts scrolling
/// its own viewport instead.
const MAX_ROWS: f32 = 18.0;

/// Mark size of an overlay against a lattice node: smaller, because an
/// attachment is a fact about its anchor rather than an element of the order.
const OVERLAY_SCALE: f32 = 0.7;
/// Screen offset of one depth step, before zoom. Up and to the right, an
/// oblique projection of a z axis out of the page; steep enough that a stem
/// never lands on a label, which sits below its mark.
const DEPTH_STEP: [f32; 2] = [8.0, -13.0];

fn depth_offset(depth: u8, scale: f32) -> eframe::egui::Vec2 {
    vec2(DEPTH_STEP[0], DEPTH_STEP[1]) * depth as f32 * scale.max(0.4)
}

fn rank_x(rank: usize) -> f32 {
    rank as f32 * RANK_PITCH
}

/// Where a node starts inside its column, before the springs take over.
fn seed_y(index: usize, count: usize) -> f32 {
    // Spread by index so two nodes never start coincident. They would separate
    // anyway — the shared solver pushes a coincident pair apart — but arriving
    // already spread means the first frame a reader sees is already a lattice.
    let span = (count.max(1) as f32).sqrt() * PITCH;
    (index as f32 / count.max(1) as f32 - 0.5) * span * 2.0
}

/// Height from the shape of the thing being drawn.
///
/// Once the layout has taken a step this follows its ASPECT, because the
/// viewport frames the whole layout into whatever region it is given: a card
/// too short for the layout's proportions does not crop the picture, it shrinks
/// it. That is not a theory. A six-node lattice occupying roughly a square of
/// world came back drawn inside a 136-point strip, every mark under three
/// points and not a label in sight, on a page with room to spare — found by
/// rendering it, which no assertion about shape kinds was ever going to catch.
///
/// Before the first step there are no bounds, so it falls back to the shape of
/// the ranking: one row pitch per node in the busiest column, which is the rule
/// this widget has always used and still keeps a three-node lattice from
/// reserving a screenful. Either way it is bounded above, so twenty thousand
/// members ask for a viewport rather than a mile of page.
///
/// Counted in ONE pass. The old expression counted a column's members once per
/// member, which is quadratic and was a real cost on a collection with
/// thousands of them, paid before anything was drawn at all.
fn auto_height(ranks: &[usize], stats: LayoutStats, width: f32) -> f32 {
    let mut rows: Vec<u32> = Vec::new();
    for rank in ranks {
        if *rank >= rows.len() {
            rows.resize(rank + 1, 0);
        }
        rows[*rank] += 1;
    }
    let busiest = rows.iter().copied().max().unwrap_or(1).max(1) as f32;
    let floor = PITCH + TOP + BOTTOM;
    let ceiling = MAX_ROWS * PITCH + TOP + BOTTOM;
    let by_rank = (busiest.min(MAX_ROWS) * PITCH + TOP + BOTTOM).clamp(floor, ceiling);

    let world = vec2(
        stats.bounds[2] - stats.bounds[0],
        stats.bounds[3] - stats.bounds[1],
    );
    if stats.steps == 0 || !world.y.is_finite() || world.y <= 0.0 {
        return by_rank;
    }
    // A single column has NO x-span at all -- the rank spring pins it exactly,
    // measured at 0.00 for every count -- so the aspect is undefined and this
    // used to fall back to the pre-step estimate. That is the commonest member
    // lattice there is, a collection whose commits nothing has joined yet, and
    // the estimate offers one PITCH per node against a settled spacing of
    // roughly five of them: every such lattice was drawn at about a fifth of
    // its size, marks and labels with it. With no width to scale against, the
    // world's own height IS the answer, and the ceiling still bounds it.
    let usable = (width - 2.0 * LABEL_MARGIN).max(1.0);
    let height = match world.x > 1.0 {
        true => usable * (world.y / world.x),
        false => world.y,
    };
    (height + 2.0 * LABEL_MARGIN).clamp(floor, ceiling)
}

/// The chain, recomputed only when the selection or the topology changes.
fn lit_chain(
    ui: &Ui,
    id: eframe::egui::Id,
    generation: u64,
    count: usize,
    edges: &[LatticeEdge],
    selected: Option<usize>,
) -> std::sync::Arc<Vec<bool>> {
    type Cached = std::sync::Arc<(u64, Option<usize>, std::sync::Arc<Vec<bool>>)>;
    let id = id.with("chain");
    let cached: Option<Cached> = ui.ctx().data(|data| data.get_temp(id));
    if let Some(cached) = &cached {
        if cached.0 == generation && cached.1 == selected {
            return cached.2.clone();
        }
    }
    let lit = std::sync::Arc::new(match selected.filter(|index| *index < count) {
        Some(index) => chain(count, edges, index),
        None => vec![true; count],
    });
    let entry: Cached = std::sync::Arc::new((generation, selected, lit.clone()));
    ui.ctx().data_mut(|data| data.insert_temp(id, entry));
    lit
}

/// The hue that restates a mark's admission.
///
/// Separate from [`draw_mark`] because it is the one channel the render tests
/// cannot see: they count shapes, and a shape has no opinion about the colour
/// it was handed.
fn mark_colour(admission: LatticeAdmission, ink: Color32) -> Color32 {
    match admission {
        LatticeAdmission::Admitted => themes::admitted_mark(),
        LatticeAdmission::Unadmitted => themes::unadmitted_mark(),
        // The ordinary ink, so a caller that has not looked at admission draws
        // exactly the picture it drew before this channel existed.
        LatticeAdmission::Unknown => ink,
    }
}

fn draw_mark(painter: &Painter, at: Pos2, scale: f32, node: &LatticeNode, colour: Color32) {
    // Three strokes for three amounts of the element: filled is here and
    // vouched for, open is here with nothing vouching, dashed is not here.
    // Absence outranks admission because a hole has no record to admit --
    // saying "waiting on a grant" about bytes we do not have would name a
    // remedy that cannot be applied.
    let outline = match (node.presence, node.admission) {
        (LatticePresence::Absent, _) => Stroke2::Dashed,
        (LatticePresence::Present, LatticeAdmission::Unadmitted) => Stroke2::Open,
        (LatticePresence::Present, _) => Stroke2::Filled,
    };
    let glyph = match (node.mark, node.produced) {
        // Authored elements are produced by definition — someone asserted
        // them — so the open-below glyph is never reachable for a square.
        (LatticeMark::Authored, _) => Glyph::Square,
        (LatticeMark::Computed, true) => Glyph::Circle,
        (LatticeMark::Computed, false) => Glyph::CircleOpenBelow,
    };
    graph::draw_node(painter, at, scale, glyph, outline, colour, 1);
}

/// Rank every node by its longest path from a minimal element.
///
/// This is the layout's whole geometry: a node's rank is its column. Ranking by
/// *longest* path rather than shortest is what keeps a node strictly to the
/// right of every one of its sources even when a short edge skips a generation,
/// which is the property that makes the picture readable as an order.
///
/// Unendorsed edges rank exactly like endorsed ones. A missing derivation still
/// says where its output would sit, and placing the hole in the right column is
/// the point of drawing it at all.
pub fn levels(count: usize, edges: &[LatticeEdge]) -> Vec<usize> {
    let mut level = vec![0usize; count];
    let mut indegree = vec![0usize; count];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); count];
    for edge in edges {
        if edge.from >= count || edge.to >= count || edge.from == edge.to {
            continue;
        }
        indegree[edge.to] += 1;
        outgoing[edge.from].push(edge.to);
    }

    // Kahn's algorithm seeded in index order, so the ranking is a pure function
    // of the input and never depends on hash or iteration order.
    let mut queue: Vec<usize> = (0..count).filter(|node| indegree[*node] == 0).collect();
    let mut settled = vec![false; count];
    let mut head = 0;
    while head < queue.len() {
        let node = queue[head];
        head += 1;
        settled[node] = true;
        for index in 0..outgoing[node].len() {
            let next = outgoing[node][index];
            level[next] = level[next].max(level[node] + 1);
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push(next);
            }
        }
    }

    // A cycle has no rank. Records should not produce one, but the reader is
    // open-world and must not hang or panic on bytes it did not write: park
    // every unranked node one column past everything that could be ranked, so
    // the nodes and their edges are still drawn and still obviously odd.
    let parked = level.iter().copied().max().map_or(0, |max| max + 1);
    for node in 0..count {
        if !settled[node] {
            level[node] = parked;
        }
    }
    level
}

/// Every node that feeds `selected`, everything `selected` feeds, and itself.
///
/// Both directions, because a reader asking "where did this come from" almost
/// always then asks "and what else was built on it". Unendorsed edges are
/// followed too: a chain that is broken is still the chain you asked for, and
/// hiding the break would answer the question wrongly.
///
/// The walk is over adjacency built once, not over the edge slice per frontier
/// node. Same answer; the old shape was `O(n * e)` and this is `O(n + e)`.
fn chain(count: usize, edges: &[LatticeEdge], selected: usize) -> Vec<bool> {
    let mut on = vec![false; count];
    if selected >= count {
        return on;
    }
    let mut upward: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut downward: Vec<Vec<usize>> = vec![Vec::new(); count];
    for edge in edges {
        if edge.from >= count || edge.to >= count {
            continue;
        }
        upward[edge.from].push(edge.to);
        downward[edge.to].push(edge.from);
    }

    on[selected] = true;
    for adjacency in [&downward, &upward] {
        let mut frontier = vec![selected];
        while let Some(node) = frontier.pop() {
            for next in &adjacency[node] {
                if !on[*next] {
                    on[*next] = true;
                    frontier.push(*next);
                }
            }
        }
    }
    on
}

#[cfg(test)]
fn settled(nodes: usize, edges: &[LatticeEdge], steps: usize) -> Vec<[f32; 2]> {
    use crate::graph::ForceLayout;
    let ranks = levels(nodes, edges);
    let pairs: Vec<(u32, u32)> = edges
        .iter()
        .filter(|edge| edge.from != edge.to)
        .map(|edge| (edge.from as u32, edge.to as u32))
        .collect();
    let mut layout = ForceLayout::new(nodes, &pairs, LayoutParams::default());
    let target: Vec<[f32; 2]> = ranks
        .iter()
        .enumerate()
        .map(|(index, rank)| [rank_x(*rank), seed_y(index, nodes)])
        .collect();
    for (index, point) in target.iter().enumerate() {
        layout.seed(index, *point);
    }
    layout.anchors(Some(&Anchors {
        target: target.iter().map(|point| [point[0], 0.0]).collect(),
        k: vec![[RANK_K, LANE_K]; nodes],
    }));
    for _ in 0..steps {
        layout.step();
    }
    layout.positions().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: usize, to: usize) -> LatticeEdge {
        LatticeEdge {
            from,
            to,
            endorsed: true,
        }
    }

    #[test]
    fn a_source_chain_ranks_one_node_per_column() {
        // facts -> index -> summary: exactly the derive chain the view exists
        // to walk, and it must read left to right without stacking.
        let ranks = levels(3, &[edge(0, 1), edge(1, 2)]);
        assert_eq!(ranks, vec![0, 1, 2]);
    }

    #[test]
    fn a_join_sits_past_the_longer_of_its_two_arms() {
        // 0 -> 1 -> 2 -> 3 and 0 -> 3. Shortest-path ranking would put the
        // join in column 1 and draw an edge running backwards.
        let ranks = levels(4, &[edge(0, 1), edge(1, 2), edge(2, 3), edge(0, 3)]);
        assert_eq!(ranks, vec![0, 1, 2, 3]);
    }

    #[test]
    fn an_unendorsed_edge_ranks_its_missing_output() {
        // The whole point of drawing a hole is that it lands in the column
        // where the derivation would have gone.
        let ranks = levels(
            2,
            &[LatticeEdge {
                from: 0,
                to: 1,
                endorsed: false,
            }],
        );
        assert_eq!(ranks, vec![0, 1]);
    }

    #[test]
    fn a_cycle_is_parked_instead_of_hanging() {
        // Records should never produce one, but an open-world reader must not
        // spin or panic on bytes it did not write.
        let ranks = levels(3, &[edge(0, 1), edge(1, 2), edge(2, 1)]);
        assert_eq!(ranks.len(), 3);
        assert_eq!(ranks[0], 0);
        assert!(
            ranks[1] > 0 && ranks[2] > 0,
            "cycle nodes still get a column"
        );
    }

    #[test]
    fn out_of_range_and_self_edges_are_skipped_not_panics() {
        let ranks = levels(2, &[edge(0, 9), edge(7, 1), edge(1, 1), edge(0, 1)]);
        assert_eq!(ranks, vec![0, 1]);
    }

    #[test]
    fn the_chain_reaches_transitive_sources_and_leaves_siblings_out() {
        // 0 -> 1 -> 2 is the chain; 3 -> 2 is a second source of the same
        // target and belongs in it; 4 is unrelated and must stay dimmed.
        let lit = chain(5, &[edge(0, 1), edge(1, 2), edge(3, 2)], 2);
        assert_eq!(lit, vec![true, true, true, true, false]);
    }

    #[test]
    fn the_chain_includes_what_was_built_on_the_selection() {
        let lit = chain(3, &[edge(0, 1), edge(1, 2)], 0);
        assert_eq!(lit, vec![true, true, true]);
    }

    #[test]
    fn a_selection_past_the_end_lights_nothing() {
        // A stale selection must not blank the view by lighting everything,
        // nor light a node that is not the one that was asked about.
        let lit = chain(2, &[edge(0, 1)], 9);
        assert_eq!(lit, vec![false, false]);
    }

    #[test]
    fn lane_members_share_an_x_band() {
        // Rank is the order itself, so the simulation is not allowed to
        // renegotiate it. A mark that wandered into the next column would draw
        // a derivation running backwards.
        let mut edges = Vec::new();
        for rank in 0..5usize {
            for row in 0..24usize {
                let index = rank * 24 + row;
                if rank + 1 < 5 {
                    edges.push(edge(index, index + 24));
                }
            }
        }
        let points = settled(120, &edges, 1_500);
        let ranks = levels(120, &edges);
        for (index, point) in points.iter().enumerate() {
            let expected = rank_x(ranks[index]);
            assert!(
                (point[0] - expected).abs() < RANK_PITCH * 0.5,
                "node {index} at x {} left column {expected}",
                point[0]
            );
        }
    }

    #[test]
    fn a_single_rank_still_places_every_node() {
        // Twenty members of one collection with no joins at all: one column,
        // and the springs have to spread them in y rather than pile them up.
        let points = settled(20, &[], 800);
        let mut ys: Vec<f32> = points.iter().map(|point| point[1]).collect();
        ys.sort_by(f32::total_cmp);
        for pair in ys.windows(2) {
            assert!(
                pair[1] - pair[0] > 2.0 * graph::MARK,
                "marks {} and {} overlap",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn auto_height_is_the_busiest_column_before_the_first_step() {
        // Three nodes must not reserve a screenful, and twenty thousand must
        // not reserve a mile of page.
        let unstepped = LayoutStats::default();
        let small = auto_height(&[0, 1, 2], unstepped, 768.0);
        assert!((small - (PITCH + TOP + BOTTOM)).abs() < 1e-3);
        let wide = auto_height(&[0, 0, 0, 1], unstepped, 768.0);
        assert!((wide - (3.0 * PITCH + TOP + BOTTOM)).abs() < 1e-3);
        let huge = auto_height(&vec![0usize; 20_000], unstepped, 768.0);
        assert!(huge <= MAX_ROWS * PITCH + TOP + BOTTOM);
    }

    #[test]
    fn auto_height_follows_the_layout_once_it_has_one() {
        // A card too short for the layout's proportions does not crop the
        // picture, it shrinks it — which is how a six-node lattice ended up
        // drawn at under three points a mark inside a 136-point strip.
        let square = LayoutStats {
            steps: 10,
            bounds: [0.0, 0.0, 800.0, 800.0],
            ..LayoutStats::default()
        };
        let tall = auto_height(&[0, 1], square, 768.0);
        let flat = LayoutStats {
            steps: 10,
            bounds: [0.0, 0.0, 2400.0, 240.0],
            ..LayoutStats::default()
        };
        let short = auto_height(&[0, 1], flat, 768.0);
        assert!(
            tall > short,
            "a square layout asked for {tall}, a wide one {short}"
        );
        assert!(
            tall <= MAX_ROWS * PITCH + TOP + BOTTOM,
            "unbounded at {tall}"
        );
        assert!(short >= PITCH + TOP + BOTTOM, "collapsed to {short}");
    }

    /// Count the shape kinds one mark emits, by actually driving egui.
    ///
    /// There was no render-level test of absence in either graph widget: they
    /// tested `arc_points`, `levels`, `chain` and `place`, so a clean render
    /// implied nothing at all about whether a hole would have been drawn. This
    /// is the smallest thing that fixes that — it asks the painter what shapes
    /// came out, rather than comparing pixels a settling layout cannot promise.
    fn mark_shapes(node: &LatticeNode) -> (usize, usize, usize, usize) {
        // A panel paints its own background, so the counts are taken against a
        // baseline frame that draws no mark at all. Otherwise every assertion
        // here is off by whatever the frame happened to contain, which is a
        // test that passes for the wrong reason the first time the frame
        // changes.
        let count = |node: Option<&LatticeNode>| {
            let ctx = eframe::egui::Context::default();
            let node = node.cloned();
            let output = ctx.run(Default::default(), |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    let painter = ui.painter().clone();
                    if let Some(node) = &node {
                        draw_mark(
                            &painter,
                            eframe::egui::pos2(60.0, 60.0),
                            1.0,
                            node,
                            Color32::WHITE,
                        );
                    }
                });
            });
            let mut kinds = [0usize; 4];
            for clipped in &output.shapes {
                match &clipped.shape {
                    eframe::egui::Shape::Circle(_) => kinds[0] += 1,
                    eframe::egui::Shape::Rect(_) => kinds[1] += 1,
                    eframe::egui::Shape::Path(_) => kinds[2] += 1,
                    eframe::egui::Shape::LineSegment { .. } => kinds[3] += 1,
                    _ => {}
                }
            }
            kinds
        };
        let base = count(None);
        let drawn = count(Some(node));
        (
            drawn[0] - base[0],
            drawn[1] - base[1],
            drawn[2] - base[2],
            drawn[3] - base[3],
        )
    }

    #[test]
    fn an_absent_member_emits_a_dashed_path() {
        // A hole is a dashed outline and a resident member is a solid mark.
        // Constraint made real: the render is asserted, not eyeballed.
        let present = LatticeNode {
            label: "here".into(),
            mark: LatticeMark::Authored,
            presence: LatticePresence::Present,
            ..LatticeNode::default()
        };
        let absent = LatticeNode {
            presence: LatticePresence::Absent,
            ..present.clone()
        };
        let (_, rects, _, _) = mark_shapes(&present);
        assert_eq!(rects, 1, "a resident authored member is one filled square");

        let (_, rects, _, segments) = mark_shapes(&absent);
        assert_eq!(rects, 0, "an absent member must not draw a solid square");
        assert!(
            segments >= 4,
            "a dashed square needs at least one dash per side, got {segments}"
        );
    }

    #[test]
    fn an_unproduced_member_is_not_pixel_identical_to_a_derive_output() {
        // This absence has never been seen on live data and is fixture-only,
        // which is exactly why it needs a test: a clean render must not be
        // allowed to imply the path works. Until now it had no geometry at
        // all — it was built as a resident computed member, which is the same
        // mark a fully-produced DERIVE output draws — and the only thing
        // saying otherwise was a count printed beside the picture.
        let produced = LatticeNode {
            label: "a".into(),
            mark: LatticeMark::Computed,
            ..LatticeNode::default()
        };
        let unproduced = LatticeNode {
            produced: false,
            ..produced.clone()
        };
        let (circles, _, paths, _) = mark_shapes(&produced);
        assert_eq!(circles, 1, "a produced output is a filled disc");
        assert_eq!(paths, 0);

        let (circles, _, paths, _) = mark_shapes(&unproduced);
        assert_eq!(
            circles, 0,
            "an unproduced member must not draw a filled disc"
        );
        assert_eq!(paths, 1, "it draws an arc with a gap at the bottom");
    }

    #[test]
    fn an_authored_member_is_never_drawn_open_below() {
        // Someone asserted it, so "nothing produced this" cannot be true of it.
        // The glyph table has to say so, or a caller passing a default could
        // make a commit look like a dangling join input.
        let node = LatticeNode {
            mark: LatticeMark::Authored,
            produced: false,
            ..LatticeNode::default()
        };
        let (circles, rects, paths, _) = mark_shapes(&node);
        assert_eq!((circles, rects, paths), (0, 1, 0));
    }

    /// Filled marks and outlined ones, counted apart.
    ///
    /// [`mark_shapes`] counts shape KINDS, and a filled disc and an outlined
    /// one are both `Shape::Circle` — so it cannot see the one distinction the
    /// admission channel is carried by. Same baseline subtraction, for the same
    /// reason: the panel paints a rect of its own.
    fn mark_fill(node: &LatticeNode) -> (usize, usize) {
        let count = |node: Option<&LatticeNode>| {
            let ctx = eframe::egui::Context::default();
            let node = node.cloned();
            let output = ctx.run(Default::default(), |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    let painter = ui.painter().clone();
                    if let Some(node) = &node {
                        draw_mark(
                            &painter,
                            eframe::egui::pos2(60.0, 60.0),
                            1.0,
                            node,
                            Color32::WHITE,
                        );
                    }
                });
            });
            let mut filled = 0usize;
            let mut outlined = 0usize;
            for clipped in &output.shapes {
                let (fill, width) = match &clipped.shape {
                    eframe::egui::Shape::Circle(circle) => (circle.fill, circle.stroke.width),
                    eframe::egui::Shape::Rect(rect) => (rect.fill, rect.stroke.width),
                    _ => continue,
                };
                if fill != Color32::TRANSPARENT {
                    filled += 1;
                }
                if width > 0.0 {
                    outlined += 1;
                }
            }
            [filled, outlined]
        };
        let base = count(None);
        let drawn = count(Some(node));
        (drawn[0] - base[0], drawn[1] - base[1])
    }

    #[test]
    fn a_member_waiting_on_a_grant_is_not_pixel_identical_to_an_admitted_one() {
        // The whole reason this channel exists: a parked COMMIT names its
        // payload exactly as an admitted one does, so before this the two drew
        // the same mark and the only thing saying otherwise was a count beside
        // the picture. Asserted on the geometry rather than the hue, because
        // the palette is capped below the text-contrast threshold and cannot
        // carry the fact by itself.
        let admitted = LatticeNode {
            label: "m".into(),
            mark: LatticeMark::Computed,
            admission: LatticeAdmission::Admitted,
            ..LatticeNode::default()
        };
        let unadmitted = LatticeNode {
            admission: LatticeAdmission::Unadmitted,
            ..admitted.clone()
        };

        assert_eq!(
            mark_fill(&admitted),
            (1, 0),
            "a vouched-for member is a solid disc"
        );
        assert_eq!(
            mark_fill(&unadmitted),
            (0, 1),
            "one waiting on a grant is an outline: here, and nothing vouches"
        );
    }

    #[test]
    fn admission_never_overrides_a_hole() {
        // A hole has no record to admit, so "waiting on a grant" would name a
        // remedy nobody can apply. Absence outranks admission, and both
        // admission answers must leave the dashes alone.
        let hole = LatticeNode {
            mark: LatticeMark::Authored,
            presence: LatticePresence::Absent,
            admission: LatticeAdmission::Unadmitted,
            ..LatticeNode::default()
        };
        let (_, rects, _, segments) = mark_shapes(&hole);
        assert_eq!(rects, 0, "an absent member must not draw a solid square");
        assert!(segments >= 4, "it stays a dashed square, got {segments}");

        let admitted = LatticeNode {
            admission: LatticeAdmission::Admitted,
            ..hole.clone()
        };
        assert_eq!(mark_shapes(&hole), mark_shapes(&admitted));
    }

    #[test]
    fn the_open_below_glyph_still_separates_its_two_strokes() {
        // This glyph cannot fill, so `Open` and `Filled` would otherwise draw
        // one picture and the distinction would fall back onto colour alone.
        // No caller reaches the combination today -- an attested member is
        // produced by the record attesting it -- which is exactly why the kit
        // must not depend on that staying true.
        let produced_by_nobody = LatticeNode {
            mark: LatticeMark::Computed,
            produced: false,
            ..LatticeNode::default()
        };
        let waiting = LatticeNode {
            admission: LatticeAdmission::Unadmitted,
            ..produced_by_nobody.clone()
        };
        let weight = |node: &LatticeNode| {
            let ctx = eframe::egui::Context::default();
            let node = node.clone();
            let output = ctx.run(Default::default(), |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    draw_mark(
                        &ui.painter().clone(),
                        eframe::egui::pos2(60.0, 60.0),
                        1.0,
                        &node,
                        Color32::WHITE,
                    );
                });
            });
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    eframe::egui::Shape::Path(path) => Some(path.stroke.width),
                    _ => None,
                })
                .expect("the open-below glyph draws one arc")
        };
        assert!(
            weight(&waiting) < weight(&produced_by_nobody),
            "it recedes by thinning, like a link: {} against {}",
            weight(&waiting),
            weight(&produced_by_nobody)
        );
    }

    #[test]
    fn an_unlooked_at_node_draws_exactly_what_it_drew_before() {
        // The default has to be the old picture, in both channels, or adding
        // this axis silently repaints every caller that has not adopted it.
        let node = LatticeNode::default();
        assert_eq!(node.admission, LatticeAdmission::Unknown);
        let ink = Color32::from_rgb(9, 9, 9);
        assert_eq!(mark_colour(node.admission, ink), ink);
        assert_eq!(
            mark_fill(&node),
            mark_fill(&LatticeNode {
                admission: LatticeAdmission::Unknown,
                ..LatticeNode::default()
            })
        );
    }

    #[test]
    fn a_single_column_is_sized_by_what_it_settled_to() {
        // Measured, not assumed: the rank spring pins a one-rank layout to an
        // x-span of exactly 0.00, so the aspect branch never applied and every
        // single-column lattice took the pre-step estimate instead -- one PITCH
        // a node against a settled spacing near five, which is a fifth-size
        // picture with no labels on it. A collection whose commits nothing has
        // joined yet is exactly this shape, so it was the common case.
        let settle = |count: usize| {
            let positions = settled(count, &[], 400);
            let ys: Vec<f32> = positions.iter().map(|point| point[1]).collect();
            let xs: Vec<f32> = positions.iter().map(|point| point[0]).collect();
            let low = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
            let high = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
            (
                high(&ys) - low(&ys),
                LayoutStats {
                    steps: 400,
                    bounds: [low(&xs), low(&ys), high(&xs), high(&ys)],
                    ..LayoutStats::default()
                },
            )
        };

        let (span, stats) = settle(2);
        assert_eq!(
            stats.bounds[2] - stats.bounds[0],
            0.0,
            "the premise: a single column has no width to take an aspect from"
        );
        let height = auto_height(&[0, 0], stats, 768.0);
        assert!(
            height >= span,
            "a column asked for {height} to draw {span} of world, which squashes it"
        );

        // Still bounded: a long column asks for a viewport, not a mile of page.
        let (_, many) = settle(10);
        let tall = auto_height(&vec![0; 10], many, 768.0);
        assert_eq!(tall, MAX_ROWS * PITCH + TOP + BOTTOM);

        // And a lattice with real width still follows its aspect rather than
        // its height, which is the case this function was written for.
        let wide = LayoutStats {
            steps: 10,
            bounds: [0.0, 0.0, 2400.0, 240.0],
            ..LayoutStats::default()
        };
        assert!(auto_height(&[0, 1], wide, 768.0) < tall);
    }

    #[test]
    fn a_label_never_wears_the_admission_hue() {
        // It did for one revision, and it is worth a test rather than a fix:
        // this palette caps any single value against either ground at 4.00:1,
        // under the 4.5:1 text threshold, so a label in the admission hue is a
        // label nobody can read. Marks are not text and may wear it; the names
        // under them may not.
        let nodes = vec![LatticeNode {
            label: "abcdef012345".into(),
            mark: LatticeMark::Computed,
            admission: LatticeAdmission::Admitted,
            ..LatticeNode::default()
        }];
        // egui's own default style, not the industrial one: the theme binds a
        // font family this test process never loads, and the assertion is
        // about which colour the label is handed, not which face draws it.
        let ctx = eframe::egui::Context::default();
        // Two passes: the layout settles from the first, and the label budget
        // is only spent once there is somewhere to put it.
        let mut text_colours = Vec::new();
        for _ in 0..2 {
            text_colours.clear();
            let output = ctx.run(Default::default(), |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(LatticeGraph::new(&nodes, &[]).height(400.0));
                });
            });
            for clipped in &output.shapes {
                if let eframe::egui::Shape::Text(text) = &clipped.shape {
                    text_colours.extend(
                        text.galley
                            .job
                            .sections
                            .iter()
                            .map(|section| section.format.color),
                    );
                }
            }
        }
        assert!(
            !text_colours.is_empty(),
            "the label budget should have drawn the one node's name"
        );
        assert!(
            !text_colours.contains(&themes::admitted_mark()),
            "a label was drawn in the admission hue: {text_colours:?}"
        );
    }

    #[test]
    fn overlays_draw_only_what_was_handed_and_never_move_the_lattice() {
        // Two nodes, three overlays: one real attachment, one at depth zero
        // (the lattice itself, which draws nothing), and one anchored past the
        // end (an attachment of a node this caller did not include). Only the
        // first may be drawn, and the lattice must draw the same marks with
        // and without it -- a layer is never a node of the order.
        let nodes = vec![
            LatticeNode {
                label: "low".into(),
                ..LatticeNode::default()
            },
            LatticeNode {
                label: "high".into(),
                ..LatticeNode::default()
            },
        ];
        let edges = [LatticeEdge {
            from: 0,
            to: 1,
            endorsed: true,
        }];
        let overlays = [
            LatticeOverlay {
                anchor: 1,
                depth: 1,
                presence: LatticePresence::Absent,
                admission: LatticeAdmission::Unknown,
            },
            LatticeOverlay {
                anchor: 0,
                depth: 0,
                presence: LatticePresence::Present,
                admission: LatticeAdmission::Unknown,
            },
            LatticeOverlay {
                anchor: 7,
                depth: 1,
                presence: LatticePresence::Present,
                admission: LatticeAdmission::Unknown,
            },
        ];
        let run = |overlays: &[LatticeOverlay]| {
            let ctx = eframe::egui::Context::default();
            let mut last = (0, 0, 0);
            for _ in 0..2 {
                ctx.run(Default::default(), |ctx| {
                    eframe::egui::CentralPanel::default().show(ctx, |ui| {
                        let drawn = LatticeGraph::new(&nodes, &edges)
                            .overlays(overlays)
                            .height(400.0)
                            .show(ui);
                        last = (drawn.drawn, drawn.folded, drawn.overlaid);
                    });
                });
            }
            last
        };
        let (drawn, folded, overlaid) = run(&overlays);
        assert_eq!(
            overlaid, 1,
            "only the in-range, nonzero-depth overlay draws"
        );
        let (bare_drawn, bare_folded, bare_overlaid) = run(&[]);
        assert_eq!(bare_overlaid, 0);
        assert_eq!(
            (drawn, folded),
            (bare_drawn, bare_folded),
            "an overlay changed which lattice marks were drawn"
        );
    }

    #[test]
    fn the_two_admission_hues_are_the_pair_the_palette_measured() {
        // Not an aesthetic choice: RAL 2005 against RAL 5012 is the widest
        // colourblind-safe separation in the table that also clears 3:1 on both
        // page poles in the mark role. Pinning it here means a later palette
        // edit has to come past this test rather than silently narrowing the
        // one axis a red deficiency keeps.
        let ink = Color32::from_rgb(9, 9, 9);
        assert_eq!(
            mark_colour(LatticeAdmission::Admitted, ink),
            themes::ral(5012)
        );
        assert_eq!(
            mark_colour(LatticeAdmission::Unadmitted, ink),
            themes::ral(2005)
        );
    }
}
