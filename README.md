![Discord Shield](https://discordapp.com/api/guilds/795317845181464651/widget.png?style=shield)

# GORBIE! - A Minimalist Notebook Environment for Rust

GORBIE continues as **[Lissajous](https://lissajous.science)**. New development
lives in the [`lissajous` crate](https://crates.io/crates/lissajous) and the
[erlkonig-ai/lissajous repository](https://github.com/erlkonig-ai/lissajous).
Gorbie remains the mascot. This final GORBIE release changes only the package
version, description, and this notice; its code is unchanged from 0.18.1.

Every other notebook environment tries to make notebooks easier, we try to make them simpler.

![GORBIE screenshot](https://github.com/triblespace/GORBIE/blob/main/assets/screenshot.png?raw=true)

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
For development, use a normal Cargo project so your IDE can index GORBIE! and
provide full static analysis.

Add the dependency and drop in a `main`:

```toml
# Cargo.toml
[dependencies]
GORBIE = "0.9.13"
```

```rust
// src/main.rs
use GORBIE::prelude::*;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.view(|ctx| {
        md!(ctx, "# GORBIE!\nA _minimalist_ notebook environment for **Rust**.");
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
//! GORBIE = "0.9.13"
//! ```
```

Make the file executable once with `chmod +x notebook.rs`.

Run it with `./notebook.rs` to load dependencies, start the notebook, and
reload on save.

The first run can take a while because Rust needs to compile and cache
dependencies - grab a coffee. Subsequent launches are fast enough that we use
them for interactive editing.

# Editor Integration
GORBIE! does not ship an editor, but it can jump to card sources. Set
`GORBIE_EDITOR` to a command with placeholders `{{file}}`, `{{line}}`, and
`{{column}}`, for example
`GORBIE_EDITOR='code -g {{file}}:{{line}}:{{column}}'` for VS Code. When set, cards show
an open-in-editor tab.

# Examples
See `GORBIE/examples` for larger notebooks and patterns. Most are runnable with
the same `watchexec` + `rust-script` shebang.

For cargo examples:
`cargo run --example grid_demo --features typst`
`cargo run --example polars --features polars`
`cargo run --example pile_inspector --features gloss`
`cargo run --example triblespace_best_practices --features triblespace`

## Typst Integration

Enable the `typst` feature for math and scientific typesetting:

```toml
GORBIE = { version = "0.8", features = ["typst"] }
```

```rust
nb.view(|ctx| {
    typst!(ctx, "= Euler's Identity\n$ e^(i pi) + 1 = 0 $");
});
```

Typst content renders as vector geometry — sharp at any zoom level, with
text selection, copy, and double-click support. The GORBIE grid constants
(`grid-span`, `grid-gutter`, etc.) are available in the Typst preamble for
grid-aligned column layouts. Compilation errors render inline as rustc-style
diagnostics with source context and hints.

# Headless capture
To export cards without opening an interactive notebook, pass `--headless`. Each card
is rendered to a PNG and saved as `card_0001.png`, `card_0002.png`, ... in the output
directory (default: `./gorbie_capture`). You can override the directory with `--out-dir`.
The renderer runs fully offscreen (no window is created). Use `--scale` to control the
pixels-per-point (default: 2.0). Use `--headless-wait-ms` to wait for repaint requests
to settle before capturing each card (default: 2000ms).

`cargo run --example pile_inspector --features gloss -- --headless --out-dir ./captures --scale 2`


# Feature Flags
GORBIE! defaults to a lean build with `markdown` enabled. Add extras as needed:
- `markdown`: rich Markdown rendering with `md!` and `note!` (default).
- `typst`: Typst integration — math, scientific typesetting, and full document rendering via `typst!` macro. Renders as vector geometry directly on egui's Painter (no SVG, no raster). Includes the RAL color palette, grid-aligned layout constants, text selection, and inline error diagnostics.
- `polars`: dataframe widget (Polars + GORBIE table).
- `triblespace`: TribleSpace widgets (commit graph, entity inspector, etc.).
- `gloss`: heavier TribleSpace visualizations (pile overview; pulls in `rapier2d`).
- `cubecl`: GPU simulated-annealing ordering for the entity inspector (use with `triblespace`).
- `telemetry`: span-based profiling via `tracing` that writes into a dedicated TribleSpace pile.

# Telemetry (Profiling)

Enable tracing span capture:

```sh
# In your notebook project:
TELEMETRY_PILE=./telemetry.pile cargo run --features telemetry

# In this repo (demo notebook):
TELEMETRY_PILE=./telemetry.pile cargo run --example playbook --features telemetry
```

Open the telemetry viewer:

```sh
cargo run --bin telemetry-viewer --features telemetry -- ./telemetry.pile
```

For in-process embedding, attach the telemetry layer to your own `tracing_subscriber`
setup via `Telemetry::layer_from_env(...)` and keep the returned guard alive.

# TribleSpace Live Patterns

When building live notebooks on top of a growing `.pile`, keep the pile/repo open
in notebook state and use `pull + checkout(prev_head..)` to process only deltas.
`widgets::triblespace::PileRepoState` / `PileRepoWidget` codify this pattern; see
the `triblespace_best_practices` example.

# Community

If you have any questions or want to chat about Rust notebooks hop into our [discord](https://discord.gg/UWZ35yHzz3).
