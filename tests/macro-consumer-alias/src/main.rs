use GORBIE::{notebook, NotebookCtx};

// Compile-only fixture: the attribute must expand through the Cargo alias.
#[notebook(name = "Lissajous legacy dependency alias")]
fn main(_nb: &mut NotebookCtx) {}
