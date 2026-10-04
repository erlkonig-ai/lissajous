//! Compact inspection of native collection observations, without logical reads.
//!
//! Build [`CollectionInfo`] outside immediate-mode painting, then retain it with
//! the exact immutable snapshot/request that produced it. Metadata lookup can
//! perform I/O (including member validation); even this small projection belongs
//! on the consumer's worker for a large cover. [`CollectionView`] only paints
//! bounded display results. Neither layer materializes a logical collection, maps missing
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

/// One contiguous group in the cover's native canonical member order.
///
/// This is direct member-byte composition, never foundational support. At most
/// 32 groups are retained: the first 31 members individually, then the remainder
/// as one group. Empty members count even though their segment has zero width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteSegment {
    pub members: usize,
    pub bytes: u64,
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
    /// Bounded direct-byte display groups. Empty if any member metadata is
    /// unavailable or the total overflows; no partial distribution is claimed.
    pub byte_segments: Vec<ByteSegment>,
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
        let (direct_bytes, byte_segments) = match direct_bytes(snapshot, cover) {
            Ok((bytes, segments)) => (Ok(bytes), segments),
            Err(error) => (Err(error), Vec::new()),
        };
        Self {
            collection: cover.collection().handle(),
            members: cover.len(),
            direct_bytes,
            byte_segments,
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
) -> Result<(u64, Vec<ByteSegment>), String> {
    let mut sum = 0_u64;
    let mut segments: Vec<ByteSegment> = Vec::new();
    for member in cover.members() {
        let metadata = snapshot
            .metadata(member)
            .map_err(|error| {
                format!(
                    "cover-member metadata error for {}: {error}",
                    hex(&member.raw)
                )
            })?
            .ok_or_else(|| format!("cover-member metadata unavailable for {}", hex(&member.raw)))?;
        sum = sum
            .checked_add(metadata.length)
            .ok_or_else(|| "direct cover-member byte total exceeds u64".to_owned())?;
        if segments.len() < 32 {
            segments.push(ByteSegment {
                members: 1,
                bytes: metadata.length,
            });
        } else {
            let last = segments.last_mut().unwrap();
            last.members += 1;
            last.bytes += metadata.length; // Bounded by the already checked total.
        }
    }
    Ok((sum, segments))
}

/// A square-edged instrument: direct member-byte composition above, direct-parent
/// support below when attached. Both are static relationships, not loading
/// progress: there is no animation, spinner or completion state.
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
        let (bounds, allocation) = ui.allocate_exact_size(
            egui::vec2(
                ui.available_width().max(1.0),
                if self.info.parent.is_some() {
                    60.0
                } else {
                    28.0
                },
            ),
            egui::Sense::hover(),
        );
        let rail = egui::Rect::from_min_size(bounds.min, egui::vec2(bounds.width(), 28.0));
        let mut response = ui.interact(rail, allocation.id.with("bytes"), egui::Sense::hover());
        let painter = ui.painter_at(rail.intersect(ui.clip_rect()));
        let weak = ui.visuals().weak_text_color();
        let text = ui.visuals().text_color();
        let error = ui.visuals().error_fg_color;
        let failed = self.info.direct_bytes.is_err()
            || self.info.support.as_ref().is_some_and(Result::is_err);
        let inner = rail.shrink(1.0);
        let mut detail = format!("Collection: {identity}\nDirect selected member bytes only; not parent support.\nExcludes descriptors, records, provenance, dependencies and attachments.");
        if let Some((total, groups)) = byte_distribution(self.info) {
            let mut used = 0_u64;
            let mut first_member = 1_usize;
            for (index, group) in groups.iter().enumerate() {
                let start = inner.left() + inner.width() * (used as f64 / total as f64) as f32;
                used += group.bytes;
                let end = inner.left() + inner.width() * (used as f64 / total as f64) as f32;
                let segment = egui::Rect::from_min_max(
                    egui::pos2(start, inner.top()),
                    egui::pos2(end, inner.bottom()),
                );
                painter.rect_filled(
                    segment,
                    0.0,
                    weak.gamma_multiply(if index % 2 == 0 { 0.10 } else { 0.20 }),
                );
                if ui.rect_contains_pointer(segment) {
                    detail.push_str(&format!(
                        "\nCanonical member group {first_member}–{}: {} members, {} bytes ({:.1}% of direct bytes).\nGroups are contiguous in native cover order; the last may aggregate members.",
                        first_member + group.members - 1,
                        group.members,
                        group.bytes,
                        group.bytes as f64 * 100.0 / total as f64,
                    ));
                }
                first_member += group.members;
            }
        }
        let amount = match &self.info.direct_bytes {
            Err(message) => {
                detail.push_str(&format!("\nDirect bytes unavailable: {message}"));
                "direct ?".to_owned()
            }
            Ok(bytes) => {
                detail.push_str(&format!("\nExact direct bytes: {bytes}"));
                format!("direct {}", compact_bytes(*bytes))
            }
        };
        let members = if self.info.resident {
            format!("{} members", self.info.members)
        } else {
            format!("{} cover members", self.info.members)
        };
        let counts = match &self.info.support {
            Some(Ok(support)) => {
                detail.push_str(&format!(
                    "\nSupport collection: {}\nSupport foundations: {}",
                    hex(&support.collection.raw),
                    support.foundations,
                ));
                format!("{members} · {} foundations", support.foundations)
            }
            Some(Err(message)) => {
                detail.push_str(&format!("\nSupport unavailable: {message}"));
                format!("{members} · support ?")
            }
            None => {
                detail.push_str("\nSupport not observed for this raw cover.");
                members
            }
        };
        detail.push_str(&format!(
            "\n{} selected {} members. Byte proportions do not measure support coverage.",
            self.info.members,
            if self.info.resident {
                "resident"
            } else {
                "cover"
            },
        ));
        stroke_rail(&painter, rail, if failed { error } else { weak });
        let content = painter.with_clip_rect(rail.shrink(2.0).intersect(painter.clip_rect()));
        let right = rail.right() - 6.0;
        let amount_color = if self.info.direct_bytes.is_err() {
            error
        } else {
            text
        };
        let amount = content.layout_no_wrap(amount, egui::FontId::monospace(10.0), amount_color);
        let amount_left = (right - amount.size().x).max(rail.left() + 6.0);
        content.galley(
            egui::pos2(amount_left, rail.top() + 9.0 - amount.size().y * 0.5),
            amount,
            amount_color,
        );
        label(
            &content,
            self.name.unwrap_or(&identity[..16]),
            egui::pos2(rail.left() + 6.0, rail.top() + 9.0),
            (amount_left - rail.left() - 14.0).max(0.0),
            10.0,
            weak,
        );
        label(
            &content,
            &counts,
            egui::pos2(rail.left() + 6.0, rail.top() + 21.0),
            (rail.width() - 12.0).max(0.0),
            8.0,
            weak,
        );
        response = response.on_hover_text(detail);

        if let Some(parent) = &self.info.parent {
            let rail = egui::Rect::from_min_size(
                egui::pos2(bounds.left(), bounds.top() + 32.0),
                egui::vec2(bounds.width(), 28.0),
            );
            let support_response =
                ui.interact(rail, allocation.id.with("support"), egui::Sense::hover());
            let painter = ui.painter_at(rail.intersect(ui.clip_rect()));
            let (amount, detail, failed) = match parent {
                Ok(parent) if parent.covered <= parent.foundations => {
                    if let Some(fraction) = parent_fraction(parent) {
                        let inner = rail.shrink(1.0);
                        let boundary = inner.left() + inner.width() * fraction;
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                inner.min,
                                egui::pos2(boundary, inner.bottom()),
                            ),
                            0.0,
                            weak.gamma_multiply(0.20),
                        );
                        painter.rect_filled(
                            egui::Rect::from_min_max(egui::pos2(boundary, inner.top()), inner.max),
                            0.0,
                            weak.gamma_multiply(0.04),
                        );
                    }
                    (
                        parent.display(),
                        format!(
                            "{} represented; {} not yet represented of {} direct-parent known foundations.\nSame immutable snapshot, including nonresident parent foundations.\nThis is support coverage, not bytes, loading progress, or global completeness.",
                            parent.covered, parent.foundations - parent.covered, parent.foundations,
                        ),
                        false,
                    )
                }
                Ok(_) => (
                    "invalid".to_owned(),
                    "Represented support exceeds parent foundations.".to_owned(),
                    true,
                ),
                Err(message) => ("unavailable".to_owned(), message.clone(), true),
            };
            stroke_rail(&painter, rail, if failed { error } else { weak });
            let content = painter.with_clip_rect(rail.shrink(2.0).intersect(painter.clip_rect()));
            let amount_color = if failed { error } else { text };
            let amount =
                content.layout_no_wrap(amount, egui::FontId::monospace(10.0), amount_color);
            let amount_left = (rail.right() - 6.0 - amount.size().x).max(rail.left() + 6.0);
            content.galley(
                egui::pos2(amount_left, rail.center().y - amount.size().y * 0.5),
                amount,
                amount_color,
            );
            label(
                &content,
                "parent support",
                egui::pos2(rail.left() + 6.0, rail.center().y),
                (amount_left - rail.left() - 14.0).max(0.0),
                10.0,
                weak,
            );
            response = response.union(support_response.on_hover_text(detail));
        }
        response
    }
}

fn stroke_rail(painter: &egui::Painter, rail: egui::Rect, color: egui::Color32) {
    painter.rect_stroke(
        rail,
        0.0,
        egui::Stroke::new(1.0_f32, color),
        egui::StrokeKind::Inside,
    );
}

fn label(
    painter: &egui::Painter,
    text: &str,
    position: egui::Pos2,
    width: f32,
    size: f32,
    color: egui::Color32,
) {
    if width <= 0.0 {
        return;
    }
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), egui::FontId::monospace(size), color, width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(position.x, position.y - galley.size().y * 0.5),
        galley,
        color,
    );
}

// Validate the bounded public display projection without re-inspecting data.
// Unknown, zero-byte, or inconsistent inputs never produce guessed proportions.
fn byte_distribution(info: &CollectionInfo) -> Option<(u64, &[ByteSegment])> {
    let total = *info.direct_bytes.as_ref().ok()?;
    if total == 0 || info.byte_segments.len() > 32 {
        return None;
    }
    let (bytes, members) =
        info.byte_segments
            .iter()
            .try_fold((0_u64, 0_usize), |(bytes, members), group| {
                if group.members == 0 {
                    return None;
                }
                Some((
                    bytes.checked_add(group.bytes)?,
                    members.checked_add(group.members)?,
                ))
            })?;
    (bytes == total && members == info.members).then_some((total, &info.byte_segments))
}

fn parent_fraction(parent: &ParentCoverage) -> Option<f32> {
    (parent.foundations > 0 && parent.covered <= parent.foundations)
        .then(|| (parent.covered as f64 / parent.foundations as f64) as f32)
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
        assert!(unknown.byte_segments.is_empty());
        assert!(byte_distribution(&unknown).is_none());
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
        assert_eq!(parent_fraction(&empty), None);
        assert_eq!(
            parent_fraction(&ParentCoverage {
                covered: 4,
                foundations: 5
            }),
            Some(0.8)
        );
        assert_eq!(
            parent_fraction(&ParentCoverage {
                covered: 6,
                foundations: 5
            }),
            None
        );
    }

    #[test]
    fn byte_groups_are_bounded_contiguous_and_keep_all_selected_members() {
        let mut store = MemoryRepo::default();
        let root = store
            .collection("inspection-byte-groups", policy())
            .unwrap();
        let members: Vec<_> = (0..70)
            .map(|index| {
                let blob: Blob<SimpleArchive> =
                    entity! { metadata::name: format!("member-{index}") }
                        .into_facts()
                        .to_blob();
                blob.get_handle()
            })
            .collect();
        let cover = root.cover(members);
        let info = CollectionInfo::observe_cover(&MetadataOnly(Some(7), false), &cover);
        assert_eq!(info.members, 70);
        assert_eq!(info.direct_bytes, Ok(490));
        assert_eq!(info.byte_segments.len(), 32);
        assert!(info.byte_segments[..31].iter().all(|segment| *segment
            == ByteSegment {
                members: 1,
                bytes: 7
            }));
        assert_eq!(
            info.byte_segments[31],
            ByteSegment {
                members: 39,
                bytes: 273
            }
        );
        assert_eq!(byte_distribution(&info).unwrap().0, 490);
        assert_eq!(info.support, None); // Bytes never manufacture support.
        let zero = CollectionInfo::observe_cover(&MetadataOnly(Some(0), false), &cover);
        assert_eq!(zero.direct_bytes, Ok(0));
        assert_eq!(
            zero.byte_segments
                .iter()
                .map(|group| group.members)
                .sum::<usize>(),
            70
        );
        assert!(byte_distribution(&zero).is_none());
        let missing = CollectionInfo::observe_cover(&MetadataOnly(None, false), &cover);
        assert!(missing.byte_segments.is_empty());
        assert!(byte_distribution(&missing).is_none());
    }

    #[test]
    fn rails_fill_grid_spans_and_unknowns_keep_compact_geometry() {
        let mut native = MemoryRepo::default();
        let root = native.collection("inspection-layout", policy()).unwrap();
        native
            .commit(root, &signer(), entity! { metadata::name: "one" })
            .unwrap();
        let observed = native.snapshot().unwrap().collection(root).unwrap();
        let basic = CollectionInfo::observe(&observed);
        let mut attached = basic.clone();
        attached.parent = Some(Ok(ParentCoverage {
            covered: 1,
            foundations: 2,
        }));
        let mut empty_parent = basic.clone();
        empty_parent.parent = Some(Ok(ParentCoverage {
            covered: 0,
            foundations: 0,
        }));
        let mut unavailable = basic.clone();
        unavailable.direct_bytes = Err("synthetic metadata error".to_owned());
        unavailable.byte_segments.clear();
        unavailable.parent = Some(Err("synthetic parent error".to_owned()));
        let context = egui::Context::default();
        let state = crate::state::StateStore::default();
        for info in [&basic, &attached, &empty_parent, &unavailable] {
            let _ = context.run_ui(Default::default(), |ui| {
                for span in [3, 6, 12] {
                    ui.push_id(span, |ui| {
                        let left = ui.next_widget_position().x;
                        crate::CardCtx::new(ui, &state).grid(|g| {
                            g.place(span, |ctx| {
                                let response = ctx.ui_mut().add(
                                    CollectionView::new(info)
                                        .name("A deliberately long native collection label"),
                                );
                                assert_eq!(
                                    response.rect.left(),
                                    left + crate::card_ctx::GRID_EDGE_PAD
                                );
                                assert_eq!(
                                    response.rect.width(),
                                    crate::card_ctx::span_width(span)
                                );
                                assert_eq!(
                                    response.rect.height(),
                                    if info.parent.is_some() { 60.0 } else { 28.0 }
                                );
                                assert!(!response.changed());
                            })
                        });
                    });
                }
            });
        }
    }
}
