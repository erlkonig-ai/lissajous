//! Native synthetic pile -> immutable snapshot -> selected collection.
//! All fixture I/O and collection inspection finish before notebook painting.
//! The four cards receive narrow inputs and detach independently; there is no
//! central mutable controller, collection catalogue, or live/private pile.
//! Run with --headless --theme light --out-dir DIR [--width 320].

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use lissajous::widgets::triblespace::collection::{CollectionInfo, CollectionView};
    use lissajous::widgets::triblespace::pile::{PileCell, PileOpen};
    use lissajous::widgets::triblespace::snapshot::SnapshotView;
    use std::sync::Arc;
    use triblespace::core::blob::encodings::simplearchive::SimpleArchive;
    use triblespace::core::blob::encodings::succinctarchive::SuccinctArchiveBlob;
    use triblespace::core::collection::{
        succinctarchive_union, AdmissionPolicy, CollectionMap, CollectionPolicy, CollectionRecord,
    };
    use triblespace::core::metadata;
    use triblespace::core::repo::pile::Pile;
    use triblespace::prelude::inlineencodings::Handle;
    use triblespace::prelude::*;

    let mut config = lissajous::NotebookConfig::new("Native pile inspectors");
    let mut headless = false;
    let mut output = std::path::PathBuf::from("pile_inspectors_capture");
    let mut width = 320.0_f32;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--headless" => headless = true,
            "--out-dir" => output = args.next().ok_or("--out-dir needs a directory")?.into(),
            "--width" => width = args.next().ok_or("--width needs points")?.parse()?,
            "--theme" => {
                config = config.with_headless_theme(match args.next().as_deref() {
                    Some("light") => lissajous::HeadlessTheme::Light,
                    Some("dark") => lissajous::HeadlessTheme::Dark,
                    _ => return Err("--theme expects light or dark".into()),
                });
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    if !width.is_finite() || width < 16.0 {
        return Err("--width must be finite and at least 16".into());
    }
    if headless {
        config = config.with_headless_capture(output);
    }

    // Synthetic fixture only. The signing key has no external authority.
    let scratch = tempfile::tempdir()?;
    let path = scratch.path().join("synthetic.pile");
    std::fs::File::create(&path)?;
    let key = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
    let mut writer = Pile::open(&path)?;
    let selected = writer.collection(
        "synthetic",
        CollectionPolicy::new(
            AdmissionPolicy::direct(key.verifying_key()),
            AdmissionPolicy::direct(key.verifying_key()),
        ),
    )?;
    let index = writer.attach::<SuccinctArchiveBlob>(selected, ())?;
    let first = writer.commit(
        selected,
        &key,
        entity! { metadata::name: "synthetic first" },
    )?;
    writer.commit(
        selected,
        &key,
        entity! { metadata::name: "synthetic second" },
    )?;
    // Deliberately prepare only one attachment. This is fixture construction,
    // not inspector-side maintenance or a fallback when an index is absent.
    let fixture = writer.snapshot()?;
    let input: Blob<SimpleArchive> =
        fixture.get(Handle::<SimpleArchive>::from_hash(first.data()))?;
    let image = succinctarchive_union::derive_element(&input)?;
    let output = writer.put::<SuccinctArchiveBlob, _>(image)?;
    writer.insert(CollectionRecord::Map(CollectionMap::sign(
        &key,
        index.handle(),
        first.data(),
        Handle::<SuccinctArchiveBlob>::to_hash(output),
    )))?;
    writer.close()?;

    let source = Arc::new(PileCell::new(PileOpen {
        path,
        host: Some(key.verifying_key()),
    }));
    let read = source.wait(); // Preflight, never the paint path.
    if let Some(error) = read.error {
        return Err(error.to_string().into());
    }
    let published = Arc::new(read.snapshot.ok_or("no native snapshot published")?);
    // Select one typed collection through that exact native snapshot. No global
    // discovery or coverage_index() scan, no body decode or index maintenance.
    let collection = published.snapshot.as_ref().collection(selected)?;
    let info = CollectionInfo::observe(&collection);
    assert_eq!(info.members, 2);
    assert!(info.direct_bytes.as_ref().is_ok_and(|bytes| *bytes > 0));
    let inspected = Arc::new((collection, info));
    let attached = published.snapshot.as_ref().attached(index)?;
    let info = CollectionInfo::observe_attached(&attached);
    assert_eq!(info.members, 1);
    assert_eq!(
        info.parent.as_ref().unwrap().as_ref().unwrap().display(),
        "50.0% (1/2)"
    );
    let attached = Arc::new((attached, info));

    config.run(move |nb| {
        let source = Arc::clone(&source);
        nb.view(move |ctx| with_width(ctx, width, |ui| source.show(ui)));
        let published = Arc::clone(&published);
        nb.view(move |ctx| {
            with_width(ctx, width, |ui| {
                // The native-only constructor is equally valid here:
                // SnapshotView::new(published.snapshot.as_ref()).
                ui.add(SnapshotView::from_published(&published));
            })
        });
        let inspected = Arc::clone(&inspected);
        nb.view(move |ctx| {
            with_width(ctx, width, |ui| {
                ui.add(CollectionView::new(&inspected.1).name("synthetic"));
            })
        });
        let attached = Arc::clone(&attached);
        nb.view(move |ctx| {
            with_width(ctx, width, |ui| {
                ui.add(CollectionView::new(&attached.1).name("synthetic index"));
            })
        });
        nb.settled();
    })?;
    // Keep the scratch file alive for every native mapping and resource owner.
    drop(scratch);
    Ok(())
}

#[cfg(unix)]
fn with_width(ctx: &mut lissajous::CardCtx, width: f32, draw: impl FnOnce(&mut egui::Ui)) {
    let start = ctx.next_widget_position();
    ctx.ui_mut().scope_builder(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(start, egui::vec2(width, 0.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_width(width);
            draw(ui);
        },
    );
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The native pile inspector example currently requires Unix.");
}
