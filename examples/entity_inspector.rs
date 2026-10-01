#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = "..", features = ["triblespace"] }
//! egui = "0.34"
//! eframe = "0.34"
//! triblespace = { path = "../../triblespace-rs", features = ["wasm"] }
//! ```

use eframe::egui;
use triblespace::core::blob::encodings::wasmcode::WasmCode;
use triblespace::core::blob::BlobCache;
use triblespace::core::examples::literature;
use triblespace::core::id::ExclusiveId;
use triblespace::core::id::Id;
use triblespace::core::inline::encodings::hash::Handle;
use triblespace::core::repo::memoryrepo::MemoryRepo;
use triblespace::core::repo::BlobStore;
use triblespace::core::repo::BlobStorePut;
use triblespace::core::value_formatter::WasmValueFormatter;
use triblespace::prelude::blobencodings::UTF8String;
use triblespace::prelude::inlineencodings::{GenId, ShortString, R256};
use triblespace::prelude::{entity, MetaDescribe, TribleSet, View};

use lissajous::prelude::*;
use lissajous::widgets::triblespace::{id_short, EntityInspectorWidget};

mod demo {
    use triblespace::prelude::*;

    // A tiny synthetic schema so we can render human-friendly rows and references.
    attributes! {
        "B603E10B4BBF45B7A1BA0B7D9FA2D001" as pub name: inlineencodings::ShortString;
        "B603E10B4BBF45B7A1BA0B7D9FA2D002" as pub isa: inlineencodings::GenId;
    }
}

fn build_demo_space() -> (TribleSet, TribleSet, MemoryRepo, Id) {
    let mut data = TribleSet::new();
    let mut metadata = TribleSet::new();
    let mut storage = MemoryRepo::default();

    let name = demo::name.id();
    let isa = demo::isa.id();
    let lit_title = literature::title.id();
    let lit_author = literature::author.id();
    let lit_firstname = literature::firstname.id();
    let lit_lastname = literature::lastname.id();
    let lit_quote = literature::quote.id();
    let lit_page_count = literature::page_count.id();

    let schema_genid = <GenId as MetaDescribe>::id();
    let schema_shortstring = <ShortString as MetaDescribe>::id();
    let schema_handle = <Handle<UTF8String> as MetaDescribe>::id();
    let schema_r256 = <R256 as MetaDescribe>::id();
    for (attr, name, schema) in [
        (name, "name", schema_shortstring),
        (isa, "isa", schema_genid),
        (lit_title, "title", schema_shortstring),
        (lit_author, "author", schema_genid),
        (lit_firstname, "firstname", schema_shortstring),
        (lit_lastname, "lastname", schema_shortstring),
        (lit_quote, "quote", schema_handle),
        (lit_page_count, "page_count", schema_r256),
    ] {
        let name_handle = storage
            .put::<UTF8String, _>(name.to_string())
            .expect("name handle");
        metadata += entity! { ExclusiveId::force_ref(&attr) @
            triblespace::core::metadata::name: name_handle,
            triblespace::core::metadata::value_encoding: schema,
        };
    }

    metadata += GenId::describe();
    metadata += <Handle<UTF8String> as MetaDescribe>::describe();
    metadata += R256::describe();
    metadata += ShortString::describe();

    fn demo_id(seed: u16) -> Id {
        let mut raw = [0u8; 16];
        raw[14..16].copy_from_slice(&seed.to_be_bytes());
        Id::new(raw).expect("demo ids are non-zero")
    }

    let e_author_kind = demo_id(0xC001);
    let e_book_kind = demo_id(0xC002);
    data += entity! { ExclusiveId::force_ref(&e_author_kind) @ demo::name: "Author" };
    data += entity! { ExclusiveId::force_ref(&e_book_kind) @ demo::name: "Book" };

    let authors = [
        ("Frank", "Herbert"),
        ("Isaac", "Asimov"),
        ("Mary", "Shelley"),
        ("Jane", "Austen"),
        ("Herman", "Melville"),
        ("Homer", ""),
        ("William", "Shakespeare"),
        ("Jules", "Verne"),
        ("George", "Orwell"),
        ("Virginia", "Woolf"),
        ("Fyodor", "Dostoevsky"),
        ("Leo", "Tolstoy"),
        ("Miguel", "Cervantes"),
        ("Franz", "Kafka"),
        ("Mark", "Twain"),
        ("Oscar", "Wilde"),
    ];

    let mut author_ids = Vec::with_capacity(authors.len());
    for (idx, (first, last)) in authors.iter().enumerate() {
        let id = demo_id(0xA000 + idx as u16);
        author_ids.push(id);
        let full_name = if last.is_empty() {
            (*first).to_string()
        } else {
            format!("{first} {last}")
        };
        let mut author = entity! { ExclusiveId::force_ref(&id) @
            demo::name: full_name,
            demo::isa: e_author_kind,
            literature::firstname: *first,
        };
        if !last.is_empty() {
            author += entity! { ExclusiveId::force_ref(&id) @
                literature::lastname: *last,
            };
        }
        data += author;
    }

    let books = [
        (
            "Dune",
            0,
            "Deep in the human unconscious is a need for a logical universe.",
            412,
        ),
        (
            "Dune Messiah",
            0,
            "He shall know your ways as if born to them.",
            256,
        ),
        (
            "Foundation",
            1,
            "Violence is the last refuge of the incompetent.",
            255,
        ),
        ("I, Robot", 1, "A robot may not injure a human being.", 224),
        (
            "Frankenstein",
            2,
            "Beware; for I am fearless, and therefore powerful.",
            280,
        ),
        (
            "The Last Man",
            2,
            "My imagination was the only reality.",
            360,
        ),
        (
            "Pride and Prejudice",
            3,
            "It is a truth universally acknowledged.",
            279,
        ),
        (
            "Sense and Sensibility",
            3,
            "What do you know of my heart?",
            240,
        ),
        ("Moby Dick", 4, "Call me Ishmael.", 635),
        ("Billy Budd", 4, "The sea had jeered at it all.", 192),
        (
            "Odyssey",
            5,
            "Tell me, O Muse, of the man of many ways.",
            500,
        ),
        ("Iliad", 5, "Sing, goddess, the anger of Achilles.", 480),
        (
            "Hamlet",
            6,
            "To be, or not to be, that is the question.",
            200,
        ),
        (
            "The Tempest",
            6,
            "We are such stuff as dreams are made on.",
            200,
        ),
        ("Twenty Thousand Leagues", 7, "The sea is everything.", 300),
        (
            "Journey to the Center",
            7,
            "Science, my boy, is made up of mistakes.",
            300,
        ),
        ("1984", 8, "Big Brother is watching you.", 328),
        (
            "Animal Farm",
            8,
            "All animals are equal, but some are more equal.",
            112,
        ),
        (
            "Mrs Dalloway",
            9,
            "Mrs. Dalloway said she would buy the flowers herself.",
            296,
        ),
        ("To the Lighthouse", 9, "Nothing was simply one thing.", 209),
        (
            "Crime and Punishment",
            10,
            "The darker the night, the brighter the stars.",
            671,
        ),
        ("The Idiot", 10, "Beauty will save the world.", 656),
        (
            "War and Peace",
            11,
            "Well, Prince, so Genoa and Lucca are now just family estates.",
            1225,
        ),
        ("Anna Karenina", 11, "All happy families are alike.", 864),
        (
            "Don Quixote",
            12,
            "The truth may be stretched, but cannot be broken.",
            863,
        ),
        (
            "Metamorphosis",
            13,
            "When Gregor Samsa awoke, he found himself changed.",
            201,
        ),
        ("The Trial", 13, "Someone must have slandered Josef K.", 255),
        (
            "Tom Sawyer",
            14,
            "Tom appeared on the sidewalk with a bucket of whitewash.",
            274,
        ),
        (
            "Huckleberry Finn",
            14,
            "You do not know about me without you have read a book.",
            366,
        ),
        (
            "Dorian Gray",
            15,
            "The only way to get rid of a temptation is to yield to it.",
            254,
        ),
        (
            "Earnest",
            15,
            "The truth is rarely pure and never simple.",
            180,
        ),
    ];

    for (idx, (title, author_idx, quote, pages)) in books.iter().enumerate() {
        let id = demo_id(0xB000 + idx as u16);
        let author_id = author_ids.get(*author_idx).copied().expect("author index");
        let quote_handle = storage.put::<UTF8String, _>(*quote).expect("quote handle");
        data += entity! { ExclusiveId::force_ref(&id) @
            demo::name: *title,
            demo::isa: e_book_kind,
            literature::title: *title,
            literature::author: author_id,
            literature::quote: quote_handle,
            literature::page_count: *pages as i128,
        };
    }

    (data, metadata, storage, demo_id(0xB000))
}

#[derive(Debug)]
struct InspectorState {
    selected: Id,
    columns: usize,
    node_count: usize,
}

impl Default for InspectorState {
    fn default() -> Self {
        use triblespace::macros::id_hex;
        Self {
            selected: id_hex!("11111111111111111111111111111111"),
            columns: 0,
            node_count: 0,
        }
    }
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let padding = lissajous::cards::DEFAULT_CARD_PADDING;
    nb.view(move |ui| {
        md!(
            ui,
            "# Hi Triblespace entity inspector (prototype)\n\nTables-first tiled layout, with orthogonal “subway” routing through gutters.\n\nClick a table to select."
        );
    });

    let (data, metadata, mut storage, default_selected) = build_demo_space();
    let reader = storage.reader().expect("demo blob store reader");
    let formatter_cache: BlobCache<_, WasmCode, WasmValueFormatter> =
        BlobCache::new(reader.clone());
    let name_cache: BlobCache<_, UTF8String, View<str>> = BlobCache::new(reader);
    let inspector = nb.state(
        "inspector",
        InspectorState {
            selected: default_selected,
            columns: 0,
            node_count: 0,
        },
        move |ui, state| {
            ui.with_padding(padding, |ctx| {
                ctx.horizontal(|ctx| {
                    ctx.label(egui::RichText::new("COLUMNS").monospace().strong());
                    let max_columns = state.node_count.max(1);
                    let constrain = |_: usize, next: usize| next.min(max_columns);
                    ctx.add(
                        widgets::NumberField::new(&mut state.columns)
                            .speed(0.25)
                            .constrain_value(&constrain),
                    );
                    ctx.label(egui::RichText::new("(0 = auto)").monospace().weak());
                });
                ctx.add_space(8.0);

                let response = EntityInspectorWidget::new(
                    &data,
                    &metadata,
                    &name_cache,
                    &formatter_cache,
                    &mut state.selected,
                )
                .columns(state.columns)
                .show(ctx);

                let stats = response.stats;
                state.node_count = stats.nodes;
                let metrics = format!(
                    "_{} nodes, {} edges ({} components), {} columns._\n\
_Canvas: {:.0}×{:.0}px • Tiles: {:.0}%._\n\
_Order: {:.0} 1D dist total • {:.1} avg._\n\
_Wire: {:.0}px total • {:.0}px avg (max {:.0}px)._\n\
_Routing: {:.1} turns avg (max {}) • span {:.1} cols (max {}) • {} left • {} fallback._",
                    stats.nodes,
                    stats.edges,
                    stats.connected_components,
                    stats.columns,
                    stats.canvas_width,
                    stats.canvas_height,
                    stats.tile_coverage * 100.0,
                    stats.linear_total,
                    stats.linear_avg,
                    stats.total_edge_len,
                    stats.avg_edge_len,
                    stats.max_edge_len,
                    stats.avg_turns,
                    stats.max_turns,
                    stats.avg_span_cols,
                    stats.max_span_cols,
                    stats.left_edges,
                    stats.fallback_tracks,
                );
                ctx.markdown(&metrics);
                ctx.add_space(8.0);
            });
        },
    );

    nb.view(move |ui| {
        let selected = inspector.read(ui).selected;
        md!(ui, "Selected entity: `{}`", id_short(selected));
    });
}
