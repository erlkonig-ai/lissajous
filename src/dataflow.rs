use eframe::egui;

/// Holds a value that can be recomputed asynchronously in a background thread.
///
/// Use [`spawn`](Self::spawn) to kick off a computation and [`poll`](Self::poll)
/// to check for completion. The current value remains accessible while a new one
/// is being computed.
///
/// This slot has no dependency key, latest-request queue or cooperative
/// cancellation. The caller decides when inputs require new work, carries their
/// identity with the result and checks it before displaying an answer. While a
/// task is tracked, [`spawn`](Self::spawn) does not queue a replacement. Calling
/// it again after completion starts another computation. See the
/// [crate-level authoring guide](crate) for composing independent cells.
///
/// On wasm, `spawn` runs the action synchronously (no threads available).
pub struct ComputedState<T> {
    value: T,
    #[cfg(not(target_arch = "wasm32"))]
    in_flight: Option<std::thread::JoinHandle<T>>,
}

impl<T> ComputedState<T> {
    /// Creates a new `ComputedState` with the given initial value.
    pub fn new(value: T) -> Self {
        Self {
            value,
            #[cfg(not(target_arch = "wasm32"))]
            in_flight: None,
        }
    }

    /// Returns a shared reference to the current value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference to the current value.
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Replaces the current value and stops tracking any in-flight computation.
    ///
    /// On native targets, dropping the join handle detaches the thread: its
    /// action continues running, but this slot will not collect that result.
    /// This is not cooperative cancellation or a resource shutdown mechanism.
    pub fn set(&mut self, value: T) {
        self.value = value;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.in_flight = None;
        }
    }

    /// Returns `true` if a background computation is in progress.
    pub fn is_running(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.in_flight.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Checks whether the in-flight computation has finished.
    /// If so, updates the stored value and returns `true`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll(&mut self) -> bool
    where
        T: Send + 'static,
    {
        let Some(handle) = self.in_flight.as_ref() else {
            return false;
        };
        if !handle.is_finished() {
            return false;
        }
        let handle = self.in_flight.take().expect("in-flight handle missing");
        self.value = handle.join().unwrap();
        true
    }

    #[cfg(target_arch = "wasm32")]
    pub fn poll(&mut self) -> bool {
        false
    }

    /// Spawns `action` on a background thread if no computation is already running.
    /// Returns a mutable reference to the current (potentially stale) value.
    ///
    /// The worker wakes the page itself: it holds a clone of `ctx` and calls
    /// [`request_repaint`](egui::Context::request_repaint) the moment the value
    /// lands, rather than leaving the notebook to discover it on whatever
    /// repaint happens to come next. A context is an `Arc` behind a lock, so
    /// the clone is a refcount bump and is `Send + Sync`; `request_repaint` is
    /// idempotent, so waking unconditionally is safe.
    ///
    /// **The wake and the slow poll a waiting page keeps up are a pair, not
    /// alternatives — do not delete either one.** If `action` panics the wake
    /// never fires: the thread dies, [`is_running`](Self::is_running) goes
    /// false, and nothing has asked for a repaint, so a page relying on the
    /// wake alone sits there looking busy for the rest of the session. With the
    /// heartbeat underneath (see [`load_auto`](crate::widgets::load_auto)) a
    /// lost wake costs one poll interval instead.
    ///
    /// On wasm `action` runs synchronously and `ctx` is unused: there is no
    /// latency to close and no thread to wake. That arm is also why a bare
    /// `std::thread::spawn` is the wrong thing to reach for in a card body.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn(
        &mut self,
        ctx: &egui::Context,
        action: impl FnOnce() -> T + Send + 'static,
    ) -> &mut T
    where
        T: Send + 'static,
    {
        self.poll();
        if !self.is_running() {
            let ctx = ctx.clone();
            self.in_flight = Some(std::thread::spawn(move || {
                let value = action();
                ctx.request_repaint();
                value
            }));
        }
        &mut self.value
    }

    #[cfg(target_arch = "wasm32")]
    pub fn spawn(&mut self, _ctx: &egui::Context, action: impl FnOnce() -> T) -> &mut T {
        self.value = action();
        &mut self.value
    }
}

impl<T: Default> Default for ComputedState<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for ComputedState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputedState")
            .field("value", &self.value)
            .field("running", &self.is_running())
            .finish()
    }
}

/// Holds a value derived from a key, recomputed only when the key moves.
///
/// The synchronous sibling of [`ComputedState`]. A notebook re-runs every card
/// body on every repaint — a hover, a scroll, a window focus — so a derivation
/// written inline in a card is a derivation performed at frame rate. For work
/// measured in seconds that is what `ComputedState` is for; for work measured
/// in milliseconds a background thread is the wrong answer, because a card that
/// shows last frame's figures for a derivation costing one millisecond is worse
/// than one that simply computes it. This is the middle case, and in practice
/// it is the common one.
///
/// The key is the whole of the contract. It must name *every* input the
/// computation reads, because nothing here can detect an input it was not told
/// about, and a stale figure is indistinguishable from a fresh one at the point
/// it is read. Prefer a key that recomputes too often to one that might not
/// recompute at all.
///
/// [`get`](Self::get) hands the key back to the closure, and the closure should
/// read its inputs from *there* rather than from the owner it happens to be
/// written inside. Be precise about what that buys. It does **not** make an
/// incomplete key impossible: only a genuinely non-capturing `fn(&K) -> T`
/// would do that, and one cannot be used here, because the key is compared on
/// every frame and so must be cheap to compare — which forces it to hold
/// *representatives*, a snapshot revision standing for twenty-three million
/// facts, rather than the inputs themselves. Requiring the key to contain every
/// input would cost the cheap comparison that is the memo's whole purpose. It
/// is a tradeoff, not an impossibility.
///
/// What it does kill is divergence: the key says `filter` is A while the
/// closure reads `self.filter`, which is B. A card body has `self` in scope
/// with every field on it, so that trap is a real one. With `&key` in hand the
/// natural thing to write is `key.filter`, and the two cannot disagree.
///
/// ```ignore
/// // In a card body, where `self` is the viewer holding the state. `book` is
/// // pulled out first so the closure borrows one field rather than all of
/// // `self`; it is not in the key because the revision stands for it.
/// let book = &self.book;
/// let rows = self
///     .by_customer
///     .get((revision, filter.clone()), |key| book.revenue(&key.1));
/// for row in rows.iter() { /* ... */ }
/// ```
///
/// One slot, always the current key: returning to a previous key is a
/// recomputation, not a hit. A map would keep figures alive against a model
/// that can run to gigabytes, for keys nobody is looking at.
pub struct DerivedState<K, T> {
    key: Option<K>,
    value: Option<std::sync::Arc<T>>,
}

impl<K, T> Default for DerivedState<K, T> {
    fn default() -> Self {
        Self {
            key: None,
            value: None,
        }
    }
}

impl<K: PartialEq, T> DerivedState<K, T> {
    /// An empty state, holding nothing until the first [`get`](Self::get).
    pub fn new() -> Self {
        Self::default()
    }

    /// The value for `key`, computing it only when the key has moved.
    ///
    /// `compute` is handed the key it is computing under. Read the inputs from
    /// it — `key.filter`, not `self.filter` — so the figure cannot be computed
    /// from one thing while being filed under another. See the type docs for
    /// what that does and does not guarantee.
    ///
    /// Returns an [`Arc`](std::sync::Arc) rather than a borrow so the caller is
    /// free to touch the rest of its state while it draws. A `&T` out of
    /// `&mut self` pins the whole owner for the length of a card body, which is
    /// exactly the span in which a card also wants its filter row and its
    /// controls; the allocation is nothing beside the computation it replaces.
    pub fn get(&mut self, key: K, compute: impl FnOnce(&K) -> T) -> std::sync::Arc<T> {
        if self.key.as_ref() != Some(&key) {
            // Order matters if `compute` panics: leave the key unset so a later
            // call retries rather than serving a value that was never produced.
            self.key = None;
            self.value = Some(std::sync::Arc::new(compute(&key)));
            self.key = Some(key);
        }
        self.value
            .clone()
            .expect("a value is present whenever the key is")
    }

    /// The held value, if the state has ever been computed.
    ///
    /// For a caller that wants to draw what it has without asking for a
    /// recomputation it is not ready to pay for.
    pub fn peek(&self) -> Option<&std::sync::Arc<T>> {
        self.value.as_ref()
    }

    /// The key the held value was computed under.
    pub fn key(&self) -> Option<&K> {
        self.key.as_ref()
    }

    /// Drop the held value, so the next [`get`](Self::get) recomputes.
    pub fn clear(&mut self) {
        self.key = None;
        self.value = None;
    }
}

impl<K: std::fmt::Debug, T> std::fmt::Debug for DerivedState<K, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedState")
            .field("key", &self.key)
            .field("held", &self.value.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    use super::ComputedState;
    use super::DerivedState;
    #[cfg(not(target_arch = "wasm32"))]
    use eframe::egui;
    use std::cell::Cell;

    /// The wake, not the poll: a finished task must have asked for a repaint
    /// by the time its value is collectable.
    ///
    /// Without this the only thing standing between a landed value and a page
    /// that shows it is the caller's heartbeat, and a test that merely compiled
    /// the call would not tell the difference.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn a_finished_task_wakes_the_page() {
        let ctx = egui::Context::default();
        // A fresh context wants its first passes; run until it stops asking, so
        // the assertion below is about the worker and not about start-up.
        for _ in 0..8 {
            if !ctx.has_requested_repaint() {
                break;
            }
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        assert!(
            !ctx.has_requested_repaint(),
            "the context should be quiet before the task is spawned"
        );

        let mut state = ComputedState::new(0u32);
        state.spawn(&ctx, || 7u32);
        while !state.poll() {
            std::thread::yield_now();
        }

        assert_eq!(*state.value(), 7);
        assert!(
            ctx.has_requested_repaint(),
            "a task that finished without waking the page leaves it looking busy"
        );
    }

    /// The whole contract in one test: compute on a new key, and only then.
    ///
    /// A state that recomputed too often would only be slow. One that
    /// recomputed too rarely would put the previous key's figures on the
    /// screen, which is the failure this has to be trusted not to have.
    #[test]
    fn a_value_is_computed_once_per_key_and_again_when_the_key_moves() {
        let computed = Cell::new(0);
        let mut state: DerivedState<(u32, char), String> = DerivedState::new();
        fn ask(
            state: &mut DerivedState<(u32, char), String>,
            computed: &Cell<u32>,
            key: (u32, char),
        ) -> std::sync::Arc<String> {
            state.get(key, |key| {
                computed.set(computed.get() + 1);
                format!("{}-{}", key.0, key.1)
            })
        }
        let mut ask = |key| ask(&mut state, &computed, key);

        assert_eq!(*ask((1, 'a')), "1-a");
        assert_eq!(computed.get(), 1);
        // Same key: the held value, untouched.
        assert_eq!(*ask((1, 'a')), "1-a");
        assert_eq!(computed.get(), 1);
        // Either half of the key moving is enough to invalidate.
        assert_eq!(*ask((2, 'a')), "2-a");
        assert_eq!(computed.get(), 2);
        assert_eq!(*ask((2, 'b')), "2-b");
        assert_eq!(computed.get(), 3);
        // Coming back to a key is a recomputation, not a hit: one slot, always
        // the current one.
        assert_eq!(*ask((1, 'a')), "1-a");
        assert_eq!(computed.get(), 4);
    }
}
