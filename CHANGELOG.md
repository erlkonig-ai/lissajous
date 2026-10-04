# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Compact native pile instrument: configured path, measured byte rail and replay
  watermark; public data-only `PileProgress` also serves preflight/error faces
  without implying a refresh action or snapshot readiness.
- Lazy single-input and tuple `.map` expressions share `.read(ctx)` with state
  handles; opt-in `.memo(stable_key)` retains one current-input result without
  a cache card. `nb.view(value.tap())` displays a lazy Debug view. Dependencies
  remain explicit; there is no automatic scheduler or change to async work.
- **One lazy retained-state API.** `NotebookCtx::state(key, init, draw)` now
  accepts an `FnOnce() -> T` initializer. Removed the eager-value overload and
  `state_with` alias; callers pass a closure or constructor. State keys, caller
  locations and retained-card identity are unchanged. Initialization is once
  per absent key, not dependency reevaluation; consumers read dependencies
  during composition or drawing and use existing derived/computed state.
- Unix native pile resource cell behind `triblespace-pile`: retained read-only open
  and refresh, coalesced byte progress, and actual immutable snapshot output.
  Consumer queries remain independent. This source integration requires the
  unpublished Core `refresh_next` API; no registry compatibility or release is
  implied by the existing dependency version.
- The pile-only feature opts into native parallel queries without TribleSpace's
  GPU, WASM or object-store defaults; `triblespace` retains the existing
  inspector and its required WASM formatters, and includes the pile cell.

## 0.19.1 - 2026-10-03

- Borrowed geospatial `MapView` with persistent camera and opaque caller-owned
  selection, dateline-aware Web Mercator geometry, invalid-vertex gaps,
  explicit fitting, and local approximate scale; no network or database bridge.
- Map callers can hide controls and projection annotations independently.
  Background clicks and map fitting clear selection immediately; drags and
  scroll zoom preserve selection.
- Floating card outlines remain above full-bleed content in light and dark
  themes.

## 0.19.0 - 2026-10-01

- **Lissajous.** The notebook library is now the `lissajous` crate, hosted at
  `erlkonig-ai/lissajous` with homepage <https://lissajous.science>. Gorbie
  remains the mascot; existing style names, fonts, editor environment variable
  and capture defaults are unchanged. Consumers can retain their Rust import
  name with `GORBIE = { package = "lissajous", version = "0.19" }`; notebook
  macros resolve the Cargo dependency alias rather than a hardcoded crate name.
- **Breaking API release.** Removed branch/head widgets and the old gloss
  surface are not compatibility shims. Optional TribleSpace integration uses
  0.47.0. The renamed `lissajous-macros` starts at 0.10.0 with this release:
  its generated headless-theme code targets this API and is not a compatible update for
  consumers pairing GORBIE 0.18 with the 0.9 macro line.

- **Read-only physics views.** `PhysicsScene` and `PhysicsView` display owned
  3D wireframes/particles with orbit, pan, zoom, stable bounds and fit/reset.
  Optional `rapier` and `salva` adapters capture real f64 collider transforms
  and fluid positions/radii without taking ownership of simulation stepping.
  Unsupported collider shapes are explicitly labelled AABB approximations.
  The generic viewer adds neither engine to default builds. Also gate
  `CardCtx::markdown` consistently so no-default-feature consumers compile.
- **Sections are top-level card components.** `docs/widgets.md` and the
  `section` docs now say where a section goes: at the top of a card, with a
  grid or one full-width widget nested inside it. A section inside a grid
  cell inherits the cell's inset, which gives the header a border and margin
  it is not designed for, and several sections in one card cannot be
  detached separately, so the rule is one section per card. The `grid_demo`
  sections card is split into one card per section accordingly.

- **Removed the cluster-health notebook and companion binary.** `trible pile
  net dashboard --gui` no longer shells out to `gorbie-cluster-health`; it
  embeds its own notebook over the live telemetry `Frame`. The companion
  rendered the older `DashboardReport` and nothing called it any more, so it
  was a second dashboard free to drift out of agreement with the one people
  actually run.
- **Resident native PNG capture.** `NotebookConfig::capture` delivers ordered
  `CapturedPng` card tiles to a synchronous consumer without filesystem output.
  File capture shares the renderer and encoder, preserving filenames, density,
  and explicit headless theme selection (dark by default). Invalid scales fail
  before GPU acquisition; renderer, encoder, and consumer errors stop delivery
  without retrying accepted images. Scale and per-layout settling remain
  separate from any caller-owned output budget or overall deadline.
- **Lazy notebook state initialization.** `NotebookCtx::state_with` accepts an
  initializer closure that runs only when its state key is absent, so
  expensive resources are not reconstructed while the notebook body rebuilds
  on later frames. The existing eager-value `state` API is unchanged. The new
  `incremental_collection` example uses it to retain two live Pile handles and
  incrementally fold exact collection-ticket additions without frame-rate
  storage polling or a resident watcher thread.
- **Security lock refresh.** Bump the retained `quinn-proto` lock entry
  from 0.11.14 to 0.11.15, closing GHSA-4w2j-m93h-cj5j.
- **Restore WASM builds after the wgpu/theme additions.** `egui-wgpu`
  is now a direct dependency on both native and web targets, and the
  native-only headless-theme initializer is correctly target-gated.
- **Live-dashboard widget set.** Four new reusable widgets grown out
  of the `drive` operational dashboard: `StreamLane` (scrolling
  tagged text runs with chunked, lazily laid-out rendering and a
  colorblind-safe default palette), `BudgetGauge` (scalar vs budget,
  composed from the LCD readout + `ProgressBar` meter; over/under is
  expressed through luminance and the RAL 2005 accent, never
  red/green), `MetricStrip` (feature `plots`; rolling time-series
  with inline readout and optional p50/p95 percentile band), and
  `EventFeed` (newest-first rows with colorhash category pills and
  collapsible detail). Plus `widgets::lcd_readout` (read-only LCD
  display field) and `colorhash::RAL_CVD_SAFE` (colorblind-safe
  categorical palette; `Fnv1a64` now implements `std::hash::Hasher`).
- **TribleSpace collection cutover.** Removed the branch/head-based
  commit browser, pile tail, pile overview, telemetry viewer, and their
  obsolete examples after TribleSpace retired mutable repositories.
  The immutable entity inspector remains, telemetry capture continues
  to publish native collection commits, and the spatial pile resolver
  now reopens and materializes a native collection.

## 0.18.0 - 2026-06-05

- **Bump optional `triblespace` dep 0.45 → 0.46.** Picks up the new
  `PinSnapshot` type (cheap O(refcount-bump) snapshot of the
  pin → head map via `PinStore::pin_snapshot()`), the
  snapshot-first publish ordering in `triblespace-net` (closes a
  race where a peer dialing in after a gossip hit a stale serving
  snapshot), and the OP_DELIVER_CAP swarm-fetch + dialer-equals-
  issuer verify path. No GORBIE-side code changes required.

## 0.17.0 - 2026-06-03

- **Bump optional `triblespace` dep 0.44 → 0.45.** Picks up the PATCH
  `LocalLeaf` archive-leaf elimination (~47% memory savings on
  `SimpleArchive` ingest, archive ingest now equal to or faster than
  heap at all measured scales), the `ArchiveEntry` / `insert_archive`
  ingest path, and the `team revoke` removal. No GORBIE-side code
  changes required.
- **Web export via recursive `cfg`-gated proc macro.** The notebook
  builder now ships a clean wasm export path through a proc-macro
  that walks the cell tree at compile time and emits per-target
  code, so notebooks compile down to a static bundle without a
  runtime feature flag.

## 0.16.0 - 2026-05-31

- **Bump optional `triblespace` dep 0.42 → 0.44.** Picks up the
  descriptive-capabilities substrate, `BranchStore → PinStore`
  rename, `Repository::new` taking `F: Into<Fragment>`, and the
  engine improvements (NotAttr, same-Variable handling,
  RegularPathConstraint end-bound proposal, path! infix `?`/`!`/`^`).
  No code changes required on the GORBIE side — the upgrade is
  source-compatible through the deprecated-but-still-working
  TribleSet handoff to Repository::new.

## 0.14.3 - 2026-05-16

- **Bump optional `triblespace` dep 0.40 → 0.41.** Tracks the
  iroh-0.98 upgrade in `triblespace-net`, which fixes the
  ed25519-dalek 3.0.0-pre.1 vs ed25519 3.0.0 compile failure
  upstream (by re-pinning to pre.6). Replaces the 0.14.2
  Cargo.lock workaround with a proper resolution path.
  Source identical to 0.14.2.

## 0.14.2 - 2026-05-16

- **Pick up `triblespace 0.40.2`** so downstream consumers
  (notably `faculties`) get the TLS-roots-from-OS-store fix
  through `GORBIE = "0.14"` without manual lockfile work.
  Cargo.lock pins bumped via `cargo update -p triblespace`;
  source identical to 0.14.1.

## 0.13.2 - 2026-05-07

- **Bump optional `triblespace` dep 0.37 → 0.38.** Tracks the
  team-rooted-gossip release. Source identical otherwise; this
  is a dep-version-tracking patch so downstream crates pinning
  `GORBIE = "0.13"` can pull the new triblespace through the
  optional `triblespace` feature without resolving against
  conflicting versions.

## 0.13.1 - 2026-05-07

- **RUSTSEC patch-bump four transitive deps in Cargo.lock**:
  - `thin-vec 0.2.14 → 0.2.16` (GHSA-xphw-cqx3-667j, high) —
    Use-After-Free + Double Free in `IntoIter::drop` when an
    element's `Drop` panics. Pulled in via `typst-utils`.
  - `quinn-proto 0.11.13 → 0.11.14` (GHSA-6xvm-j4wr-6v98, high) —
    Unauthenticated remote DoS via panic in QUIC transport
    parameter parsing. Stale lockfile entry — not in the active
    dep graph but tracked in `Cargo.lock`.
  - `rand 0.8.5 → 0.8.6` (GHSA-cq8v-f236-94qc, low) — Unsoundness
    with custom logger using `rand::rng()`. Pulled in via
    `ashpd` (dark-light) and `lipsum` (typst-library).
  - `rand 0.9.2 → 0.9.3` — Same advisory, 0.9 line. Pulled in
    via `cubecl-common`.
  All four are pure patch bumps within existing semver ranges;
  no Cargo.toml changes needed.

## 0.13.0 - 2026-05-07

- **Bump optional `triblespace` dep to 0.37.** Telemetry +
  pile-inspector examples track the new release. The bump is
  the headline reason this is a minor (rather than patch)
  release — pre-1.0, but breaking for downstreams that pin to
  `triblespace 0.36`.
- **Bump egui family to 0.34.x latest** (`egui_plot` 0.34 →
  0.35). Picks up the upstream `hit_test.rs:365` fix that
  prevented `Sense::click_and_drag` on widgets adjacent to
  click-sensors — see float / drag-sense changes below.
- **Bump `rustls-webpki` 0.103.9 → 0.103.13** (RUSTSEC
  advisories).
- **Stacked floats no longer drag in lockstep.** Float handles
  use `Sense::click_and_drag` and read `dragged()` /
  `drag_delta()` directly — egui's drag sense is z-aware, so a
  drag-start event is consumed by exactly the topmost handle
  under the pointer. The 0.34 migration's `hit_test.rs:365`
  panic that prompted manual pointer tracking has been fixed
  upstream in 0.34.x; ~25 lines of memory-id bookkeeping gone.
- **Floating cards render at natural content height.** Tall
  floats (multi-page wiki fragments, long compass goal lists)
  no longer clip at a fixed viewport-height cap. `max_rect.max.y`
  is set to `min_y + min_height` so `available_height()` is a
  finite floor, but `min_rect` grows freely past it via
  `allocate_space(...)` — so `inner.response.rect.height()` ends
  up as the actual natural content height, regardless of how
  tall the body is, while still preventing fill-available
  widgets (transitive ScrollArea, vertical layouts) from seeing
  `f32::INFINITY` and reporting runaway heights.
- **Fix infinite-scroll feedback when a content-anchored float
  is open.** The notebook's scroll-content extension was
  comparing `frame_rect.bottom()` (screen coords, drifts with
  scroll position) against `float_bottom` (content coords,
  fixed) — subtracting the two mixed coordinate systems and
  grew linearly with scroll position, so each frame the user
  scrolled, the allocated scroll content expanded, which let
  them scroll further next frame. Compare *heights* instead
  (`frame_rect.height()` vs `float_bottom`); both are
  coord-system-independent extents. Wiki and compass floats
  triggered this; activity timeline (no content-anchored
  floats) was unaffected.

## 0.12.0 - 2026-04-19
- **Central panel background** restored after the egui 0.34 migration —
  `fn ui` now receives a bare `&mut Ui` with no default frame/fill, so
  the notebook wraps its body in `Frame::central_panel(&ctx.global_style())`.
- **Drag-to-scroll disabled** on the main notebook ScrollArea. egui's
  hit-test panics (`hit_test.rs:365`) when a big drag-sensing ScrollArea
  coexists with nearby click-sensing widgets — which is every interactive
  card in a notebook. Users still scroll via scroll bars + mouse wheel.
- **Floating card drag handle** uses `Sense::click()` with manual drag
  detection via `pointer.primary_down` + `pointer.hover_pos`, same
  sidestep around the egui hit-test bug.

## 0.9.13 - 2026-04-05
- Switch `telemetry` to the shared `triblespace::telemetry` implementation and
  use `TELEMETRY_*` environment variables.
- Upgrade optional `triblespace` dependency to `0.34.1`.
- Update telemetry viewer to rely on generic shared telemetry fields (no
  `card_index` dependency).
- Refresh the TribleSpace examples for current `Checkout` and `NsTAIInterval`
  APIs, and drop the removed `Blake2b` schema from `schema_inspector`.
- Rename telemetry viewer binary to `telemetry-viewer`.

## 0.8.2 - 2026-03-16
- **Collapsed detached placeholders**: detached cards leave a slim 12px
  (GRID_ROW_MODULE) hatched strip instead of reserving full card height.
- **Grid-aligned card chrome**: detach/open-in-editor tab buttons (24px),
  drag handle (12px), and handle stripes all snap to the grid module.

## 0.8.1 - 2026-03-16
- **Clickable Typst links**: `#link()` and auto-detected URLs are now
  interactive — pointer changes to a hand cursor on hover and click opens
  the URL. Link text renders in RAL signal blue.
- Links demo card in `grid_demo` example.

## 0.8.0 - 2026-03-16
- **Typst integration** (`typst` feature): compile Typst markup and render as
  vector geometry directly on egui's Painter. Supports math (inline/display),
  full documents, shapes, dash patterns, text stroke, even-odd fill rule, and
  the RAL color palette. Includes AST-aware text selection, copy, and
  double-click (opacity transition for structure, paragraph for text).
  Grid constants (`grid-span`, `grid-gutter`, etc.) available in the Typst
  preamble for grid-aligned column layouts. Compilation errors render inline
  as rustc-style diagnostics with source context and hints.
- **CardCtx convenience methods**: `slider()`, `number()`, `text_field()`,
  `toggle()`, `small_button()`, `progress()` — shadows egui defaults with
  GORBIE-styled widgets.
- **Grid-native widget sizing**: NumberField and TextField snap to
  `2 * GRID_ROW_MODULE` (24px) minimum height instead of egui's default.
- 12-column modular grid with named spans (`full`, `half`, `third`, `quarter`,
  `two_thirds`, `three_quarters`) and skip/furniture helpers.
- Grid-aligned markdown heading sizes for IosevkaGorbie.
- Widget showcase and visual grid reference in `grid_demo` example.

## 0.5.1 - 2026-01-22
- Fix detached card dragging across the margin by deferring anchor layer switches.

## 0.5.2 - 2026-02-04
- Switch to crates.io dependencies for gorbie-macros, gorbie-commonmark, and triblespace 0.9.0.

## 0.5.3 - 2026-02-08
- Bump optional `triblespace` dependency to `0.11.0`.

## 0.5.0 - 2026-01-22
- Bump crate version to 0.5.0.
- Pin optional `triblespace` dependency to `0.8.0` for publishing.
- Update README snippets to `GORBIE = "0.5.0"`.
