//! A source cell publishes a native snapshot; another cell consumes its value.
//! Unix only. Set LISSAJOUS_PILE to an existing scratch/public pile.
//! Requires the exact Core source with refresh_next; see README's pile section.

#[cfg(unix)]
use lissajous::prelude::*;

#[cfg(unix)]
#[notebook]
fn main(nb: &mut NotebookCtx) {
    use lissajous::widgets::triblespace::pile::{PileCell, PileOpen};

    let source = nb.state(
        "pile-resource",
        || {
            let path = std::env::var_os("LISSAJOUS_PILE")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "data.pile".into());
            // None believes no host's MERGEs. A real application supplies its
            // explicit public host; the cell never discovers a signing key.
            PileCell::new(PileOpen { path, host: None })
        },
        |ctx, resource| resource.show(ctx),
    );

    nb.view(move |ctx| {
        // Copy the output and release the resource's state guard. An expensive
        // consumer would send this snapshot to its OWN keyed background task.
        let read = { source.read(ctx).read() };
        ctx.heading("Independent snapshot consumer");
        if let Some(published) = read.and_then(|read| read.snapshot) {
            ctx.label(format!(
                "Last successful immutable prefix: {} bytes",
                published.snapshot.prefix_len()
            ));
        } else {
            ctx.label("No successful snapshot yet");
        }
    });
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The pile-resource example currently supports Unix file identity only.");
}
