#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = "..", features = ["triblespace"] }
//! ed25519-dalek = "2.1"
//! pollster = "0.4"
//! tempfile = "3.26"
//! triblespace = { path = "../../triblespace-rs" }
//! ```
//!
//! A native, stateful notebook card which observes collection commits appended
//! through a second handle to the same Pile. The card polls a cheap store
//! snapshot at an absolute deadline; it does not own a worker thread.

use std::fs::File;
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use eframe::egui;
use tempfile::TempDir;
use triblespace::core::blob::encodings::simplearchive::SimpleArchive;
use triblespace::core::blob::encodings::succinctarchive::{
    OrderedUniverse, Rank9AcceleratedSuccinctArchiveBlob, SuccinctArchiveBlob, UnionArchive,
};
use triblespace::core::collection::{
    AdmissionPolicy, Collection, CollectionPolicy, CollectionSnapshotExt, CollectionStoreExt,
    Support,
};
use triblespace::core::examples::literature;
use triblespace::core::repo::pile::{Pile, PileSnapshot};
use triblespace::core::repo::{StoreChanges, StoreSnapshot};
use triblespace::prelude::*;

use lissajous::cards::DEFAULT_CARD_PADDING;
use lissajous::prelude::*;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MORE_TITLES: [&str; 4] = [
    "Dune Messiah",
    "Children of Dune",
    "God Emperor of Dune",
    "Heretics of Dune",
];

#[derive(Debug, Eq, PartialEq)]
enum Observation {
    NoCollectionChange,
    Folded(usize),
}

enum DemoError {
    Publication(String),
    Observation(String),
}

impl DemoError {
    fn message(&self) -> &str {
        match self {
            Self::Publication(message) | Self::Observation(message) => message,
        }
    }
}

struct Demo {
    observer: Pile,
    writer: Pile,
    collection: Collection<SimpleArchive>,
    raw: Collection<SuccinctArchiveBlob>,
    accelerated: Collection<Rank9AcceleratedSuccinctArchiveBlob>,
    signing_key: SigningKey,
    author: Id,
    checkpoint: Option<Support>,
    acknowledged_snapshot: Option<PileSnapshot>,
    observed_titles: Vec<String>,
    next_title: usize,
    next_probe: Option<Instant>,
    last_status: String,
    error: Option<DemoError>,
    // Fields drop in declaration order. Keep the directory last so both Pile
    // handles release the file before TempDir removes its parent directory.
    _directory: TempDir,
}

impl Demo {
    fn new() -> Result<Self, String> {
        let directory = tempfile::tempdir()
            .map_err(|error| format!("could not create a temporary directory: {error}"))?;
        let pile_path = directory.path().join("incremental-collection.pile");
        File::create(&pile_path)
            .map_err(|error| format!("could not create {}: {error}", pile_path.display()))?;

        // A deterministic demo key makes reruns legible; the temporary pile is
        // isolated per process and this key must not be reused as real authority.
        let signing_key = SigningKey::from_bytes(&[0x47; 32]);
        let authority = signing_key.verifying_key();
        let name = "gorbie-incremental-literature";
        let policy = CollectionPolicy::new(
            AdmissionPolicy::direct(authority),
            AdmissionPolicy::direct(authority),
        );

        // Open the observer before publishing the initial commits. Seeing them
        // therefore exercises Pile's external-append refresh path immediately.
        let observer = Pile::open(&pile_path)
            .map_err(|error| format!("could not open observer pile: {error}"))?;
        let mut writer = Pile::open(&pile_path)
            .map_err(|error| format!("could not open writer pile: {error}"))?;
        let collection = writer
            .collection(name, policy.clone())
            .map_err(|error| format!("could not register the collection: {error}"))?;

        let author = entity! {
            literature::firstname: "Frank",
            literature::lastname: "Herbert",
        };
        let author_id = author
            .root()
            .ok_or_else(|| "the intrinsic author fragment has no root".to_owned())?;
        writer
            .commit(collection, &signing_key, author)
            .map_err(|error| format!("could not publish the author: {error}"))?;
        writer
            .commit(
                collection,
                &signing_key,
                entity! {
                    literature::title: "Dune",
                    literature::author: &author_id,
                },
            )
            .map_err(|error| format!("could not publish the initial book: {error}"))?;

        let raw = writer
            .derive::<SuccinctArchiveBlob>(collection, (), policy.clone())
            .map_err(|error| format!("could not register the Succinct collection: {error}"))?;
        let accelerated = writer
            .derive::<Rank9AcceleratedSuccinctArchiveBlob>(raw, (), policy)
            .map_err(|error| {
                format!("could not register the Rank9-accelerated collection: {error}")
            })?;
        Ok(Self {
            observer,
            writer,
            collection,
            raw,
            accelerated,
            signing_key,
            author: author_id,
            checkpoint: None,
            acknowledged_snapshot: None,
            observed_titles: Vec::new(),
            next_title: 0,
            next_probe: Some(Instant::now()),
            last_status: "Waiting for the initial observation".to_owned(),
            error: None,
            _directory: directory,
        })
    }

    fn next_title(&self) -> Option<&'static str> {
        MORE_TITLES.get(self.next_title).copied()
    }

    fn publish_next(&mut self) -> Result<&'static str, String> {
        let title = self
            .next_title()
            .ok_or_else(|| "all example commits have already been published".to_owned())?;
        self.writer
            .commit(
                self.collection,
                &self.signing_key,
                entity! {
                    literature::title: title,
                    literature::author: &self.author,
                },
            )
            .map_err(|error| format!("could not publish {title:?}: {error}"))?;
        self.next_title += 1;
        Ok(title)
    }

    fn observe(
        &mut self,
        mut consume: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<Observation, String> {
        // This is the observation boundary. If another handle appends after it,
        // acknowledging exactly `sampled` leaves that later write for the next
        // probe rather than accidentally swallowing it.
        let sampled = self
            .observer
            .snapshot()
            .map_err(|error| format!("could not sample the Pile snapshot: {error}"))?;
        if let Some(previous) = self.acknowledged_snapshot.as_ref() {
            let changes = sampled.changes_since(previous);
            if !changes.contains(StoreChanges::COLLECTION_RECORDS) {
                // This local demo has a direct, timeless policy and publishes
                // all blobs before their commits. A networked/delegated reader
                // must also react to residency, proof, and validity changes.
                self.acknowledged_snapshot = Some(sampled);
                return Ok(Observation::NoCollectionChange);
            }
        }

        let current = self
            .collection
            .admitted(&sampled)
            .map_err(|error| format!("could not discover the collection cover: {error}"))?;
        let added = match self.checkpoint.as_ref() {
            Some(previous) => current
                .additions_since(previous)
                .map_err(|error| format!("cover is not an additions-only advance: {error}"))?,
            None => current.clone(),
        };

        // Other collections' MERGE/DERIVE records share the same Pile-level
        // snapshot component. Once the exact source cover says there is no
        // semantic delta, acknowledge the sampled false positive without
        // touching either archive representation.
        if added.is_empty() {
            self.acknowledged_snapshot = Some(sampled);
            return Ok(Observation::NoCollectionChange);
        }

        let maintained = pollster::block_on(async {
            self.observer
                .maintain_exact(self.raw, &self.signing_key, &current)
                .await
                .map_err(|error| format!("could not maintain the raw Succinct view: {error}"))?;
            self.observer
                .maintain_exact(self.accelerated, &self.signing_key, &current)
                .await
                .map_err(|error| {
                    format!("could not maintain the Rank9-accelerated Succinct view: {error}")
                })
        })?;
        let full = maintained
            .collection_exact(self.accelerated, &current)
            .map_err(|error| format!("could not attach the Succinct full view: {error}"))?
            .view::<UnionArchive<OrderedUniverse>>()
            .map_err(|error| format!("could not read the Succinct full view: {error}"))?;
        let changed = added
            .materialize::<TribleSet, _>(&sampled)
            .map_err(|error| format!("could not attach the SimpleArchive delta: {error}"))?;

        let titles: Vec<String> = find!(
            title: String,
            pattern_changes!(&full, &changed, [
                { _?author @ literature::firstname: "Frank" },
                { _?book @
                    literature::author: _?author,
                    literature::title: ?title
                }
            ])
        )
        .collect();

        for title in &titles {
            consume(title)?;
        }

        // The consumer continuation and sampled snapshot advance only after
        // every fallible step and the complete fold succeeded. A failure
        // remains retryable; advancing the immutable full-view cache is safe.
        let count = titles.len();
        self.observed_titles.extend(titles);
        self.checkpoint = Some(current);
        self.acknowledged_snapshot = Some(sampled);
        Ok(Observation::Folded(count))
    }

    fn advance_probe_deadline(&mut self, previous_deadline: Instant) {
        let finished = Instant::now();
        let elapsed = finished.saturating_duration_since(previous_deadline);
        let steps = elapsed.as_nanos() / POLL_INTERVAL.as_nanos() + 1;
        let advanced = u32::try_from(steps)
            .ok()
            .and_then(|steps| previous_deadline.checked_add(POLL_INTERVAL * steps))
            .filter(|deadline| *deadline > finished)
            .unwrap_or(finished + POLL_INTERVAL);
        self.next_probe = Some(advanced);
    }
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.state_with("incremental-collection", Demo::new, |ctx, state| {
        ctx.with_padding(DEFAULT_CARD_PADDING, |ctx| match state {
            Err(error) => {
                ctx.heading("Incremental collection");
                ctx.label(
                    egui::RichText::new(format!("Initialization failed: {error}"))
                        .color(egui::Color32::RED),
                );
                if ctx.button("Retry initialization").clicked() {
                    *state = Demo::new();
                }
            }
            Ok(demo) => {
                ctx.heading("Incremental collection");
                ctx.label("The observer and writer are separate handles to one append-only Pile.");
                ctx.label(
                    egui::RichText::new(
                        "The demo writer is the collection authority. The 100 ms gate does not \
                         model delegated-capability expiry; authorization clocks require their own \
                         wake-up policy even when no collection record changes.",
                    )
                    .weak(),
                );
                if let Some(title) = demo.next_title() {
                    if ctx
                        .button(format!("Publish {title:?} through writer"))
                        .clicked()
                    {
                        match demo.publish_next() {
                            Ok(title) => {
                                demo.last_status =
                                    format!("Published {title:?}; observation pending");
                                if matches!(demo.error.as_ref(), Some(DemoError::Publication(_))) {
                                    demo.error = None;
                                }
                                demo.next_probe = Some(Instant::now());
                            }
                            Err(error) => demo.error = Some(DemoError::Publication(error)),
                        }
                    }
                } else {
                    ctx.label("All example commits have been published.");
                }

                let now = Instant::now();
                if let Some(deadline) = demo.next_probe.filter(|deadline| now >= *deadline) {
                    match demo.observe(|_| Ok(())) {
                        Ok(Observation::Folded(count)) => {
                            demo.last_status = format!("Folded {count} newly supported title(s)");
                            if matches!(demo.error.as_ref(), Some(DemoError::Observation(_))) {
                                demo.error = None;
                            }
                            demo.advance_probe_deadline(deadline);
                        }
                        Ok(Observation::NoCollectionChange) => {
                            demo.last_status = "Collection is up to date".to_owned();
                            if matches!(demo.error.as_ref(), Some(DemoError::Observation(_))) {
                                demo.error = None;
                            }
                            demo.advance_probe_deadline(deadline);
                        }
                        Err(error) => {
                            demo.last_status =
                                "Observation failed; retry will fold the same delta".to_owned();
                            demo.error = Some(DemoError::Observation(error));
                            demo.next_probe = None;
                        }
                    }
                }

                ctx.label(&demo.last_status);
                if let Some(error) = &demo.error {
                    ctx.label(
                        egui::RichText::new(error.message())
                            .color(egui::Color32::RED)
                            .monospace(),
                    );
                    if matches!(error, DemoError::Observation(_))
                        && ctx.button("Retry observation now").clicked()
                    {
                        demo.next_probe = Some(Instant::now());
                    }
                }

                ctx.separator();
                ctx.label(format!(
                    "Checkpoint: {} payload member(s); observed incremental rows:",
                    demo.checkpoint.as_ref().map_or(0, Support::len)
                ));
                for title in &demo.observed_titles {
                    ctx.label(format!("• {title}"));
                }

                // The deadline is retained in state. Unrelated repaints do not
                // postpone the probe by resetting a relative 100 ms timer.
                if let Some(deadline) = demo.next_probe {
                    ctx.ctx()
                        .request_repaint_after(deadline.saturating_duration_since(Instant::now()));
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_external_appends_and_retries_a_failed_fold() {
        let mut demo = Demo::new().expect("demo initialization");

        assert_eq!(demo.observe(|_| Ok(())).unwrap(), Observation::Folded(1));
        assert_eq!(demo.observed_titles, ["Dune"]);
        assert_eq!(
            demo.observe(|_| Ok(())).unwrap(),
            Observation::NoCollectionChange
        );
        assert_eq!(demo.observed_titles, ["Dune"]);

        demo.publish_next().expect("writer commit");
        let checkpoint_before_failure = demo.checkpoint.clone();
        let snapshot_before_failure = demo.acknowledged_snapshot.clone();
        let titles_before_failure = demo.observed_titles.clone();

        let error = demo
            .observe(|_| Err("simulated sink failure".to_owned()))
            .expect_err("the consumer must fail");
        assert_eq!(error, "simulated sink failure");
        assert_eq!(demo.checkpoint, checkpoint_before_failure);
        let snapshot_after_failure = demo
            .acknowledged_snapshot
            .as_ref()
            .expect("the prior snapshot remains acknowledged");
        let snapshot_before_failure = snapshot_before_failure
            .as_ref()
            .expect("the initial observation acknowledges a snapshot");
        assert_eq!(
            snapshot_after_failure.changes_since(snapshot_before_failure),
            StoreChanges::NONE
        );
        assert_eq!(
            snapshot_before_failure.changes_since(snapshot_after_failure),
            StoreChanges::NONE
        );
        assert_eq!(demo.observed_titles, titles_before_failure);

        assert_eq!(demo.observe(|_| Ok(())).unwrap(), Observation::Folded(1));
        assert_eq!(demo.observed_titles, ["Dune", "Dune Messiah"]);
    }
}
