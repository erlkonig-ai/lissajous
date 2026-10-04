//! Widgets for inspecting immutable TribleSpace data.

pub mod entity_inspector;

/// A retained, read-only native pile resource that publishes immutable snapshots.
#[cfg(unix)]
pub mod pile;

pub use entity_inspector::id_full;
pub use entity_inspector::id_short;
pub use entity_inspector::EntityInspectorResponse;
pub use entity_inspector::EntityInspectorStats;
pub use entity_inspector::EntityInspectorWidget;
pub use entity_inspector::EntityOrder;
