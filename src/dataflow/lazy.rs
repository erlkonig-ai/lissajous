use std::hash::Hash;
use std::ops::Deref;
use std::sync::Arc;

use eframe::egui;

use super::DerivedState;
use crate::state::{ArcReadGuard, StateAccess, StateId};

/// A value that can be read from a notebook context.
///
/// Handles return a guard; lazy recipes return an owned [`Arc`]. Neither needs
/// to clone the underlying state. Reading is synchronous and explicit: this is
/// a pull interface, not a subscription or an automatic dependency scheduler.
pub trait ReadValue {
    type Value;
    type Read: Deref<Target = Self::Value>;

    fn read(&self, ctx: &impl StateAccess) -> Self::Read;

    /// Own this input and a computation without evaluating either one.
    ///
    /// Each read resolves the input in the supplied context and calls `compute`
    /// with a borrowed value. Chaining maps passes that same context all the way
    /// to the source handles. Use [`Mapped::memo`] to opt into caching.
    fn map<F, T>(self, compute: F) -> Mapped<(Self,), F>
    where
        Self: Sized,
        F: Fn(&Self::Value) -> T,
    {
        Mapped {
            inputs: (self,),
            compute,
        }
    }

    /// Render this value's `Debug` representation in a normal notebook view.
    ///
    /// `nb.view(value.tap())` reads only when the card draws, forcing this
    /// recipe's dependencies but not downstream recipes. The callback owns the
    /// recipe; copy it when `Copy`, or explicitly clone a cloneable recipe when
    /// it is also needed elsewhere. This does not memoize an ordinary map.
    fn tap(self) -> impl for<'a, 'b> FnMut(&'a mut crate::CardCtx<'b>) + 'static
    where
        Self: Sized + 'static,
        Self::Value: std::fmt::Debug,
    {
        move |ctx| {
            let text = format!("{:#?}", &*self.read(ctx));
            ctx.monospace(text);
        }
    }
}

impl<T: Send + Sync + 'static> ReadValue for StateId<T> {
    type Value = T;
    type Read = ArcReadGuard<T>;

    fn read(&self, ctx: &impl StateAccess) -> Self::Read {
        StateId::read(*self, ctx)
    }
}

/// Map one to eight readable inputs with a closure taking separate arguments.
///
/// For example, `(selection, filter).map(|selection, filter| ...)` owns both
/// inputs but reads neither until the resulting recipe is read. Arguments are
/// borrowed, so ordinary maps work with large, non-`Clone` state. Tuple inputs
/// are read in declaration order, not as an atomic snapshot. Avoid conflicting
/// lock orders and do not write a dependency from its mapping closure.
pub trait Map<F>: Sized {
    type Output;
    #[doc(hidden)]
    type Reads;

    #[doc(hidden)]
    fn resolve(&self, ctx: &impl StateAccess) -> Self::Reads;
    #[doc(hidden)]
    fn evaluate(reads: &Self::Reads, compute: &F) -> Self::Output;

    fn map(self, compute: F) -> Mapped<Self, F> {
        Mapped {
            inputs: self,
            compute,
        }
    }
}

/// A lazy, context-free recipe. Construction and copying never read its inputs.
///
/// Every read calls the closure. `Copy` and `Clone` are available exactly when
/// the inputs and closure support them; a captured closure need not be `Copy`.
#[derive(Clone, Copy)]
pub struct Mapped<I, F> {
    inputs: I,
    compute: F,
}

impl<I: Map<F>, F> ReadValue for Mapped<I, F> {
    type Value = I::Output;
    type Read = Arc<I::Output>;

    fn read(&self, ctx: &impl StateAccess) -> Self::Read {
        let reads = self.inputs.resolve(ctx);
        Arc::new(I::evaluate(&reads, &self.compute))
    }
}

impl<I, F> Mapped<I, F> {
    /// Cache this mapping's result in the context's state store, without a card.
    ///
    /// `stable_key` identifies one pure computation across reconstructions and
    /// consumer cards. Use distinct keys for distinct computations, including
    /// computations with the same Rust types. The key is NOT the cache validity
    /// key: the current input values are compared separately on every read.
    /// Changing a captured value or replacing the closure does not invalidate
    /// the cache. Represent every changing parameter as an explicit input (or
    /// include an immutable source's revision as an input).
    ///
    /// Memo inputs must be `Clone + PartialEq + Send + Sync + 'static`; outputs
    /// must be `Send + Sync + 'static`. Only opt-in memoization clones inputs.
    /// Project small public values out of large state before memoizing. A hit
    /// skips THIS mapping closure, not input resolution or upstream maps; memo
    /// expensive upstream mappings separately when needed.
    ///
    /// One slot holds only the current input: A → B → A computes three times.
    /// Reads and computations are synchronous, with no scheduling or background
    /// work. Do not recursively read the same memo slot from its computation.
    pub fn memo(self, stable_key: impl Hash) -> Memo<I, F> {
        Memo {
            mapped: self,
            id: egui::Id::new(("lissajous::dataflow::memo", stable_key)),
        }
    }
}

/// A mapped recipe with a store-backed, single-current-input cache.
///
/// Slot identity is notebook-store-wide, independent of the consuming card.
/// Rebuilding this value each frame does not discard its retained result.
#[derive(Clone, Copy)]
pub struct Memo<I, F> {
    mapped: Mapped<I, F>,
    id: egui::Id,
}

// Public only because it bounds a public trait implementation. The tuple
// implementations define which readable input sets can be memoized.
#[doc(hidden)]
pub trait MemoInputs<F>: Map<F> {
    type Key: PartialEq + Send + Sync + 'static;
    fn snapshot(reads: &Self::Reads) -> Self::Key;
    fn evaluate_key(key: &Self::Key, compute: &F) -> Self::Output;
}

impl<I: MemoInputs<F>, F> ReadValue for Memo<I, F>
where
    I::Output: Send + Sync + 'static,
{
    type Value = I::Output;
    type Read = Arc<I::Output>;

    fn read(&self, ctx: &impl StateAccess) -> Self::Read {
        // Resolve the whole input chain and release its guards before touching
        // the cache. State insertion holds the store's global write lock, so
        // its initializer must only construct an empty cache, never read inputs.
        let key = {
            let reads = self.mapped.inputs.resolve(ctx);
            I::snapshot(&reads)
        };
        let slot = StateId::<DerivedState<I::Key, I::Output>>::new(self.id);
        let cache = ctx.store().get_or_insert_with(slot, DerivedState::default);
        let result = cache
            .write()
            .get(key, |key| I::evaluate_key(key, &self.mapped.compute));
        result
    }
}

macro_rules! impl_inputs {
    ($($input:ident : $index:tt),+) => {
        impl<F, T, $($input: ReadValue),+> Map<F> for ($($input,)+)
        where
            F: Fn($(&$input::Value),+) -> T,
        {
            type Output = T;
            type Reads = ($($input::Read,)+);

            fn resolve(&self, ctx: &impl StateAccess) -> Self::Reads {
                ($(self.$index.read(ctx),)+)
            }

            fn evaluate(reads: &Self::Reads, compute: &F) -> T {
                compute($(&*reads.$index),+)
            }
        }

        impl<F, T, $($input: ReadValue),+> MemoInputs<F> for ($($input,)+)
        where
            F: Fn($(&$input::Value),+) -> T,
            $($input::Value: Clone + PartialEq + Send + Sync + 'static,)+
        {
            type Key = ($($input::Value,)+);

            fn snapshot(reads: &Self::Reads) -> Self::Key {
                ($((*reads.$index).clone(),)+)
            }

            fn evaluate_key(key: &Self::Key, compute: &F) -> T {
                compute($(&key.$index),+)
            }
        }
    };
}

impl_inputs!(A: 0);
impl_inputs!(A: 0, B: 1);
impl_inputs!(A: 0, B: 1, C: 2);
impl_inputs!(A: 0, B: 1, C: 2, D: 3);
impl_inputs!(A: 0, B: 1, C: 2, D: 3, E: 4);
impl_inputs!(A: 0, B: 1, C: 2, D: 3, E: 4, G: 5);
impl_inputs!(A: 0, B: 1, C: 2, D: 3, E: 4, G: 5, H: 6);
impl_inputs!(A: 0, B: 1, C: 2, D: 3, E: 4, G: 5, H: 6, J: 7);

#[cfg(test)]
mod tests;
