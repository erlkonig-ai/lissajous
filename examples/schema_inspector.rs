#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = "..", features = ["triblespace"] }
//! egui = "0.34"
//! triblespace = { path = "../../triblespace-rs" }
//! ```

use triblespace::core::blob::encodings::simplearchive::SimpleArchive;
use triblespace::core::blob::encodings::succinctarchive::SuccinctArchiveBlob;
use triblespace::core::blob::encodings::utf8string::UTF8String;
use triblespace::core::blob::encodings::wasmcode::WasmCode;
use triblespace::core::id::Id;
use triblespace::core::id::RawId;
use triblespace::core::inline::encodings::boolean::Boolean;
use triblespace::core::inline::encodings::ed25519::{
    ED25519PublicKey, ED25519RComponent, ED25519SComponent,
};
use triblespace::core::inline::encodings::f256::{F256BE, F256LE};
use triblespace::core::inline::encodings::f64::F64;
use triblespace::core::inline::encodings::genid::GenId;
use triblespace::core::inline::encodings::hash::{Blake3, Handle};
use triblespace::core::inline::encodings::iu256::{I256BE, I256LE, U256BE, U256LE};
use triblespace::core::inline::encodings::linelocation::LineLocation;
use triblespace::core::inline::encodings::r256::{R256BE, R256LE};
use triblespace::core::inline::encodings::range::{RangeInclusiveU128, RangeU128};
use triblespace::core::inline::encodings::shortstring::ShortString;
use triblespace::core::inline::encodings::time::NsTAIInterval;
use triblespace::core::inline::Inline;
use triblespace::core::metadata;
use triblespace::core::metadata::MetaDescribe;
use triblespace::core::repo::BlobStore;
use triblespace::core::repo::BlobStoreGet;
use triblespace::core::trible::{Fragment, TribleSet};
use triblespace::macros::{find, pattern};
use triblespace::prelude::View;

use lissajous::prelude::*;

fn build_schema_metadata() -> Fragment {
    let mut metadata = Fragment::empty();

    metadata += Boolean::describe();
    metadata += ShortString::describe();
    metadata += GenId::describe();
    metadata += F64::describe();
    metadata += F256LE::describe();
    metadata += F256BE::describe();
    metadata += U256LE::describe();
    metadata += U256BE::describe();
    metadata += I256LE::describe();
    metadata += I256BE::describe();
    metadata += R256LE::describe();
    metadata += R256BE::describe();
    metadata += RangeU128::describe();
    metadata += RangeInclusiveU128::describe();
    metadata += LineLocation::describe();
    metadata += NsTAIInterval::describe();
    metadata += ED25519RComponent::describe();
    metadata += ED25519SComponent::describe();
    metadata += ED25519PublicKey::describe();
    metadata += Blake3::describe();
    metadata += Handle::<UTF8String>::describe();
    metadata += Handle::<SimpleArchive>::describe();
    metadata += Handle::<SuccinctArchiveBlob>::describe();
    metadata += Handle::<WasmCode>::describe();

    metadata += UTF8String::describe();
    metadata += SimpleArchive::describe();
    metadata += SuccinctArchiveBlob::describe();
    metadata += WasmCode::describe();

    metadata
}

fn render_schema_sections(
    ui: &mut egui::Ui,
    title: &str,
    metadata_set: &TribleSet,
    blobs: &impl BlobStoreGet,
    kind: Id,
) {
    let id_color = ui.visuals().weak_text_color();
    let id_size = ui.text_style_height(&egui::TextStyle::Small);
    let body_size = ui.text_style_height(&egui::TextStyle::Body);
    let desc_size = ui.text_style_height(&egui::TextStyle::Small);
    let separator_stroke = egui::Stroke::new(1.0, ui.visuals().weak_text_color());

    let mut rows: Vec<(Id, String, String)> = find!(
        (
            id: Id,
            name: Inline<Handle<UTF8String>>,
            description: Inline<Handle<UTF8String>>
        ),
        pattern!(metadata_set, [{
            ?id @
                metadata::tag: kind,
                metadata::name: ?name,
                metadata::description: ?description
        }])
    )
    .into_iter()
    .filter_map(|(id, name, description)| {
        let name = blobs.get::<View<str>, UTF8String>(name).ok()?;
        let description = blobs.get::<View<str>, UTF8String>(description).ok()?;
        Some((id, name.to_string(), description.to_string()))
    })
    .collect();

    rows.sort_by(|left, right| {
        let left_name = left.1.as_str();
        let right_name = right.1.as_str();
        let left_id: &RawId = left.0.as_ref();
        let right_id: &RawId = right.0.as_ref();
        left_name
            .cmp(right_name)
            .then_with(|| left_id.cmp(right_id))
    });

    ui.label(egui::RichText::new(title).heading());
    ui.add_space(6.0);

    for (idx, row) in rows.iter().enumerate() {
        let name = row.1.as_str();
        let description = row.2.as_str();

        let row_height = body_size.max(id_size);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), row_height),
            egui::Layout::left_to_right(egui::Align::BOTTOM),
            |ui| {
                ui.label(
                    egui::RichText::new(name)
                        .monospace()
                        .size(body_size)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("{:X}", row.0))
                        .monospace()
                        .size(id_size)
                        .color(id_color),
                );
            },
        );
        ui.label(
            egui::RichText::new(description)
                .size(desc_size)
                .color(ui.visuals().text_color()),
        );
        if idx + 1 < rows.len() {
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
            ui.painter()
                .hline(rect.x_range(), rect.center().y, separator_stroke);
        } else {
            ui.add_space(6.0);
        }
    }
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let padding = lissajous::cards::DEFAULT_CARD_PADDING;
    let mut metadata = build_schema_metadata();
    let reader = metadata.blobs_mut().reader().expect("metadata blob reader");
    let metadata_set = metadata.into_facts();

    nb.view(move |ctx| {
        ctx.with_padding(padding, |ctx| {
            ctx.label(egui::RichText::new("Schema metadata").heading());
            ctx.label("Built-in value and blob schemas with their discovery metadata.");
            ctx.add_space(6.0);
            ctx.separator();
            ctx.add_space(12.0);
            render_schema_sections(
                ctx,
                "Value schemas",
                &metadata_set,
                &reader,
                metadata::KIND_INLINE_ENCODING,
            );
            ctx.add_space(8.0);
            render_schema_sections(
                ctx,
                "Blob schemas",
                &metadata_set,
                &reader,
                metadata::KIND_BLOB_ENCODING,
            );
        });
    });
}
