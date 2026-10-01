# Lissajous Widgets

## Feature flags

To keep the default build light, some widgets are feature-gated:
- `markdown`: rich Markdown rendering for `md!` and `note!` (default).
- `polars`: `lissajous::widgets::dataframe` (Polars + Lissajous table).
- `triblespace`: Triblespace widgets under `lissajous::widgets::triblespace`.
- `cubecl`: GPU simulated-annealing ordering for the entity inspector (use with `triblespace`).

Without `markdown`, `md!` and `note!` are unavailable. Disable defaults with
`default-features = false`.

`md!` renders Markdown inside a padded card. Use `lissajous::widgets::markdown`
when you want inline Markdown without padding.

## Sections and grids

`ctx.section(title, |ctx| ...)` is a top-level component of a card: a
collapsible region with a coloured header bar that runs the full card width.
Put the section at the top of the card body and nest the layout inside it,
usually a grid:

```rust
nb.view(|ctx| {
    ctx.section("Parameters", |ctx| {
        ctx.grid(|g| {
            g.place(6, |ctx| { ctx.number(&mut a); });
            g.place(6, |ctx| { ctx.number(&mut b); });
        });
    });
});
```

A section may also wrap one monolithic full-width widget instead of a grid
(a map, a globe, a large table) when the content should not carry the grid's
margins.

Two rules follow from what a section and a card are:

- Never place a section inside a grid cell. A cell insets its content, so
  the header bar gets a border and a margin it is not designed to have; the
  header is meant to touch the card's edges. Grids go inside sections, not
  the other way round.
- One section per card. Every `nb.view` is a cell the reader can detach
  from the notebook and move around the workspace; several sections in one
  cell can only move together. Split a card with several sections into one
  card per section. The section titles carry their own RAL colours, so the
  split costs nothing visually.

## Text fields

`lissajous::widgets::TextField` supports `rows()` to set the minimum visible height
for multiline inputs and `max_rows()` to cap it. When `max_rows` is set, extra
lines are clipped rather than expanding the widget.

## Headless capture

Notebooks can export cards to PNGs without opening the interactive window.
Run with `--headless` to render each card to `card_0001.png`, `card_0002.png`, ...
in `./gorbie_capture` by default. Override the output directory with `--out-dir`.
Rendering runs fully offscreen (no window is created). Use `--scale` to control the
pixels-per-point (default: 2.0).

## Physics snapshots

`widgets::PhysicsView` holds only orbit/pan/zoom state. Call
`view.show(ui, &PhysicsScene)` with owned sampled geometry. `PhysicsScene`
contains `Line3`, `Particle3`, `Label3`, `LegendEntry`, warnings and an explicit
length-unit label. `view.bounds(Bounds3 { min, max })` supplies a fixed camera
envelope; otherwise the first finite scene is fitted once. Fit/Reset never
advance the simulation. The scale bar is world-unit-aware; wireframes are
intentional x-ray overlays over depth-sorted, true-radius particle disks.

The generic viewer has no physics dependencies. Feature `rapier` adds
`widgets::physics::rapier::scene(&RigidBodySet, &ColliderSet)`; `salva` adds
`widgets::physics::salva::scene(&LiquidWorld)` and `fluid_scene(&Fluid)`.
Combine snapshots with `scene.extend(other_scene)`; different declared units
produce a warning, not a silent conversion. Unsupported Rapier shapes produce
explicit AABB approximations/warnings. Salva boundary points are not included
as fluid. Capture at your chosen real simulation timestep, outside the widget.
See `examples/physics_widgets.rs` and the README for feature commands and limits.

## Triblespace entity inspector

`lissajous::widgets::triblespace::EntityInspectorWidget` renders an entity graph from a
data `TribleSet` and uses a metadata `TribleSet` for attribute labels and value
formatters. Pass both sets, a `BlobCache` for `UTF8String` attribute names, a
`BlobCache` for `WasmCode` value formatters, and a mutable selection `Id`. The
widget caches its internal graph and will reset the selection to the first
entity if the current `Id` is not present in the data.
Use `EntityOrder::Id` for a stable, deterministic ordering. With the `cubecl`
feature, `EntityOrder::Anneal` runs a GPU simulated annealing pass (starting
from ID order) and updates the layout as it improves. If SA is unavailable, the
widget stays on `EntityOrder::Id`.

## Dataframes (polars)

Enable the `polars` feature to access dataframe widgets.

Core widgets:
- `dataframe(ui, df) -> Result<DataFrame, String>`: interactive SQL + sortable table. Uses
  per-card temp state and returns the query result (or the query error).
- `dataframe_summary(ui, df)`: per-column nulls + quick stats table.
- `data_summary_tiny(ui, active_df, total_df)`: compact “rows × columns” line. Pass the same
  dataframe twice for the unfiltered view.
- `data_export_tiny(ui, df)`: copy/save CSV for a dataframe (typically the active filtered view).

Response:
- Store the `Result` in your own state if you want other cards to react to the current query.

## Graph views: the visual language

`MeshGraph`, `LatticeGraph` and the wiki viewer share one solver and one drawing
kit (`src/graph/`). This section is the **visual** contract that sits on top of
that kit: what the chrome around a graph is, what each channel is allowed to
carry, and which devices are deliberately scarce. It is a sibling of the web
system in `gorbie-system/gorbie.css`, which is itself a port of `src/themes.rs`
— the same tokens, the same derivations, the same reasons.

### An instrument, not a chart

A graph drawn into a bare rect is a chart. The site's register is an instrument:
a measured object inside a frame that states what it measured. So a graph view
wears the same furniture the pages do, and **every part of it reads a real
value** — that is the condition on which the measurement decals were restored to
the site at all, after a first version that was a tick rail measuring nothing.

| device | on the page | in a graph view | reads |
|---|---|---|---|
| bezel rail | section measure bar | left of the viewport, 30pt | fine ticks at the 12pt module, heavy ticks at the 144pt réseau pitch |
| rail designation | `.bezel-id` | `M·01`, `L·02`, `J·03` + the view's name, vertical | which view this is |
| datum arrows | `.bezel-rail::before/after` | rail top and bottom, accent | closes the vertical dimension |
| vertical readout | `.bezel-dim` | rail foot, accent | the world-space height on screen |
| horizontal callout | `.dim-x` | under the viewport, accent | the world-space width, the réseau decimation, the zoom |
| registration brackets | `.frame` | four corners, 14pt, accent | nothing — they mark the *primary* view |
| spec strip | `.strip` | under the callout | five counts read off the data |
| phosphor well | `.lcd` | viewport corner | the solver's own state |

**Scarcity is part of the look, not a caveat on it.** JP: *"you don't have to put
them everywhere, but I like the technical look"*; the site's own comment says an
annotation that appears on everything annotates nothing. So:

* The **primary** view in a dashboard gets the full instrument.
* A **secondary** view gets `bezel-rail--light`: fine ticks and a name, no datum
  arrows, no readout, no brackets. It is an annotation, not a measurement.
* A view inside a notebook card gets the rail only.

The rail carries a name *and* a dimension, and in a short viewport they collide.
The dimension yields: it is the annotation, the name is the label.

### The réseau is in world space

`paint_reseau_grid` paints the page margin at a fixed pitch. Inside a graph
viewport the same field belongs in **world coordinates**, so it pans and zooms
with the graph. That turns texture into instrumentation: count the crosses and
you have the distance, watch them crowd and you have the zoom. A dot field says
"you may draw here"; a réseau says "this has been measured" — and here it
finally does measure.

Decimate by powers of two whenever the on-screen pitch would fall below 48pt, so
the field stays a real logarithmic scale bar instead of collapsing into grey
mush at the far end of the zoom range. Report the factor in the horizontal
callout (`RÉSEAU ×1`, `×2`, `×4`), because a scale bar whose scale is unstated
is not a scale bar. Colour stays `blend(bg, border, 0.28)` — a cross carries more
ink than a dot and has to sit further back to stay quiet.

### Four channels, and colour is not one of them

The palette is a true inversion 16.03:1 across, which caps any single value at
`sqrt(16.03) = 4.00:1` against both grounds — below the 4.5:1 text threshold.
Colour here **cannot** carry a fact on its own even when it looks like it does.
So every fact rides on geometry:

| channel | carries |
|---|---|
| **glyph** | square = authored/heard · circle = computed · slash = an attempt that failed |
| **stroke** | filled = whole · open = present but qualified · dashed = a hole |
| **track** | a ratio, as an arc, plus a datum tick that says a measurement exists |
| **position** | rank, in the lattice views; force equilibrium, in the mesh |

The accent (RAL 2005) is spent on at most three things, in this order:

1. **What you asked about** — the ring on the selected or hovered node.
2. **Direction** — link barbs, *only* on the links of that node. A barb on every
   link puts twenty accent marks in one field, which reads as measles rather
   than as scarce saturation; and at rest nobody is asking about direction.
3. **Datum marks** — the rail arrows and the two dimension readouts, which
   measure the *frame* and not the data.

Nothing else. No status colours on nodes: the status palette is fixed, must
always be paired with an icon and a label, and a coloured node is a node made
harder to read. Teal is reserved for inside the phosphor well, which is the one
lit surface and therefore the one legitimate place for a scanline.

### A measured zero is not a missing measurement

`draw_track` returns early both for `None` and for `Some(0.0)`, so a node that
was successfully observed and reported exactly zero renders **identically** to a
node nobody has measured. That is the one thing these views exist to prevent.

Draw a **datum tick** at the track's 12 o'clock origin whenever a measurement
exists, of any value including zero; omit it when there is none. Put it strictly
*outside* the track: drawn across it, it lands exactly where the arc begins and
ends, so a 94% ring reads as a closed ring with a tick rather than as six per
cent short — the tick fills in the gap it exists to let you see. Outside, it is
an index mark beside the gauge, and the gap beneath it is the shortfall.

Pair it with digits: `0%` for the measured zero, `—` for the absent measurement.

### Percentages beside the arcs

An arc is a bar in polar coordinates, and JP asked for percentages over bars
explicitly, so the numbers stay readable by a machine as well as by an eye. The
arc is for the glance across many nodes and is the channel that survives zooming
out; the digits are the read. Draw both at `Lod::Full`, the arc alone below it.

Where the layout has columns, **align the percentages into one** and let
complete values recede to `--weak` while everything short of complete stays at
`--fg`. An aligned column of digits makes the incomplete rows findable without a
single arc, which is why the derivation forest draws no tracks at all: beside
such a column a 7pt arc says nothing the number does not.

### The mesh's states are two facts, not one

The old widget collapsed *never observed*, *observed but stale* and *observed as
exactly zero* into one glyph, under a legend that said "connected but stale".
They are two orthogonal facts and want two channels. The vocabulary is the
producer's: absent stays absent, stale is freshness metadata, `None` on failed
or unobserved scans, `Some(0)` only after a successful exact observation.

| state | glyph | stroke | track |
|---|---|---|---|
| heard, fresh | square | filled | tick + arc + `n%` |
| heard, stale | square | open | tick + arc + `n%` |
| never heard — named only by a peer | square | dashed | bare, `—` |
| scan attempted and failed | square + slash | dashed | bare, `—` |

Freshness qualifies a report, so it can only apply where there *is* one; that is
why it lives on the stroke of the two heard states and nowhere else.

Then draw an **observation census** beneath the graph: one zebra row per state,
with its count, its share and the handles. JP asked whether slashed daemons were
down or just quiet and could not tell from the picture — that is a *roster*
question, and a graph is a bad roster. The answer is not a cleverer glyph, it is
the table the system already owns. It is the legend, the roster and the count in
one device, it needs no legend lookup, and it survives being read by a machine.

### What each view becomes when it is folded

Prettiness that hides state is a regression, and a hairball is prettiness that
hides state. Each view has an honest aggregate form, and it is a *different
instrument* rather than a blurrier version of the same one:

* **Derivation forest** — chains of depth ≤ 2 are not a graph problem, they are
  a table. Draw one zebra row per chain with rank as the column, so "everything
  at depth 2" is one glance rather than a traversal, and start every link in a
  rank at the same x so the links read as the grid the rest of the system is
  built on. Collections with no derivation fold into a single `Glyph::Aggregate`
  that **states its own count** — folded, not hidden.
* **Join semilattice** — tens of thousands of results with two parents each. At
  `Lod::Aggregate` fold to the **merge-depth profile**: one aggregate mark per
  rank, area proportional to the count, each stating its count and share. The
  decay rate is a real diagnostic a hairball cannot give — a balanced schedule
  halves at each step, a degenerate one does not. Keep the aggregates on the
  same rank axis the members occupy, so magnifying unfolds them in place: one
  picture at two scales, not two views.
* `Glyph::Aggregate` takes a fixed inner mark, not one scaled with the ring. The
  inner dot is the rank point where members unfold; scaling it with the ring
  turns a row of aggregates into a row of eyeballs.

### Absence is fixture-only, and must be labelled so

Non-resident descriptors and declared-but-unperformed derivations are the point
of the lattice view, and that path has never been exercised on live data — every
pile we own reports "0 with no resident descriptor". A clean render must not be
allowed to imply the path works. Keep a fixture that carries at least one
absent member and one unendorsed edge, render it, and let the spec strip say
where the numbers came from (`SOURCE FIXTURE`).

One rendering note that is easy to get wrong: a dashed circle needs its dash
phase carried **across** the polyline segments that approximate it. `arc_points`
emits 64 short segments; restarting the pattern on each draws a full dash every
time and the ring comes out solid, silently turning every absence mark back into
an ordinary open one. `Shape::dashed_line` does carry the phase — any
reimplementation must too.

### Scale: what a thousand peers actually looks like

A clean picture of twelve nodes says nothing about a thousand, so the design was
rendered against a 1000-peer fixture settled with a real force solver before any
of the rules below were written down.

**Level of detail decimates the NORM, never the EXCEPTION.** A level of detail
that folds everything equally is a blur, and a blur of a thousand healthy peers
hides the eleven that are not — which are the only reason anyone opened the
view. So the fresh majority collapses to one dot per node (and, lower still,
into folded screen cells), while every node in an exceptional state keeps its
full mark and a small knockout of the page ground beneath it, so it stays
legible in the densest part of the field. The cost is bounded by how bad things
are rather than by how big the colony is: exceptions are rare by definition, and
a colony where that stops being true is a colony whose legend is telling you so.

Rendering the fixture made the payoff obvious and it was not the one expected:
failures **cluster by site**. Fifty-three exceptional marks scattered over a
folded field show at a glance which clusters are in trouble, which is a spatial
fact a roster cannot give and a hairball cannot either.

**The census is the LOD-invariant channel.** Four rows whether the colony is
twelve peers or ten thousand. Everything else in the view degrades as the zoom
goes out — the label first, then the track, then the glyph — and the census does
not, which is why it belongs under every mesh rather than only the small ones.
Past a few dozen peers it lists counts and shares instead of handles.

**Links recede with the level of detail.** At `Lod::Full` a link is a fact you
can follow; by `Lod::Marks` it is one of thousands and what it contributes is
density, so it has to step back until the marks read on top of it. Two and a
half thousand links at full weight are a grey wash that drowns every node in the
field, and a wash that drowns the nodes has stopped being information.

**But recede toward the RÉSEAU, never toward the ground — the réseau is the
floor of the visual stack, and nothing carrying data may sink to or below the
field that measures it.** Receding toward the ground walks straight through it.
Measured on the bone page: a 45%-receded wire lands at `#d2d3cc` against a
réseau of `#d5d7cf`, a **1.05:1** difference, and by `Lod::Dots` the link is
*lighter* than the réseau — the data has gone behind the instrument. Both poles
do it, because the inversion is faithful: on graphite the same link recedes to
`#383937` against a réseau of `#3e3e3c`. Blending from the réseau toward the
link colour instead, with a floor of 0.35, holds every band at 1.16–1.43 against
the field and always on the forward side. Three layers, always in this order:
réseau behind, links between, marks and labels in front.

**Report what was DRAWN, not only what exists.** When the view folds 947 marks
into 158, the spec strip says so (`DRAWN 211`). A view that folds and does not
disclose it is a view you cannot trust the next time it looks sparse.

Two measured findings, recorded because both were assumptions until they were
rendered:

* A 1000-peer colony fitted to a dashboard panel lands in **`Lod::Dots`, not
  `Lod::Aggregate`** — mark ≈ 1.8pt at the fit zoom. The aggregate band is
  reached by zooming out past the fit, or by a colony roughly an order of
  magnitude larger. So at the size this system is actually asked for, every node
  still gets a dot and the exceptions still get a glyph; what has already been
  lost by then is the track and the label.
* The réseau's decimation is what keeps the field readable across that range:
  ×1 at the twelve-node zoom, ×2 at the thousand-node fit, ×4 pulled back. State
  the factor in the horizontal callout at every level.

#### One implementation hazard, for whoever tunes the solver

A repulsion with a **distance cutoff and no long-range term collapses every
cluster into one blob.** Two sites further apart than the cutoff exert nothing
on each other while the inter-site springs pull without opposition, so the
layout converges to a single amorphous mass — and revealing latent structure is
the entire reason a force layout is being paid for. The fixture did exactly this
on the first run: fourteen sites, one blob, no structure visible at any zoom.
Adding a single coarse Barnes-Hut level (each cell's centre of mass repelling
every node outside its own neighbourhood) separated all fourteen at the same
cost class. An O(n²) GPU pass does not have this problem; anything that buckets
or tiles for speed does, and it fails silently — the picture still looks like a
graph, it just no longer means anything.
