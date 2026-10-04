//! Widgets for inspecting immutable TribleSpace data.

#[cfg(feature = "triblespace")]
pub mod entity_inspector;

/// A retained, read-only native pile resource that publishes immutable snapshots.
#[cfg(all(feature = "triblespace-pile", unix))]
pub mod pile;

#[cfg(feature = "triblespace")]
pub use entity_inspector::{
    id_full, id_short, EntityInspectorResponse, EntityInspectorStats, EntityInspectorWidget,
    EntityOrder,
};
