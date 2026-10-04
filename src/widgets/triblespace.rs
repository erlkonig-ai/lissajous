//! Widgets for inspecting immutable TribleSpace data.

#[cfg(feature = "triblespace")]
pub mod entity_inspector;

/// A retained, read-only native pile resource that publishes immutable snapshots.
#[cfg(all(feature = "triblespace-pile", unix))]
pub mod pile;

/// Read-only inspection of one native immutable pile snapshot.
#[cfg(all(feature = "triblespace-pile", unix))]
pub mod snapshot;

/// Prepared, read-only inspection of a chosen native collection observation.
#[cfg(feature = "triblespace-pile")]
pub mod collection;

#[cfg(feature = "triblespace-pile")]
mod inspector_bytes;

#[cfg(feature = "triblespace")]
pub use entity_inspector::{
    id_full, id_short, EntityInspectorResponse, EntityInspectorStats, EntityInspectorWidget,
    EntityOrder,
};
