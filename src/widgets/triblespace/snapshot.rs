//! A borrowed face for one successful native pile observation.
//!
//! This is not a loader: an immutable snapshot cannot be partly ready. The
//! widget reads its validated prefix length in constant time. Local observation
//! tokens and host come from the same [`Published`] value, never a later
//! resource attempt. A failed refresh belongs on the pile cell, not here.

use super::inspector_bytes::compact_bytes;
use super::pile::{Observation, Published};
use triblespace::core::repo::pile::PileSnapshot;
use triblespace::prelude::VerifyingKey;

/// Compact, read-only inspection of a native immutable snapshot.
///
/// Retain or share the caller's publication, not this widget. Construction and
/// paint perform no I/O, coverage scan, query, validation, or resource mutation.
/// Observation tokens are local to the originating pile cell, not portable
/// versions, content hashes, or identities comparable across source cells.
pub struct SnapshotView<'a> {
    snapshot: &'a PileSnapshot,
    publication: Option<(Observation, Option<VerifyingKey>)>,
}

impl<'a> SnapshotView<'a> {
    pub fn new(snapshot: &'a PileSnapshot) -> Self {
        Self {
            snapshot,
            publication: None,
        }
    }

    /// Include only the local metadata paired with this exact publication.
    pub fn from_published(published: &'a Published) -> Self {
        Self {
            snapshot: &published.snapshot,
            publication: Some((published.observation, published.host)),
        }
    }

    /// Exact validated byte prefix of this snapshot, not the current file size.
    pub fn prefix_bytes(&self) -> usize {
        self.snapshot.inner().prefix_len()
    }

    fn identity(&self) -> String {
        match self.publication {
            Some((observation, _)) => {
                format!(
                    "snapshot local {}/{}",
                    observation.refresh, observation.append
                )
            }
            None => "snapshot".to_owned(),
        }
    }
}

impl egui::Widget for SnapshotView<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        use egui::{pos2, vec2, Align2, FontId, Sense, Stroke, StrokeKind};
        let (rail, response) =
            ui.allocate_exact_size(vec2(ui.available_width().max(1.0), 28.0), Sense::hover());
        let painter = ui.painter_at(rail.intersect(ui.clip_rect()));
        let weak = ui.visuals().weak_text_color();
        let text = ui.visuals().text_color();
        painter.rect_stroke(rail, 0.0, Stroke::new(1.0_f32, weak), StrokeKind::Inside);
        let inner = rail.shrink(6.0);
        let bytes = self.prefix_bytes();
        let amount =
            painter.layout_no_wrap(compact_bytes(bytes as u64), FontId::monospace(10.0), text);
        let amount_left = (inner.right() - amount.size().x).max(inner.left());
        let labels = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
        labels.galley(
            pos2(amount_left, rail.center().y - amount.size().y / 2.0),
            amount,
            text,
        );
        let identity_right = (amount_left - 10.0).max(inner.left());
        let identity_clip =
            egui::Rect::from_min_max(inner.min, pos2(identity_right, inner.bottom()));
        labels
            .with_clip_rect(identity_clip.intersect(labels.clip_rect()))
            .text(
                pos2(inner.left(), rail.center().y),
                Align2::LEFT_CENTER,
                self.identity(),
                FontId::monospace(10.0),
                weak,
            );
        let mut detail = format!("{}\nValidated prefix: {bytes} bytes", self.identity());
        if let Some((_, host)) = self.publication {
            let host = host.map_or_else(
                || "none".to_owned(),
                |host| {
                    host.to_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect()
                },
            );
            detail.push_str(&format!(
                "\nHost: {host}\nLocal refresh/append tokens; not a portable revision.\nHost identifies believed MERGEs, not READ authority.",
            ));
        }
        response.on_hover_text(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::triblespace::pile::{Observation, Read};
    use std::sync::Arc;
    use triblespace::core::repo::pile::{Pile, PileFile};
    use triblespace::core::repo::{BlobStorePut, SnapshotSource};
    use triblespace::prelude::blobencodings::UTF8String;

    fn publish(pile: &mut Pile, append: u64) -> Published {
        Published {
            snapshot: Arc::new(pile.snapshot().unwrap()),
            host: None,
            observation: Observation { refresh: 2, append },
            open_seconds: 0.0,
            refresh_seconds: 0.0,
        }
    }

    #[test]
    fn native_prefix_and_tokens_stay_with_the_supplied_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        std::fs::File::create(&path).unwrap();
        let mut pile = Pile::new(PileFile::open(&path).unwrap());
        let first = publish(&mut pile, 0);
        assert_eq!(SnapshotView::new(&first.snapshot).prefix_bytes(), 0);
        assert_eq!(SnapshotView::new(&first.snapshot).identity(), "snapshot");
        pile.put::<UTF8String, _>("native snapshot".to_owned())
            .unwrap();
        let second = publish(&mut pile, 1);
        assert!(SnapshotView::new(&second.snapshot).prefix_bytes() > 0);
        assert_eq!(SnapshotView::new(&first.snapshot).prefix_bytes(), 0);
        let failed_attempt = Read {
            snapshot: Some(second.clone()),
            observation: Observation {
                refresh: 3,
                append: 2,
            },
            error: Some(Arc::new(std::io::Error::other("later failure").into())),
            ..Default::default()
        };
        let view = SnapshotView::from_published(failed_attempt.snapshot.as_ref().unwrap());
        assert_eq!(view.identity(), "snapshot local 2/1");
        assert_eq!(view.prefix_bytes(), second.snapshot.prefix_len());
        drop(pile);
        assert_eq!(SnapshotView::new(&first.snapshot).prefix_bytes(), 0);
    }

    #[test]
    fn static_face_is_compact_at_narrow_and_wide_widths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.pile");
        std::fs::File::create(&path).unwrap();
        let mut pile = Pile::new(PileFile::open(&path).unwrap());
        let published = publish(&mut pile, u64::MAX);
        let context = egui::Context::default();
        let _ = context.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                for width in [16.0, 180.0, 320.0, 640.0] {
                    ui.scope(|ui| {
                        ui.set_width(width);
                        let response = ui.add(SnapshotView::from_published(&published));
                        assert_eq!(response.rect.height(), 28.0);
                        assert!((response.rect.width() - width).abs() < 0.1);
                        assert!(!response.changed());
                    });
                }
            });
        });
    }
}
