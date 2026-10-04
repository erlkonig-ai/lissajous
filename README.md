![Discord Shield](https://discordapp.com/api/guilds/795317845181464651/widget.png?style=shield)

# Lissajous - A Minimalist Notebook Environment for Rust

Lissajous is the notebook library formerly named GORBIE. Gorbie remains our
mascot. Visit [lissajous.science](https://lissajous.science) or the
[source repository](https://github.com/erlkonig-ai/lissajous).

Every other notebook environment tries to make notebooks easier, we try to make them simpler.

![Lissajous screenshot](https://github.com/erlkonig-ai/lissajous/blob/main/assets/screenshot.png?raw=true)

## Core Ideas

A notebook is just Rust. By being fully native you can visualize huge datasets,
build complex UIs, and leverage the entire Rust ecosystem without being forced to
shoehorn everything into a web browser, JavaScript and serialized JSON.

This is a library, not a server. Your notebook lives in your Rust project,
runs in-process with your existing dependencies. No separate server, no custom
file format, no sync step - just Rust and an egui window when you want it.

We don't ship yet another editor. Most developers already have a
well-tuned setup, and notebook tools often spend time re-inventing the wheel
with worse results. We focus on the notebook experience and plug into the
tools you already use.

Compose a notebook from cells that own their internal state and expose useful
values to other cells, in the spirit of Observable and marimo. A map can publish
a selection; a plot can consume that selection without knowing about the map's
camera, gestures or drawing cache. The notebook body wires those dependencies
together; it need not become a central application controller.

This reactive composition is distinct from the rendering mechanism. Lissajous
uses immediate-mode egui: the notebook and card drawing code run again on
repaint, while keyed state survives. Dependencies and recomputation are explicit
Rust, not a runtime source analyzer or an automatic dependency scheduler. See
[Authoring notebooks](#authoring-notebooks) before building a larger notebook.

Interactive development stays simple: we re-run the notebook on each change,
not hot-reload. Rust's incremental compilation keeps that fast enough to feel
live.

# Getting Started
For development, use a normal Cargo project so your IDE can index Lissajous and
provide full static analysis.

Add the dependency and drop in a `main`:

```toml
# Cargo.toml
[dependencies]
lissajous = "0.19"
```

Existing consumers can keep their `GORBIE::` Rust imports by using an explicit
Cargo package alias instead:

```toml
GORBIE = { package = "lissajous", version = "0.19" }
```

The notebook macro resolves either dependency name. The companion proc-macro
crate is `lissajous-macros` 0.10, normally re-exported by `lissajous` rather
than added directly. This rename does not make the removed pre-0.19 APIs
compatible; see the [changelog](CHANGELOG.md).

The example below uses the normal `lissajous` dependency name.

```rust
// src/main.rs
use lissajous::prelude::*;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.view(|ctx| {
        md!(ctx, "# Lissajous\nA _minimalist_ notebook environment for **Rust**.");
    });

    let slider = nb.state("slider", || 0.5, |ctx, value| {
        ctx.grid(|g| {
            g.two_thirds(|ctx| {
                ctx.slider(value, 0.0..=1.0);
            });
            g.third(|ctx| {
                ctx.number(value);
            });
        });
    });

    nb.view(move |ctx| {
        let value = *slider.read(ctx);
        ctx.progress(value);
    });
}
```

Run it with `cargo run` to start the notebook.

`nb.state("key", || expensive(), |ctx, value| { /* ... */ })` invokes its
initializer only while that state key is absent. Put construction inside the
closure (or pass a constructor such as `Camera::default`), so rebuilding the
notebook does not reconstruct a retained resource. There is no eager-value
overload or `state_with` alias.
This initializer-only signature is currently an unreleased source change;
published 0.19.1 documentation still describes the earlier eager signature.

For reload-on-change with Cargo, use:
`watchexec -r -w src -w Cargo.toml -- cargo run`
or `cargo watch -x run` (install with `cargo install cargo-watch`).

## Authoring notebooks

### Private machinery, useful public values

Treat each instrument as a cell with a small interface. Keep a map's camera,
gesture state, prepared geometry and pending map queries with the map. Publish
the selected entity or area. A plot owns its axes, display options and pending
series query; its input is the selection, not the map controller. Another
selector should be able to supply that same input without rewriting the plot.

Use `nb.state` for retained cell state, especially resources with nontrivial
construction. Its initializer is lazy, not asynchronous or dependency-reactive:
it runs only when the key is absent, on the calling thread. Starting a worker
there can be appropriate; opening a large dataset or doing slow work there can
still block the UI. Do not read another `StateId` inside the initializer:
insertion holds the state store's write lock. Read dependencies in the notebook
body or draw callback instead. The returned `StateId<T>` is a typed handle, not a subscription or a
read-only capability. Share handles deliberately, and read or copy only the
inputs a consumer needs. Small copied outputs and narrow Rust interfaces are
also useful; not every output needs its own visible card.

Avoid one `AppState` containing every instrument and a central `refresh()` that
rebuilds everything. Splitting that object into nested `map`, `plot` and `table`
fields does not change the architecture if every cell still receives the whole
controller and depends on its orchestration. A small composition root connecting
independent cells is different from a god object. Legitimate coordinated edits
through several typed handles remain supported; see
[`multi_state`](examples/multi_state.rs). Keep lock scopes short, use a consistent
order when taking several locks, and never reacquire a cell's own write-locked
state through its handle from inside that cell's callback.

### Worked example: map selection, independent consumer

Run `cargo run --example geospatial_map` and read
[`examples/geospatial_map.rs`](examples/geospatial_map.rs). It contains three
small cells:

```text
selection: StateId<Option<u64>>  <---  map's private camera + prepared geometry
             |
             +---> independent selection summary
```

The visible selection card shows the public value and offers a clear button.
The map receives that handle, keeps its own `MapState` private inside a module,
and publishes only selection changes. The summary accepts the same typed
selection handle; it never reads `MapState`. A lazy, memoized expression computes
a small label only when the selection changes, without a separate cache card.
Replacing the map with a list
selector leaves the summary unchanged. The geometry and IDs are synthetic;
there is no dataset reader, network service or domain-specific backend hidden
in the example.

This is explicit value flow, not a promise of topological or atomic evaluation
of all cells. Paint order can affect which repaint first observes an edit. The
example requests a repaint after publishing a selection so an earlier card can
also display it. Detaching or redocking a card changes its placement, not its
state ownership, inputs or resource lifetime.

### Choose recomputation explicitly

An initializer does not rerun when another cell changes. A stateless `view`
reads a `StateId` each paint; a stateful consumer can read the same handle while
retaining its own axes, cache and pending tasks. Read or copy the input, release
its guard, then decide what to recompute.

For ordinary value flow, `ReadValue` gives state handles and lazy recipes a
common `.read(ctx)` interface. A handle returns a read guard; a recipe returns
an owned `Arc`. `.map` owns its inputs and closure but does no work until read:

```rust
use lissajous::prelude::*;

fn expensive_calculation(left: &u32, right: &u32) -> u32 {
    left + right // Stand-in for your synchronous calculation.
}

fn totals(nb: &mut NotebookCtx, left: StateId<u32>, right: StateId<u32>) {
    let total = (left, right).map(expensive_calculation).memo("total");
    // Inspect an intermediate using a normal view with Debug formatting.
    nb.view(total.tap());
    let label = total.map(|total| format!("Downstream: {total}"));
    nb.view(move |ctx| {
        let text = label.read(ctx);
        ctx.label(text.as_str());
    });
}
```

Reading the intermediate forces only its dependencies, not downstream recipes.
The downstream read reuses `total`'s memoized result. `.tap()` owns the recipe
but reads only when its view draws; it adds no caching of its own. The named
function recipe above is `Copy`; use `total.clone().tap()` for a non-`Copy`,
cloneable recipe that you also want to consume downstream. Use a single handle's
`.map(|value| ...)`, or tuples of one to eight readable inputs with separate
borrowed arguments. Chains receive the final read's context throughout; handles
and recipes retain no notebook context. Recipes are `Copy` when their inputs
and closure are, but captured closures need not be `Copy`. Ordinary `.map`
computes on **every** read, without cloning source state or requiring `Clone`.

`.memo(stable_key)` opts a mapping into a store-backed `DerivedState` cache.
Rebuilding the recipe each frame reuses one slot in that notebook's state store,
independent of which card reads it; it creates no visible card. The stable key
names the computation, while its current input values determine validity. Use
distinct stable keys for distinct computations, even when their types match.
Reusing a key for a different computation is a programming error, not automatic
invalidation. Changing settings belong in the inputs, not a continually changing
stable key, which would create additional retained slots. A hit skips the
wrapped mapping closure, **not** input resolution or upstream maps. Memoize an
expensive upstream mapping separately if needed.

Only memoization requires cloning its explicit input values (which must also
be `PartialEq + Send + Sync + 'static`); outputs must be `Send + Sync + 'static`.
Project a small selection or revision out of large state before the expensive
memoized mapping. Cloning the whole instrument just to read it is unnecessary.
Every changing parameter must be an explicit input: replacing a closure or
changing its captures does not invalidate a memo. For example, make a changing
multiplier another input, `(value, multiplier).map(|v, m| v * m)`, rather than
capturing it. Immutable source captures need an explicit revision dependency
when their identity changes.

All of this is synchronous pull evaluation, not topological scheduling or an
atomic multi-state snapshot. Input guards are released before memo cache locks
are acquired; ordinary maps borrow their inputs during computation. Keep lock
orders consistent, never read a write-locked cell's own handle in its callback,
and never recursively read a memo's own slot from its computation. Heavy work
still belongs off the paint path. The map example's summary now uses this API.

`DerivedState<K, T>::get(key, compute)` is a synchronous, single-current-key memo.
Use it for inexpensive derivations, not heavy I/O or decoding on the paint path.
Its key must account for every relevant input: selection, filter, source or
snapshot identity, and any other parameters. Read changing parameters from the
closure's key; a source revision can represent the corresponding immutable
source. Returning to an older key recomputes; there is no multi-key result cache.

`ComputedState<T>` offers a background result slot on native targets, but it is
not keyed computation or a latest-request queue. Call `poll`, and call `spawn`
only when your input requires new work. While a task is running, `spawn` does
not enqueue a replacement; repeatedly calling it after completion starts new
work. `set` drops the tracked join handle rather than cooperatively cancelling
the running thread. On wasm, `spawn` is synchronous. Choose another suitable
worker/executor when those properties do not fit the resource or platform.

For asynchronous consumers, capture an immutable input and carry its exact
request identity back with the answer. Compare that identity with the current
input before presenting the result; include a generation when a refresh or an
A → B → A change must invalidate an earlier A. Keep the latest desired request
when a bounded worker is busy, and submit it when capacity returns. Decide
explicitly whether an old answer remains visible, labelled as old, or is cleared
while waiting. Wake the UI on completion and keep an appropriate polling/repaint
path while work is pending. These are application responsibilities, not behavior
inferred by reading a `StateId`.

### Share sources, not controllers

Cell ownership does not mean one database connection, file reader or worker per
cell. Retain one appropriate resource session at the source boundary and share
its narrow read interface or immutable snapshots with consumers. Cells own their
questions and bounded answers; the source owns I/O and observation lifetime.
For a native pile, reuse its retained session rather than reopening the same
pile for each card. The same principle applies to a database, a simulation or
an in-memory dataset. Do not copy an entire source into a second application
catalogue merely to connect cells. Keep slow acquisition and computation off
paint, and preserve the source identity and authority relevant to each answer.

API details: [current source `NotebookCtx::state`](src/lib.rs),
[`StateId`](https://docs.rs/lissajous/0.19.1/lissajous/state/struct.StateId.html),
[`DerivedState`](https://docs.rs/lissajous/0.19.1/lissajous/dataflow/struct.DerivedState.html),
and [`ComputedState`](https://docs.rs/lissajous/0.19.1/lissajous/dataflow/struct.ComputedState.html).

### Native pile resource (source integration)

With the `triblespace-pile` feature on Unix, `widgets::triblespace::pile::PileCell`
owns one read-only native reader and publishes `Arc<PileSnapshot>`. See
[`examples/pile_resource.rs`](examples/pile_resource.rs): one cell opens and
refreshes the source; an independent consumer copies its immutable snapshot.
The resource displays only opening, byte replay, snapshot, ready or failed
status. Collection selection, query tasks, result caches and query errors belong
to consumers, not this cell. Share one cell for consumers of the same source;
detaching a card does not reopen its pile.

The compact face shows the configured path, a replay-watermark tick and measured
bytes inside one rail; hatched space is unread or unknown, not simulated progress.
Byte labels use decimal units (GB), with exact byte counts and the full path on
hover. `PileProgress::new(path, progress).error(error).refreshable(false)` renders
the same data-only face during a caller's preflight, without opening a resource
or advertising a refresh action that caller cannot perform.

Use `--no-default-features --features triblespace-pile` for this example's lean
graph: native storage and parallel queries, without the facade's GPU, WASM or
object-store defaults. The existing `triblespace` feature also includes the pile
cell and retains WASM value formatting for the entity inspector.

This integration currently requires the unpublished Core `refresh_next` API,
reviewed at `3dd8930e5c97db1b319ea4d7f6e262eca559a8c2`. A registry dependency
version alone does **not** establish that API is present. Use an exact-source
Cargo override for the matching TribleSpace graph before building this example;
this is not a claim that the registry release supports it.

`PileCell::new` starts its background owner without reading the file on paint.
`read()` is a nonblocking latest-value read; `refresh()` coalesces a request;
`wait()` is only for explicitly blocking headless/capture preparation. The owner
notices file growth while idle. A final native `snapshot()` bulk-refreshes, so
100% of a sampled byte total is not snapshot readiness, index coverage or query
completion. Missing files and native replay errors remain errors, never an
empty successful dataset or an automatic repair.

An output's `Published.observation` identifies its successful read. During a
refresh or same-file failure, that output may remain the labelled last success;
`Read.observation` and its status describe the newer resource attempt. These
tokens are local to one cell, not portable versions. Include source identity and
the consumer's exact inputs in asynchronous result checks. The public host is
paired with the snapshot but is not itself a grant of READ authority. Changing
path or host requires replacing the resource (for example with a distinct state
key); never quietly relabel an old snapshot.

Atomic file replacement is supported. In-place edits or truncation violate the
native mapped-file contract and are not repaired by reopening. Drop signals
shutdown without waiting on a native file lock; published snapshots keep their
own backing alive.

## Script Workflow (Quick/Share)
If you want a single-file notebook or quick distribution, use
[`watchexec`](https://github.com/watchexec/watchexec) and
[`rust-script`](https://github.com/fornwall/rust-script). It skips IDE support,
but it is handy for sharing.

Install them with `cargo install watchexec-cli rust-script`, then add this
header to `notebook.rs` and paste the same `main` function below it:

```rust
#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = "0.19"
//! ```
```

Make the file executable once with `chmod +x notebook.rs`.

Run it with `./notebook.rs` to load dependencies, start the notebook, and
reload on save.

The first run can take a while because Rust needs to compile and cache
dependencies - grab a coffee. Subsequent launches are fast enough that we use
them for interactive editing.

# Editor Integration
Lissajous does not ship an editor, but it can jump to card sources. Set
`GORBIE_EDITOR` to a command with placeholders `{{file}}`, `{{line}}`, and
`{{column}}`, for example
`GORBIE_EDITOR='code -g {{file}}:{{line}}:{{column}}'` for VS Code. When set, cards show
an open-in-editor tab.

`GORBIE_EDITOR`, the `gorbie_capture` output default, and existing `Gorbie*`
style type names remain unchanged by the package rename.

# Examples
See `examples/` for larger notebooks and patterns. Most are runnable with
the same `watchexec` + `rust-script` shebang.

For cargo examples:
`cargo run --example grid_demo --features typst`
`cargo run --example polars --features polars`
`cargo run --example entity_inspector --features triblespace`
`cargo run --example spatial_pile_resolver --features triblespace`

## Typst Integration

Enable the `typst` feature for math and scientific typesetting:

```toml
lissajous = { version = "0.19", features = ["typst"] }
```

```rust
nb.view(|ctx| {
    typst!(ctx, "= Euler's Identity\n$ e^(i pi) + 1 = 0 $");
});
```

Typst content renders as vector geometry — sharp at any zoom level, with
text selection, copy, and double-click support. The Lissajous grid constants
(`grid-span`, `grid-gutter`, etc.) are available in the Typst preamble for
grid-aligned column layouts. Compilation errors render inline as rustc-style
diagnostics with source context and hints.

# Located observations

`widgets::MapView` displays bounded, borrowed WGS84 point/path query results.
Keep `MapCamera` and `Option<Id>` in notebook state; the caller's opaque entity
IDs come back on hover/click without a database, parser or network layer inside
the widget. Pan and zoom survive refreshed data. The fit button is explicit.

```rust,ignore
let result = MapView::new(&features, &mut camera, &mut selected)
    .legend(&legend)
    .height(360.0)
    .show(ui);
if let Some(entity) = result.clicked {
    // Query this entity's observations/provenance in the caller.
}
```

Longitude/latitude are degrees in EPSG:4326. Display uses Web Mercator; invalid
coordinates and latitudes beyond ±85.05112878° are skipped and counted rather
than relocated. Paths follow the shorter longitude arc, including across the
dateline, and invalid vertices break them. The scale bar is an approximate local
ground distance at the view centre, not an area-preserving measurement.

The tile-free canvas shows supplied geometry only: blank is not a land/water
classification. No satellite imagery or basemap is fetched. A future raster
layer must declare its projected/georeferenced extent; a GeoJSON outline alone
is not a georeferenced image. `geospatial_map` is a synthetic example, not a
real measurement fixture.

# Headless capture

Native callers can use `NotebookConfig::capture(options, body, emit)` to receive
resident PNGs without creating a directory or reopening exported files.
`CaptureOptions` supplies scale and per-layout settle timeout; each `CapturedPng`
contains card/tile indices, dimensions and encoded bytes. Delivery is ordered
and stops at the first renderer, encoder or consumer error without retrying
accepted images. File capture uses the same rendering/encoding path and
`CapturedPng::filename()` conventions. Both paths default to a deterministic
dark theme; use `with_headless_theme` to choose light or explicitly opt into
desktop detection. Capture is not a GPU-memory quota or an overall rendering
deadline.

To export cards without opening an interactive notebook, pass `--headless`. Each card
is rendered to a PNG and saved as `card_0001.png`, `card_0002.png`, ... in the output
directory (default: `./gorbie_capture`). You can override the directory with `--out-dir`.
The renderer runs fully offscreen (no window is created). Use `--scale` to control the
pixels-per-point (default: 2.0). Use `--headless-wait-ms` to wait for repaint requests
to settle before capturing each card (default: 2000ms).

`cargo run --example intro -- --headless --out-dir ./captures --scale 2`

## Read-only physics snapshots

`widgets::PhysicsView` is a lightweight egui Painter-based orthographic 3D
viewer. Its generic `PhysicsScene` owns f64 line, particle and label arrays;
no physics dependency is enabled by default. Keep the view in notebook state
and supply a snapshot captured by your own simulation or a saved frame:

```rust
use lissajous::widgets::{Bounds3, PhysicsScene, PhysicsView};

let mut camera = PhysicsView::default().height(360.0).bounds(Bounds3 {
    min: [-0.02, -0.01, -0.01],
    max: [0.02, 0.01, 0.01],
});
let snapshot = PhysicsScene::default(); // Populate from actual sampled data.
// In the notebook card: camera.show(ctx, &snapshot);
```

Drag to orbit, Shift/right/middle-drag to pan, and scroll to zoom. Fit and
Reset are explicit; the camera does not continuously re-fit changing frames.
An optional fixed bounds envelope keeps scales comparable across runs. A
world-orientation triad, scale bar, legend and snapshot diagnostics remain
visible. The scene declares its length units (`"m"` by default); no coordinate
or unit conversion happens implicitly.

Enable optional `rapier` / `salva` features for adapters matching
`rapier3d-f64 = 0.35.1` and `salva3d-f64 = 0.10.0`:

```rust,ignore
use lissajous::widgets::physics;

let mut snapshot = physics::rapier::scene(&rigid_bodies, &colliders);
snapshot.extend(physics::salva::scene(&liquid_world));
// Or: physics::salva::fluid_scene(&fluid)
camera.show(ctx, &snapshot);
```

Rapier snapshots compose body and collider-local world transforms and recurse
through compounds. Cuboids, balls and capsules are wireframes; unsupported
shapes get labelled, warned world-AABB approximations, or are omitted with a
warning when no finite AABB exists. Salva copies real fluid particle positions
and each fluid's configured radius, not invented motion or a mass-derived
radius. Particles pending deletion are retained and reported until the actual
simulation removes them. Boundary sampling points are not mislabelled as fluid.

Wireframes intentionally remain visible through depth-sorted particle disks;
this is an x-ray diagnostic, not hidden-surface rendering or a reconstructed
fluid surface. Radii are drawn to scale (no minimum display-radius inflation).
Invalid primitives are omitted with a count. Particle sorting/drawing runs on
the CPU; this is not a million-particle GPU renderer. The widget neither owns
nor advances a solver, requests simulation work, or certifies physical results.

```sh
cargo run --example physics_widgets
cargo run --example physics_widgets --features rapier,salva
```

The example shows labelled static geometry and, with the features enabled,
real initialized physics objects. It does not fabricate a running simulation.


# Feature Flags
Lissajous defaults to a lean build with `markdown` enabled. Add extras as needed:
- `markdown`: rich Markdown rendering with `md!` and `note!` (default).
- `typst`: Typst integration — math, scientific typesetting, and full document rendering via `typst!` macro. Renders as vector geometry directly on egui's Painter (no SVG, no raster). Includes the RAL color palette, grid-aligned layout constants, text selection, and inline error diagnostics.
- `polars`: dataframe widget (Polars + Lissajous table).
- `triblespace-pile`: retained native pile resource and immutable snapshot output
  on Unix (currently requires the exact Core source described above).
- `triblespace`: also enables the entity graph inspector and its WASM value formatters.
- `cubecl`: GPU simulated-annealing ordering for the entity inspector (use with `triblespace`).
- `telemetry`: span-based profiling via `tracing` that writes into a dedicated TribleSpace pile.
- `rapier`: read-only wireframe snapshot adapter for `rapier3d-f64` 0.35.1.
- `salva`: read-only particle snapshot adapter for `salva3d-f64` 0.10.0 (does not implicitly enable Rapier).

# Telemetry (Profiling)

Enable tracing span capture:

```sh
# In your notebook project:
TELEMETRY_PILE=./telemetry.pile \
TELEMETRY_COLLECTION_NAME=gorbie \
cargo run --features telemetry

# In this repo (demo notebook):
TELEMETRY_PILE=./telemetry.pile \
TELEMETRY_COLLECTION_NAME=gorbie-playbook \
cargo run --example playbook --features telemetry
```

For in-process embedding, attach the telemetry layer to your own `tracing_subscriber`
setup via `Telemetry::layer_from_env(...)` and keep the returned guard alive.

# Community

If you have any questions or want to chat about Rust notebooks hop into our [discord](https://discord.gg/UWZ35yHzz3).
