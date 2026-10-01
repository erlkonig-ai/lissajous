use lissajous::{notebook, NotebookCtx};

// Compile-only fixture: do not run it as part of macro resolution checks.
#[notebook(name = "Lissajous default dependency")]
fn main(_nb: &mut NotebookCtx) {}
