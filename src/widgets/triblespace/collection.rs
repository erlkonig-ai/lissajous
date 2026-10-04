//! Compact inspection of native collection observations, without logical reads.
//!
//! Build [`CollectionInfo`] outside immediate-mode painting, then retain it with
//! the exact immutable snapshot/request that produced it. Metadata lookup can
//! perform I/O (including member validation); even this small projection belongs
//! on the consumer's worker for a large cover. [`CollectionView`] only paints
//! scalar results. Neither layer materializes a logical collection, maps missing
//! data, performs maintenance, acquires remote bytes, or writes to the store.

use super::inspector_bytes::compact_bytes;
use triblespace::core::collection::{
    AttachedSnapshot, CollectionEncoding, CollectionHandle, CollectionSnapshot, Cover,
};
use triblespace::core::repo::{BlobStoreMeta, StoreRead};

/// Foundations and the exact collection whose lattice contains them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportInfo {
    pub collection: CollectionHandle,
    pub foundations: usize,
}

/// Intersection with the direct parent's admitted foundations in the SAME
/// snapshot. This is local known-prefix coverage, not global completeness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentCoverage {
    pub covered: usize,
    pub foundations: usize,
}

impl ParentCoverage {
    /// No percentage exists for an empty denominator; it is not 100% complete.
    pub fn percentage(&self) -> Option<f64> {
        (self.foundations != 0).then(|| self.covered as f64 * 100.0 / self.foundations as f64)
    }

    pub fn display(&self) -> String {
        let percentage = self
            .percentage()
            .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.1}%"));
        format!("{percentage} ({}/{})", self.covered, self.foundations)
    }
}

/// A small, immutable display projection, not a catalog or a logical value.
///
/// A successful zero is distinct from an unavailable observation. Independent
/// fields remain useful when metadata or support fails. Keep this value paired
/// with its source snapshot in retained consumer state; a newer pile publication
/// must not relabel an older result as current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionInfo {
    pub collection: CollectionHandle,
    pub members: usize,
    /// Sum of direct cover-member lengths only. Excludes descriptors, record
    /// frames, provenance metadata, referenced attachments and dependencies.
    pub direct_bytes: Result<u64, String>,
    /// None for a raw cover: a coordinate alone makes no support claim.
    pub support: Option<Result<SupportInfo, String>>,
    /// Present only for an attached observation, never arbitrary collections.
    pub parent: Option<Result<ParentCoverage, String>>,
    resident: bool,
}

impl CollectionInfo {
    /// Inspect an already selected native collection observation.
    /// Ordinary support belongs to this collection, including for derived views.
    pub fn observe<R: StoreRead, E: CollectionEncoding>(
        observed: &CollectionSnapshot<R, E>,
    ) -> Self {
        let mut info = Self::observe_cover(observed.snapshot(), observed.cover());
        info.resident = true;
        info.support = Some(
            observed
                .support()
                .map(|support| SupportInfo {
                    collection: support.collection().handle(),
                    foundations: support.len(),
                })
                .map_err(|error| format!("cover support unavailable: {error}")),
        );
        info
    }

    /// Inspect exactly the selected attachment, including partial/empty covers.
    /// The support's own collection identifies the direct parent; its admitted
    /// foundations and the intersection are queried through the same snapshot.
    pub fn observe_attached<R: StoreRead, E: CollectionEncoding>(
        observed: &AttachedSnapshot<R, E>,
    ) -> Self {
        let mut info = Self::observe_cover(observed.snapshot(), observed.cover());
        info.resident = true;
        let support = observed.support();
        info.support = Some(Ok(SupportInfo {
            collection: support.collection().handle(),
            foundations: support.len(),
        }));
        info.parent = Some((|| {
            let parent = support
                .collection()
                .admitted(observed.snapshot())
                .map_err(|error| format!("parent foundations unavailable: {error}"))?;
            let covered = parent
                .intersection(support)
                .map_err(|error| format!("parent support comparison unavailable: {error}"))?;
            Ok(ParentCoverage {
                covered: covered.len(),
                foundations: parent.len(),
            })
        })());
        info
    }

    /// Inspect a caller-supplied exact cover using metadata only.
    ///
    /// This lower-level coordinate does not establish admission, residency or
    /// support. The face therefore says "cover members", not "resident members".
    /// Pass the immutable reader against which the caller selected this cover.
    pub fn observe_cover<R: BlobStoreMeta, E: CollectionEncoding>(
        snapshot: &R,
        cover: &Cover<E>,
    ) -> Self {
        Self {
            collection: cover.collection().handle(),
            members: cover.len(),
            direct_bytes: direct_bytes(snapshot, cover),
            support: None,
            parent: None,
            resident: false,
        }
    }

    /// Whether the cover came from a native resident observation, rather than
    /// a caller-supplied coordinate. This is not a freshness test.
    pub fn is_resident_observation(&self) -> bool {
        self.resident
    }
}

fn direct_bytes<R: BlobStoreMeta, E: CollectionEncoding>(
    snapshot: &R,
    cover: &Cover<E>,
) -> Result<u64, String> {
    cover.members().try_fold(0u64, |sum, member| {
        let metadata = snapshot
            .metadata(member)
            .map_err(|error| {
                format!(
                    "cover-member metadata error for {}: {error}",
                    hex(&member.raw)
                )
            })?
            .ok_or_else(|| format!("cover-member metadata unavailable for {}", hex(&member.raw)))?;
        sum.checked_add(metadata.length)
            .ok_or_else(|| "direct cover-member byte total exceeds u64".to_owned())
    })
}

/// A square-edged, data-only instrument. Its coverage is a static relationship,
/// not loading progress: there is no animated rail, spinner or completion state.
pub struct CollectionView<'a> {
    info: &'a CollectionInfo,
    name: Option<&'a str>,
}

impl<'a> CollectionView<'a> {
    pub fn new(info: &'a CollectionInfo) -> Self {
        Self { info, name: None }
    }

    /// Optional caller-supplied label; identity always remains visible on hover.
    pub fn name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }
}

impl egui::Widget for CollectionView<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let identity = hex(&self.info.collection.raw);
        let stroke = egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color());
        egui::Frame::new()
            .corner_radius(0.0)
            .stroke(stroke)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 2.0).max(0.0));
                ui.label(
                    egui::RichText::new(self.name.unwrap_or(&identity[..16]))
                        .monospace()
                        .size(11.0),
                )
                .on_hover_text(format!("Collection: {identity}"));
                let members_label = if self.info.resident {
                    "resident members"
                } else {
                    "cover members"
                };
                row(ui, members_label, Ok(self.info.members.to_string()));
                row(
                    ui,
                    "direct member bytes",
                    self.info
                        .direct_bytes
                        .as_ref()
                        .map(|bytes| compact_bytes(*bytes)),
                )
                .on_hover_text(format!(
                    "{}\nDirect selected member metadata only; excludes descriptors, records, \
                         provenance, dependent blobs and referenced attachments.",
                    self.info
                        .direct_bytes
                        .as_ref()
                        .map_or_else(|error| error.clone(), |bytes| format!("{bytes} bytes"),),
                ));
                match &self.info.support {
                    Some(Ok(support)) => {
                        row(
                            ui,
                            "support foundations",
                            Ok(support.foundations.to_string()),
                        )
                        .on_hover_text(format!(
                            "Support collection: {}",
                            hex(&support.collection.raw)
                        ));
                    }
                    Some(Err(error)) => {
                        row(ui, "support foundations", Err(error));
                    }
                    None => {
                        row(ui, "support foundations", Ok("not observed".to_owned()));
                    }
                }
                if let Some(parent) = &self.info.parent {
                    row(
                        ui,
                        "parent support",
                        parent.as_ref().map(ParentCoverage::display),
                    )
                    .on_hover_text(
                        "Intersection with the direct parent's admitted foundations in \
                             this same snapshot, including nonresident parent foundations. \
                             Not global synchronization completeness.",
                    );
                }
            })
            .response
    }
}

fn row(ui: &mut egui::Ui, label: &str, value: Result<String, &String>) -> egui::Response {
    let error = value.as_ref().err();
    let text = match &value {
        Ok(value) => format!("{label}  {value}"),
        Err(_) => format!("{label}  unavailable"),
    };
    let mut text = egui::RichText::new(text).monospace().size(10.0);
    if error.is_some() {
        text = text.color(ui.visuals().error_fg_color);
    } else {
        text = text.color(ui.visuals().weak_text_color());
    }
    let response = ui.add(egui::Label::new(text).wrap());
    match error {
        Some(error) => response.on_hover_text(error.as_str()),
        None => response,
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02X}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use triblespace::core::blob::encodings::simplearchive::SimpleArchive;
    use triblespace::core::blob::encodings::succinctarchive::SuccinctArchiveBlob;
    use triblespace::core::collection::{
        empty_metadata_handle, succinctarchive_union, AdmissionPolicy, CollectionCommit,
        CollectionMap, CollectionPolicy, CollectionRecord,
    };
    use triblespace::core::metadata;
    use triblespace::core::repo::BlobMetadata;
    use triblespace::prelude::inlineencodings::Handle;
    use triblespace::prelude::*;

    fn signer() -> SigningKey {
        SigningKey::from_bytes(&[91; 32])
    }

    fn policy() -> CollectionPolicy {
        CollectionPolicy::new(AdmissionPolicy::Open, AdmissionPolicy::Open)
    }

    #[test]
    fn root_counts_payloads_not_attestations_and_snapshot_is_immutable() {
        let mut store = MemoryRepo::for_host(signer().verifying_key());
        let root = store.collection("inspection-root", policy()).unwrap();
        let empty = store.snapshot().unwrap().collection(root).unwrap();
        store
            .commit(root, &signer(), entity! { metadata::name: "first" })
            .unwrap();
        store
            .commit(
                root,
                &SigningKey::from_bytes(&[92; 32]),
                entity! { metadata::name: "first" },
            )
            .unwrap();
        let selected = store.snapshot().unwrap().collection(root).unwrap();
        let info = CollectionInfo::observe(&selected);
        assert_eq!(info.members, 1);
        assert_eq!(info.direct_bytes, Ok(64));
        assert_eq!(
            info.support,
            Some(Ok(SupportInfo {
                collection: root.handle(),
                foundations: 1,
            }))
        );
        assert_eq!(info.parent, None);
        assert!(info.is_resident_observation());
        store
            .commit(root, &signer(), entity! { metadata::name: "second" })
            .unwrap();
        assert_eq!(CollectionInfo::observe(&selected), info);
        assert_eq!(CollectionInfo::observe(&empty).members, 0);
        assert_eq!(CollectionInfo::observe(&empty).direct_bytes, Ok(0));
        assert_eq!(
            CollectionInfo::observe(&store.snapshot().unwrap().collection(root).unwrap()).members,
            2
        );
    }

    #[test]
    fn attached_partial_cover_keeps_parent_denominator_and_never_falls_back() {
        let mut store = MemoryRepo::for_host(signer().verifying_key());
        let root = store.collection("inspection-attached", policy()).unwrap();
        let index = store.attach::<SuccinctArchiveBlob>(root, ()).unwrap();
        let empty = store.snapshot().unwrap().attached(index).unwrap();
        assert_eq!(
            CollectionInfo::observe_attached(&empty)
                .parent
                .unwrap()
                .unwrap()
                .display(),
            "n/a (0/0)"
        );
        let first = store
            .commit(root, &signer(), entity! { metadata::name: "first" })
            .unwrap();
        // An admitted parent's bytes need not be resident to count below.
        let absent: Blob<SimpleArchive> = entity! { metadata::name: "not stored" }
            .into_facts()
            .to_blob();
        store
            .insert(CollectionRecord::Commit(CollectionCommit::sign(
                &signer(),
                root.handle(),
                Handle::<SimpleArchive>::to_hash(absent.get_handle()),
                empty_metadata_handle(),
            )))
            .unwrap();
        let before = store.snapshot().unwrap();
        let info = CollectionInfo::observe_attached(&before.attached(index).unwrap());
        assert_eq!(info.members, 0);
        assert_eq!(info.direct_bytes, Ok(0));
        assert_eq!(info.parent.unwrap().unwrap().display(), "0.0% (0/2)");
        let source: Blob<SimpleArchive> = before
            .get(Handle::<SimpleArchive>::from_hash(first.data()))
            .unwrap();
        let image = succinctarchive_union::derive_element(&source).unwrap();
        let expected_bytes = image.bytes.len() as u64;
        store
            .insert(CollectionRecord::Map(CollectionMap::sign(
                &signer(),
                index.handle(),
                first.data(),
                Handle::<SuccinctArchiveBlob>::to_hash(image.get_handle()),
            )))
            .unwrap();
        let unavailable = store.snapshot().unwrap().attached(index).unwrap();
        assert_eq!(CollectionInfo::observe_attached(&unavailable).members, 0);
        store.put::<SuccinctArchiveBlob, _>(image).unwrap();
        let selected = store.snapshot().unwrap().attached(index).unwrap();
        let partial = CollectionInfo::observe_attached(&selected);
        assert_eq!(partial.members, 1);
        assert_eq!(partial.direct_bytes, Ok(expected_bytes));
        assert_eq!(
            partial.support,
            Some(Ok(SupportInfo {
                collection: root.handle(),
                foundations: 1,
            }))
        );
        assert_eq!(
            partial.parent.as_ref().unwrap().as_ref().unwrap().display(),
            "50.0% (1/2)"
        );
        store
            .commit(root, &signer(), entity! { metadata::name: "later" })
            .unwrap();
        assert_eq!(CollectionInfo::observe_attached(&selected), partial);
        assert_eq!(CollectionInfo::observe_attached(&unavailable).members, 0);
        assert_eq!(
            CollectionInfo::observe_attached(&empty)
                .parent
                .unwrap()
                .unwrap()
                .display(),
            "n/a (0/0)"
        );
    }

    // This seam cannot read bodies, reconstruct a view, or mutate a store.
    struct MetadataOnly(Option<u64>, bool);
    impl BlobStoreMeta for MetadataOnly {
        type MetaError = std::io::Error;
        fn metadata<E: BlobEncoding + 'static>(
            &self,
            _: Inline<Handle<E>>,
        ) -> Result<Option<BlobMetadata>, Self::MetaError>
        where
            Handle<E>: InlineEncoding,
        {
            if self.1 {
                return Err(std::io::Error::other("fixture metadata failure"));
            }
            Ok(self.0.map(|length| BlobMetadata {
                timestamp: 0,
                length,
            }))
        }
    }

    #[test]
    fn raw_cover_unknown_metadata_errors_and_overflow_are_not_zero() {
        let mut store = MemoryRepo::default();
        let root = store.collection("inspection-metadata", policy()).unwrap();
        let a: Blob<SimpleArchive> = entity! { metadata::name: "a" }.into_facts().to_blob();
        let b: Blob<SimpleArchive> = entity! { metadata::name: "b" }.into_facts().to_blob();
        let cover = root.cover([a.get_handle(), b.get_handle()]);
        let unknown = CollectionInfo::observe_cover(&MetadataOnly(None, false), &cover);
        assert_eq!(unknown.members, 2);
        assert!(!unknown.is_resident_observation());
        assert_eq!(unknown.support, None);
        assert_eq!(unknown.parent, None);
        assert!(unknown
            .direct_bytes
            .unwrap_err()
            .contains("metadata unavailable"));
        assert!(
            CollectionInfo::observe_cover(&MetadataOnly(None, true), &cover)
                .direct_bytes
                .unwrap_err()
                .contains("metadata error")
        );
        assert!(
            CollectionInfo::observe_cover(&MetadataOnly(Some(u64::MAX), false), &cover)
                .direct_bytes
                .unwrap_err()
                .contains("exceeds u64")
        );
        assert_eq!(
            CollectionInfo::observe_cover(&MetadataOnly(Some(7), false), &cover).direct_bytes,
            Ok(14)
        );
        assert_eq!(
            CollectionInfo::observe_cover(&MetadataOnly(None, true), &root.cover([])).direct_bytes,
            Ok(0)
        );
    }

    #[test]
    fn percentages_preserve_exact_counts_and_zero_is_not_complete() {
        assert_eq!(
            ParentCoverage {
                covered: 240,
                foundations: 300
            }
            .display(),
            "80.0% (240/300)"
        );
        let empty = ParentCoverage {
            covered: 0,
            foundations: 0,
        };
        assert_eq!(empty.percentage(), None);
        assert_eq!(empty.display(), "n/a (0/0)");
    }
}
