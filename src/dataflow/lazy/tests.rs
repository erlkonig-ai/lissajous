use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::*;
use crate::state::StateStore;

struct Context<'a>(&'a StateStore);

impl StateAccess for Context<'_> {
    fn store(&self) -> &StateStore {
        self.0
    }
}

fn state<T: Send + Sync + 'static>(ctx: &Context<'_>, key: &str, value: T) -> StateId<T> {
    let id = StateId::new(egui::Id::new(key));
    ctx.store().get_or_insert_with(id, || value);
    id
}

fn expensive_calculation(left: &u32, right: &u32) -> u32 {
    left + right
}

#[test]
fn construction_is_lazy_and_plain_maps_compute_on_every_read() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let source = StateId::<u32>::new(egui::Id::new("not-yet-present"));
    let calls = Cell::new(0);
    let mapped = source.map(|value| {
        calls.set(calls.get() + 1);
        value + 1
    });
    assert_eq!(calls.get(), 0);
    store.get_or_insert_with(source, || 4);
    assert_eq!(*mapped.read(&ctx), 5);
    assert_eq!(*mapped.read(&ctx), 5);
    assert_eq!(calls.get(), 2);
}

#[test]
fn tuple_function_and_chaining_use_the_final_read_context() {
    let first_store = StateStore::default();
    let second_store = StateStore::default();
    let first = Context(&first_store);
    let second = Context(&second_store);
    let left = state(&first, "left", 1);
    let right = state(&first, "right", 2);
    state(&second, "left", 10_u32);
    state(&second, "right", 20_u32);
    let sum = (left, right).map(expensive_calculation);
    let text = sum.map(|sum| format!("sum: {sum}"));
    assert_eq!(&*text.read(&first), "sum: 3");
    assert_eq!(&*text.read(&second), "sum: 30");
}

#[test]
fn non_clone_state_can_project_a_small_memo_dependency() {
    struct LargeState {
        selected: u32,
        data: Vec<u8>,
    }
    let store = StateStore::default();
    let ctx = Context(&store);
    let source = state(
        &ctx,
        "large",
        LargeState {
            selected: 4,
            data: vec![0; 1024],
        },
    );
    let projections = Cell::new(0);
    let computations = Cell::new(0);
    let selected = source.map(|state| {
        projections.set(projections.get() + 1);
        assert_eq!(state.data.len(), 1024);
        state.selected
    });
    let label = selected
        .map(|selected| {
            computations.set(computations.get() + 1);
            format!("selected: {selected}")
        })
        .memo("label");
    assert_eq!(&*label.read(&ctx), "selected: 4");
    assert_eq!(&*label.read(&ctx), "selected: 4");
    assert_eq!(
        projections.get(),
        2,
        "upstream maps resolve the current input"
    );
    assert_eq!(computations.get(), 1, "a hit skips the wrapped map");
}

#[test]
fn reconstructing_a_recipe_reuses_the_slot_across_consumers() {
    let store = StateStore::default();
    let first_card = Context(&store);
    let second_card = Context(&store);
    let left = state(&first_card, "left", 1);
    let right = state(&first_card, "right", 2);
    let first = (left, right)
        .map(expensive_calculation)
        .memo("total")
        .read(&first_card);
    let next_frame = (left, right)
        .map(expensive_calculation)
        .memo("total")
        .read(&second_card);
    assert_eq!(*first, 3);
    assert!(Arc::ptr_eq(&first, &next_frame));
}

#[test]
fn each_input_invalidates_and_returning_to_an_old_input_recomputes() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let left = state(&ctx, "left", 1_u32);
    let right = state(&ctx, "right", 2_u32);
    let calls = Cell::new(0);
    let total = (left, right)
        .map(|left, right| {
            calls.set(calls.get() + 1);
            left + right
        })
        .memo("total");
    let old = total.read(&ctx);
    assert_eq!(*old, 3);
    assert!(Arc::ptr_eq(&old, &total.read(&ctx)));
    *left.read_mut(&ctx) = 4;
    assert_eq!(*total.read(&ctx), 6);
    *right.read_mut(&ctx) = 7;
    assert_eq!(*total.read(&ctx), 11);
    *left.read_mut(&ctx) = 1;
    *right.read_mut(&ctx) = 2;
    assert_eq!(*total.read(&ctx), 3);
    assert_eq!(calls.get(), 4);
    assert_eq!(*old, 3, "previously returned values remain valid");
}

#[test]
fn distinct_keys_and_stores_do_not_share_results() {
    let first_store = StateStore::default();
    let second_store = StateStore::default();
    let first = Context(&first_store);
    let second = Context(&second_store);
    let source = state(&first, "input", 1_u32);
    state(&second, "input", 1_u32);
    let calls = Cell::new(0);
    let mapped = source.map(|value| {
        calls.set(calls.get() + 1);
        value + 1
    });
    let a = mapped.memo("a");
    let b = mapped.memo("b");
    let first_a = a.read(&first);
    let first_b = b.read(&first);
    let second_a = a.read(&second);
    assert_eq!(calls.get(), 3);
    assert!(!Arc::ptr_eq(&first_a, &first_b));
    assert!(!Arc::ptr_eq(&first_a, &second_a));
    assert!(Arc::ptr_eq(&first_a, &a.read(&first)));
}

#[test]
fn changing_captures_are_not_dependencies_but_explicit_inputs_are() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let input = state(&ctx, "input", 3_u32);
    let reconstruct = |multiplier| input.map(move |value| value * multiplier).memo("captured");
    assert_eq!(*reconstruct(2).read(&ctx), 6);
    assert_eq!(
        *reconstruct(4).read(&ctx),
        6,
        "captures cannot invalidate a slot"
    );

    let multiplier = state(&ctx, "multiplier", 2_u32);
    let explicit = (input, multiplier)
        .map(|value, multiplier| value * multiplier)
        .memo("explicit");
    assert_eq!(*explicit.read(&ctx), 6);
    *multiplier.read_mut(&ctx) = 4;
    assert_eq!(*explicit.read(&ctx), 12);
}

#[test]
fn copy_recipes_do_not_require_copy_inputs_or_outputs_and_captures_need_not_be_copy() {
    fn assert_copy<T: Copy>(_: T) {}
    struct NotClone(u32);
    let store = StateStore::default();
    let ctx = Context(&store);
    let input = state(&ctx, "input", NotClone(7));
    assert_copy(input);
    let projected = input.map(|value| value.0);
    assert_copy(projected);
    let mapped = projected.map(|value| value.to_string());
    assert_copy(mapped);
    assert_copy(mapped.memo("copy"));

    let prefix = String::from("value: ");
    let captured = projected.map(move |value| format!("{prefix}{value}"));
    assert_eq!(&*captured.read(&ctx), "value: 7");
}

#[test]
fn memo_dependencies_resolve_before_cache_locks_and_release_their_guards() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let input = state(&ctx, "input", 3_u32);
    let upstream = input.map(|value| value + 1).memo("upstream");
    let result = (upstream, input)
        .map(|upstream, value| {
            // A probe, not recommended pure-computation style: the source guard
            // must be gone and insertion's store lock must be released.
            assert!(input.try_read_mut(&ctx).is_some());
            let probe = state(&ctx, "created-during-computation", 9_u32);
            assert_eq!(*probe.read(&ctx), 9);
            upstream + value
        })
        .memo("result");
    assert_eq!(*result.read(&ctx), 7);
    assert_eq!(*result.read(&ctx), 7);
}

#[test]
fn shared_upstream_memo_in_a_diamond_computes_once_per_input() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let input = state(&ctx, "input", 2_u32);
    let calls = Cell::new(0);
    let base = input
        .map(|value| {
            calls.set(calls.get() + 1);
            value * 10
        })
        .memo("base");
    let left = base.map(|value| value + 1).memo("left");
    let right = base.map(|value| value + 2).memo("right");
    let total = (left, right).map(expensive_calculation).memo("total");
    assert_eq!(*total.read(&ctx), 43);
    assert_eq!(*total.read(&ctx), 43);
    assert_eq!(calls.get(), 1);
    *input.read_mut(&ctx) = 3;
    assert_eq!(*total.read(&ctx), 63);
    assert_eq!(calls.get(), 2);
}

#[test]
fn reading_an_intermediate_forces_only_its_prefix() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let input = state(&ctx, "input", 2_u32);
    let upstream_calls = Cell::new(0);
    let downstream_calls = Cell::new(0);
    let intermediate = input
        .map(|value| {
            upstream_calls.set(upstream_calls.get() + 1);
            value * 10
        })
        .memo("intermediate");
    let downstream = intermediate.map(|value| {
        downstream_calls.set(downstream_calls.get() + 1);
        value + 1
    });
    assert_eq!(*intermediate.read(&ctx), 20);
    assert_eq!(upstream_calls.get(), 1);
    assert_eq!(downstream_calls.get(), 0);
    assert_eq!(*downstream.read(&ctx), 21);
    assert_eq!(upstream_calls.get(), 1);
    assert_eq!(downstream_calls.get(), 1);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_tap_is_lazy_and_draws_as_a_normal_view() {
    use crate::{NotebookConfig, NotebookCore};
    use std::rc::Rc;

    let mut core = NotebookCore::new(NotebookConfig::new("tap-test"), Box::new(|_| {}));
    let input = state(&Context(&core.state_store), "input", 2_u32);
    let upstream_calls = Rc::new(Cell::new(0));
    let downstream_calls = Cell::new(0);
    let counter = upstream_calls.clone();
    let total = input
        .map(move |value| {
            counter.set(counter.get() + 1);
            value * 10
        })
        .memo("total");
    // This captured recipe is Clone, not Copy. Owning it does not read it.
    let tap = total.clone().tap();
    assert_eq!(upstream_calls.get(), 0);
    let mut notebook = core.build_notebook();
    notebook.view(tap);
    assert_eq!(notebook.cards.len(), 1);
    assert_eq!(upstream_calls.get(), 0);
    let downstream = total.map(|value| {
        downstream_calls.set(downstream_calls.get() + 1);
        value + 1
    });
    let ui = egui::Context::default();
    for _ in 0..2 {
        let output = ui.run(egui::RawInput::default(), |ctx| {
            assert!(core.draw_card(ctx, &mut notebook, 0, 480.0).is_some());
        });
        assert!(!output.shapes.is_empty());
    }
    assert_eq!(upstream_calls.get(), 1);
    assert_eq!(downstream_calls.get(), 0);
    assert_eq!(*downstream.read(&notebook), 21);
    assert_eq!(upstream_calls.get(), 1);
    assert_eq!(downstream_calls.get(), 1);
}

#[test]
fn a_panicking_map_invalidates_the_memo_and_does_not_poison_locks() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let input = state(&ctx, "input", 1_u32);
    let fail = Cell::new(false);
    let calls = Cell::new(0);
    let mapped = input
        .map(|value| {
            calls.set(calls.get() + 1);
            assert!(!fail.get(), "computation failed");
            value + 10
        })
        .memo("retry");
    assert_eq!(*mapped.read(&ctx), 11);
    *input.read_mut(&ctx) = 2;
    fail.set(true);
    assert!(catch_unwind(AssertUnwindSafe(|| mapped.read(&ctx))).is_err());
    *input.read_mut(&ctx) = 1;
    fail.set(false);
    assert_eq!(*mapped.read(&ctx), 11);
    assert_eq!(calls.get(), 3, "even the old key is retried after a panic");
    assert_eq!(*mapped.read(&ctx), 11);
    assert_eq!(calls.get(), 3);
}

#[test]
fn singleton_and_eight_input_tuples_are_supported() {
    let store = StateStore::default();
    let ctx = Context(&store);
    let value = state(&ctx, "value", 2_u32);
    assert_eq!(*(value,).map(|value| value + 1).read(&ctx), 3);
    let sum = (value, value, value, value, value, value, value, value)
        .map(|a, b, c, d, e, f, g, h| a + b + c + d + e + f + g + h)
        .memo("eight");
    assert_eq!(*sum.read(&ctx), 16);
}
