use std::hash::Hash;

use crate::cards::Card;
use crate::state::StateId;
use crate::CardCtx;
use crate::NotebookCtx;

type StatefulCardFn<T> = dyn for<'a, 'b> FnMut(&'a mut CardCtx<'b>, &mut T);

/// A card that owns persistent state of type `T` across frames.
pub struct StatefulCard<T> {
    state: StateId<T>,
    function: Box<StatefulCardFn<T>>,
}

impl<T> StatefulCard<T> {
    pub(crate) fn new(
        state: StateId<T>,
        function: impl for<'a, 'b> FnMut(&'a mut CardCtx<'b>, &mut T) + 'static,
    ) -> Self {
        Self {
            state,
            function: Box::new(function),
        }
    }
}

impl<T: Send + Sync + 'static> Card for StatefulCard<T> {
    fn draw(&mut self, ctx: &mut CardCtx<'_>) {
        let mut current = self.state.read_mut(ctx);
        (self.function)(ctx, &mut current);
    }
}

/// Creates a card with persistent state keyed by `key`, lazily initialized by `init`.
///
/// Returns a [`StateId`] handle that can be used to read the state from other cards.
#[track_caller]
pub fn stateful_card<K, T>(
    nb: &mut NotebookCtx,
    key: &K,
    init: impl FnOnce() -> T,
    function: impl for<'a, 'b> FnMut(&'a mut CardCtx<'b>, &mut T) + 'static,
) -> StateId<T>
where
    K: Hash + ?Sized,
    T: Send + Sync + 'static,
{
    nb.state(key, init, function)
}
