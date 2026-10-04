# Lissajous

Before authoring or restructuring a notebook, read
[README: Authoring notebooks](README.md#authoring-notebooks) and the worked
[geospatial_map example](examples/geospatial_map.rs). Apply its cell-owned state,
narrow public values and explicit dependency model; immediate-mode painting is
not a reason to centralize every instrument in one mutable application object.
The crate-level rustdocs point consuming authors to the same guide. Check the
actual `state`, `StateId`, `DerivedState` and `ComputedState` APIs before
assuming scheduling, cancellation or lazy initialization behavior.
`state` takes a once-only initializer closure. Read changing cell inputs in the
notebook body or draw callback, never by reading `StateId` inside that initializer
(state insertion holds the store write lock). Use existing derived/computed state
when an input change should trigger recomputation.

- Prefer the cleanest solution even if it breaks compatibility; don’t rewrite for its own sake, but surface improvement opportunities and propose how to proceed.
