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

Immediate-mode: the notebook redraws every frame, and state lives in
`nb.state` handles. This makes it easy to build interactive UIs that are extremely
robust and responsive to user input without complex reactivity systems.

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

    let slider = nb.state("slider", 0.5, |ctx, value| {
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

`nb.state` keeps the first value for a key, but its ordinary Rust argument is
still evaluated before every call. Use `nb.state_with("key", || expensive(),
|ctx, value| { /* ... */ })` when a resource should be constructed only while
that state key is absent.

For reload-on-change with Cargo, use:
`watchexec -r -w src -w Cargo.toml -- cargo run`
or `cargo watch -x run` (install with `cargo install cargo-watch`).

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
- `triblespace`: immutable TribleSpace data inspection with the entity graph widget.
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
