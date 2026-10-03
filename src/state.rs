use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use eframe::egui;
use parking_lot::RawRwLock;
use parking_lot::RwLock;

/// Shared read guard returned by [`StateStore::read`] and [`StateId::read`].
pub type ArcReadGuard<T> = parking_lot::lock_api::ArcRwLockReadGuard<RawRwLock, T>;
/// Shared write guard returned by [`StateStore::read_mut`] and [`StateId::read_mut`].
pub type ArcWriteGuard<T> = parking_lot::lock_api::ArcRwLockWriteGuard<RawRwLock, T>;

/// Type-erased, thread-safe store that holds all notebook state values.
///
/// Values are keyed by [`StateId`] and stored behind `Arc<RwLock<T>>` so they
/// can be shared across cards and frames without cloning the inner data.
#[derive(Debug, Default)]
pub struct StateStore {
    states: RwLock<HashMap<egui::Id, Arc<dyn Any + Send + Sync>>>,
}

impl StateStore {
    fn get_raw<T: Send + Sync + 'static>(&self, id: egui::Id) -> Option<Arc<RwLock<T>>> {
        let entry = self.states.read().get(&id).cloned()?;
        entry.downcast::<RwLock<T>>().ok()
    }

    fn get_or_insert_with_raw<T: Send + Sync + 'static>(
        &self,
        id: egui::Id,
        init: impl FnOnce() -> T,
    ) -> Arc<RwLock<T>> {
        if let Some(existing) = self.states.read().get(&id).cloned() {
            return existing
                .downcast::<RwLock<T>>()
                .expect("state store type mismatch");
        }

        let mut states = self.states.write();
        if let Some(existing) = states.get(&id).cloned() {
            return existing
                .downcast::<RwLock<T>>()
                .expect("state store type mismatch");
        }

        // Construct under the write lock so concurrent first users cannot run
        // `init` more than once. If it panics, nothing has been inserted and a
        // later call can retry normally; parking_lot locks are not poisoned.
        let state = Arc::new(RwLock::new(init()));
        states.insert(id, state.clone());
        state
    }

    pub(crate) fn get_or_insert_with<T: Send + Sync + 'static>(
        &self,
        id: StateId<T>,
        init: impl FnOnce() -> T,
    ) -> Arc<RwLock<T>> {
        self.get_or_insert_with_raw(id.id(), init)
    }

    /// Returns the state for the given handle or panics when it is missing.
    /// Use `try_get` when the state may be absent (e.g., handle from another store).
    pub fn get<T: Send + Sync + 'static>(&self, id: StateId<T>) -> Arc<RwLock<T>> {
        self.get_raw(id.id()).unwrap_or_else(|| {
            let type_name = std::any::type_name::<T>();
            panic!(
                "state missing for id {:?} ({type_name}); this usually means the handle was created in a different StateStore. Use try_read/try_read_mut when the state may be absent.",
                id.id()
            )
        })
    }

    /// Returns the state for the given handle if it exists.
    pub fn try_get<T: Send + Sync + 'static>(&self, id: StateId<T>) -> Option<Arc<RwLock<T>>> {
        self.get_raw(id.id())
    }

    /// Returns a read guard for the given handle or panics when missing.
    pub fn read<T: Send + Sync + 'static>(&self, id: StateId<T>) -> ArcReadGuard<T> {
        self.get(id).read_arc()
    }

    /// Returns a read guard for the given handle if it exists and can be read.
    pub fn try_read<T: Send + Sync + 'static>(&self, id: StateId<T>) -> Option<ArcReadGuard<T>> {
        self.try_get(id).and_then(|state| state.try_read_arc())
    }

    /// Returns a write guard for the given handle or panics when missing.
    pub fn read_mut<T: Send + Sync + 'static>(&self, id: StateId<T>) -> ArcWriteGuard<T> {
        self.get(id).write_arc()
    }

    /// Returns a write guard for the given handle if it exists and can be written.
    pub fn try_read_mut<T: Send + Sync + 'static>(
        &self,
        id: StateId<T>,
    ) -> Option<ArcWriteGuard<T>> {
        self.try_get(id).and_then(|state| state.try_write_arc())
    }
}

/// Typed handle for a value stored in a [`StateStore`].
///
/// Cheap to copy and safe to pass between cards. The type parameter `T`
/// prevents accidental access to the wrong type at compile time.
/// This is explicit access, not a subscription, projection or read-only
/// capability. Prefer a useful shared value or narrow interface to a handle for
/// an entire application controller; see the [crate-level authoring guide](crate).
/// A stateful card already holds its state's write lock during its callback:
/// do not reacquire that same state through a handle there.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct StateId<T> {
    id: egui::Id,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for StateId<T> {}

impl<T> Clone for StateId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> StateId<T> {
    pub(crate) fn new(id: egui::Id) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    pub(crate) fn id(self) -> egui::Id {
        self.id
    }
}

/// Trait for anything that provides access to the shared [`StateStore`].
///
/// Both [`CardCtx`] (inside card closures) and
/// [`NotebookCtx`](crate::NotebookCtx) (in the notebook body) implement
/// this, so [`StateId::read`] and friends work uniformly with either.
pub trait StateAccess {
    /// Returns a reference to the backing [`StateStore`].
    fn store(&self) -> &StateStore;
}

impl<T: Send + Sync + 'static> StateId<T> {
    /// Acquires a read guard on the state value. Panics if the state is missing.
    pub fn read(self, ctx: &impl StateAccess) -> ArcReadGuard<T> {
        ctx.store().read(self)
    }

    /// Acquires a read guard if the state exists and is not write-locked.
    pub fn try_read(self, ctx: &impl StateAccess) -> Option<ArcReadGuard<T>> {
        ctx.store().try_read(self)
    }

    /// Acquires a write guard on the state value. Panics if the state is missing.
    pub fn read_mut(self, ctx: &impl StateAccess) -> ArcWriteGuard<T> {
        ctx.store().read_mut(self)
    }

    /// Acquires a write guard if the state exists and is not locked.
    pub fn try_read_mut(self, ctx: &impl StateAccess) -> Option<ArcWriteGuard<T>> {
        ctx.store().try_read_mut(self)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use super::*;

    #[test]
    fn lazy_initialization_is_skipped_for_an_existing_key() {
        let store = StateStore::default();
        let state = StateId::new(egui::Id::new("lazy-existing"));
        let initializations = Cell::new(0);

        let first = store.get_or_insert_with(state, || {
            initializations.set(initializations.get() + 1);
            41_u32
        });
        let second = store.get_or_insert_with(state, || {
            initializations.set(initializations.get() + 1);
            99_u32
        });

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*second.read(), 41);
        assert_eq!(initializations.get(), 1);
    }

    #[test]
    fn panicking_initializer_leaves_the_key_absent() {
        let store = StateStore::default();
        let state = StateId::new(egui::Id::new("lazy-retry"));

        let failure = catch_unwind(AssertUnwindSafe(|| {
            store.get_or_insert_with(state, || -> u32 { panic!("initialization failed") });
        }));
        assert!(failure.is_err());

        let recovered = store.get_or_insert_with(state, || 41_u32);
        assert_eq!(*recovered.read(), 41);
    }
}
