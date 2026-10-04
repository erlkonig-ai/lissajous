//! One retained, read-only pile, with native immutable snapshots as its output.
//!
//! Retain [`PileCell`] with [`crate::NotebookCtx::state`]. Construction
//! starts an I/O owner; painting only reads its latest publication. Consumers
//! clone [`Published::snapshot`] and own their own queries, tasks and answers.
//! This resource never selects a collection, runs a query, fetches a missing
//! body or builds an index. Detaching its card changes placement only.
//! Enable `triblespace-pile` for this resource without the inspector's WASM
//! formatter dependency; the existing `triblespace` feature includes both.
//!
//! This source requires Core's `refresh_next` API (revision `3dd8930e`),
//! not merely a registry version bearing the same number. Piles must remain
//! append-only. Atomic path replacement is supported; editing or truncating an
//! already mapped file is not. A detected violation is refused, but no wrapper
//! can make previously handed-out mappings safe after mutation.

use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};
use triblespace::core::repo::pile::{Pile, PileFile, PileSnapshot, ReadError};
use triblespace::core::repo::SnapshotSource;
use triblespace::prelude::VerifyingKey;

#[path = "pile_progress.rs"]
mod progress;
use progress::{replay_batch, Batch};
pub use progress::{Phase, PileProgress, Progress};

const OBSERVE_INTERVAL: Duration = Duration::from_secs(2);

/// Explicit source identity. The host chooses whose native MERGEs are believed;
/// it is not a signing key, collection selector or grant of READ authority.
#[derive(Clone)]
pub struct PileOpen {
    pub path: PathBuf,
    pub host: Option<VerifyingKey>,
}

/// Local resource tokens, not portable versions or collection identities.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Observation {
    /// Advances when a changed file observation is noticed on the same lineage.
    pub append: u64,
    /// Advances on explicit refresh and when the path changes reader lineage.
    pub refresh: u64,
}

/// A successful native observation. All fields describe this exact snapshot.
#[derive(Clone)]
pub struct Published {
    pub snapshot: Arc<PileSnapshot>,
    pub host: Option<VerifyingKey>,
    pub observation: Observation,
    pub open_seconds: f64,
    pub refresh_seconds: f64,
}

/// During refresh or a same-file failure, `snapshot` may be the last success;
/// its own token is never relabelled as the new observation.
#[derive(Clone, Default)]
pub struct Read {
    pub snapshot: Option<Published>,
    pub observation: Observation,
    pub progress: Progress,
    pub error: Option<Arc<ReadError>>,
}

#[derive(Default)]
struct Shared {
    value: Mutex<Read>,
    changed: Condvar,
    repaint: Mutex<Option<egui::Context>>,
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut Read)) {
        change(&mut self.value.lock().unwrap_or_else(|e| e.into_inner()));
        self.changed.notify_all();
        if let Ok(context) = self.repaint.try_lock() {
            if let Some(context) = &*context {
                context.request_repaint();
            }
        }
    }

    fn fail(&self, error: ReadError) {
        self.update(|value| {
            value.progress.phase = Phase::Failed;
            value.error = Some(Arc::new(error));
        });
    }

    fn fail_attempt(&self, error: ReadError, refresh: u64) {
        self.update(|value| {
            // A refresh requested during an older failing read still deserves
            // its own attempt; it must not wake wait() with that older error.
            if value.observation.refresh == refresh {
                value.progress.phase = Phase::Failed;
                value.error = Some(Arc::new(error));
            }
        });
    }

    fn invalidate(&self) -> Observation {
        let mut observation = Observation::default();
        let mut retired = None;
        self.update(|value| {
            value.observation.refresh += 1;
            retired = value.snapshot.take();
            value.error = None;
            value.progress = Progress::default();
            observation = value.observation;
        });
        drop(retired); // Native backing is never released under the UI mailbox lock.
        observation
    }

    fn clear_snapshot(&self) {
        let mut retired = None;
        self.update(|value| retired = value.snapshot.take());
        drop(retired);
    }
}

/// One I/O owner per source, never an executor for its consumers.
///
/// Drop signals shutdown without joining: a native record or file-lock wait may
/// be unbounded. The owner exits between records or after the final snapshot,
/// and drops its read-only handle. Already published snapshots remain owned.
pub struct PileCell {
    path: PathBuf,
    refresh: Option<mpsc::SyncSender<()>>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
}

impl PileCell {
    pub fn new(options: PileOpen) -> Self {
        Self::with_interval(options, OBSERVE_INTERVAL)
    }

    fn with_interval(options: PileOpen, interval: Duration) -> Self {
        let path = options.path.clone();
        let (refresh, requests) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared::default());
        let (ending, output) = (Arc::clone(&stop), Arc::clone(&shared));
        let thread = std::thread::Builder::new()
            .name("pile-resource".into())
            .spawn(move || {
                // A panicking owner must not strand headless waiters or a busy card.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = Session::new(options);
                    let mut force = true;
                    loop {
                        if ending.load(Ordering::Relaxed) {
                            break;
                        }
                        match session.update(&output, &ending, force) {
                            Ok(true) => continue, // Path changed during work; observe again.
                            Ok(false) => {}
                            Err(error) => output.fail_attempt(error, session.attempt_refresh),
                        }
                        if ending.load(Ordering::Relaxed) {
                            break;
                        }
                        force = match requests.recv_timeout(interval) {
                            Ok(()) => true,
                            Err(mpsc::RecvTimeoutError::Timeout) => false,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        };
                    }
                }));
                if result.is_err() {
                    output.fail(
                        std::io::Error::other("pile resource worker panicked; recreate the cell")
                            .into(),
                    );
                }
            });
        if let Err(error) = thread {
            shared.fail(error.into());
        }
        Self {
            path,
            refresh: Some(refresh),
            stop,
            shared,
        }
    }

    /// Paint resource progress only: no filesystem call, native read, query or
    /// worker join. A heartbeat backs up completion wakes.
    pub fn show(&self, ui: &mut egui::Ui) {
        if let Ok(mut context) = self.shared.repaint.try_lock() {
            *context = Some(ui.ctx().clone());
        }
        let delay = if let Some(read) = self.read() {
            let error = read.error.as_ref().map(ToString::to_string);
            ui.add(PileProgress::new(&self.path, read.progress).error(error.as_deref()));
            if read.progress.active() {
                Duration::from_millis(100)
            } else {
                OBSERVE_INTERVAL
            }
        } else {
            Duration::from_millis(100)
        };
        ui.ctx().request_repaint_after(delay);
    }

    /// Clone the latest publication without waiting for its short metadata lock.
    pub fn read(&self) -> Option<Read> {
        match self.shared.value.try_lock() {
            Ok(value) => Some(value.clone()),
            Err(std::sync::TryLockError::Poisoned(error)) => Some(error.into_inner().clone()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// Request a new native observation even when metadata is unchanged.
    /// Requests coalesce; no consumer work can be submitted to this owner.
    pub fn refresh(&self) {
        self.shared.update(|value| {
            value.observation.refresh += 1;
            value.error = None;
            value.progress.phase = Phase::Replay;
        });
        if self.refresh.as_ref().is_none_or(|sender| {
            matches!(
                sender.try_send(()),
                Err(mpsc::TrySendError::Disconnected(_))
            )
        }) {
            self.shared.fail(
                std::io::Error::other("pile resource worker stopped; recreate the cell").into(),
            );
        }
    }

    /// Wait for a settled attempt. Headless/capture only; never call in paint.
    /// After `refresh`, an older success cannot satisfy this wait. Native I/O
    /// has no deadline; errors are returned in [`Read`].
    pub fn wait(&self) -> Read {
        let mut value = self.shared.value.lock().unwrap_or_else(|e| e.into_inner());
        while value.progress.active() {
            value = self
                .shared
                .changed
                .wait(value)
                .unwrap_or_else(|e| e.into_inner());
        }
        value.clone()
    }
}

impl Drop for PileCell {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.refresh.take();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileId {
    device: u64,
    inode: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    file: FileId,
    modified: SystemTime,
    length: u64,
}

impl Stamp {
    fn from_metadata(metadata: Metadata) -> Result<Self, ReadError> {
        if !metadata.is_file() {
            return Err(std::io::Error::other("pile is not a regular file").into());
        }
        Ok(Self {
            file: FileId {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            modified: metadata.modified()?,
            length: metadata.len(),
        })
    }
    fn at(path: &Path) -> Result<Self, ReadError> {
        Self::from_metadata(std::fs::metadata(path)?)
    }
}

struct Opened {
    pile: Pile,
    stamp: Stamp,
    open_seconds: f64,
    // Only another inode may replace a mapping known to have been mutated.
    invalid: bool,
}

struct Session {
    options: PileOpen,
    opened: Option<Opened>,
    attempted: Option<Stamp>,
    missing: bool,
    attempt_refresh: u64,
}

impl Session {
    fn new(options: PileOpen) -> Self {
        Self {
            options,
            opened: None,
            attempted: None,
            missing: false,
            attempt_refresh: 0,
        }
    }

    /// True means an observed path race merits an immediate retry.
    fn update(
        &mut self,
        shared: &Shared,
        stop: &AtomicBool,
        force: bool,
    ) -> Result<bool, ReadError> {
        self.attempt_refresh = shared
            .value
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observation
            .refresh;
        let mut current = match Stamp::at(&self.options.path) {
            Ok(stamp) => stamp,
            Err(error) => {
                if !self.missing {
                    self.attempt_refresh = shared.invalidate().refresh;
                }
                self.missing = true;
                self.attempted = None;
                return Err(error);
            }
        };
        self.missing = false;
        if self.attempted == Some(current) && !force {
            return Ok(false);
        }
        let previous = self.attempted.replace(current);
        let replaced = self
            .opened
            .as_ref()
            .is_none_or(|old| old.stamp.file != current.file);
        if replaced {
            self.attempt_refresh = shared.invalidate().refresh;
            self.opened = None;
            let started = Instant::now();
            let file = PileFile::open_read_only(&self.options.path)?;
            let actual = Stamp::from_metadata(file.backing_file_metadata()?)?;
            if actual.file != current.file {
                self.attempted = None;
                return Ok(true);
            }
            self.opened = Some(Opened {
                pile: Pile::with_host(file, self.options.host),
                stamp: actual,
                open_seconds: started.elapsed().as_secs_f64(),
                invalid: false,
            });
            current = actual;
        } else if previous != Some(current) {
            shared.update(|value| value.observation.append += 1);
        }
        let opened = self.opened.as_mut().expect("opened resource");
        // Do not bless an in-place shrink/edit as a fresh safe reader. Metadata
        // cannot prove safety against arbitrary concurrent mutations: the native
        // append-only contract still applies throughout the operation.
        if current.length < opened.stamp.length
            || (current.length == opened.stamp.length && current.modified != opened.stamp.modified)
        {
            opened.invalid = true;
        }
        if opened.invalid {
            shared.clear_snapshot();
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
                "pile changed in place; stop its users and replace it atomically, do not retry this mapping").into());
        }
        opened.stamp = current;
        let observation = {
            let mut value = shared.value.lock().unwrap_or_else(|e| e.into_inner());
            value.error = None;
            value.observation
        };
        self.attempt_refresh = observation.refresh;
        let Some((snapshot, seconds)) =
            prepare_snapshot(&mut opened.pile, shared, || stop.load(Ordering::Relaxed))?
        else {
            return Ok(false);
        };
        let after = Stamp::at(&self.options.path)?;
        if after.file != current.file {
            self.attempted = None;
            shared.invalidate();
            return Ok(true);
        }
        if after.length < snapshot.prefix_len() as u64
            || after.length < current.length
            || (after.length == current.length && after.modified != current.modified)
        {
            opened.invalid = true;
            shared.clear_snapshot();
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "pile changed in place during snapshot publication",
            )
            .into());
        }
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        // Keep the replayed prefix separate from the latest stat: bytes arriving
        // after snapshot() must cause another attempt, not disappear in a stamp.
        let covered = snapshot.prefix_len() as u64;
        self.attempted = Some(if after.length == covered {
            after
        } else {
            current
        });
        opened.stamp = after;
        let mut retired = None;
        shared.update(|value| {
            retired = value.snapshot.replace(Published {
                snapshot: Arc::new(snapshot),
                host: self.options.host,
                observation,
                open_seconds: opened.open_seconds,
                refresh_seconds: seconds,
            });
            value.error = None;
            if value.observation == observation {
                value.progress.phase = Phase::Ready;
            }
        });
        drop(retired);
        // A growing writer does not keep this pass alive forever. The next
        // observation notices the outstanding suffix via attempted above.
        Ok(false)
    }
}

/// Replay yields between complete records. The final native snapshot still
/// bulk-refreshes and may consume newer appends; it is not a bounded operation.
fn prepare_snapshot(
    pile: &mut Pile,
    shared: &Shared,
    cancelled: impl Fn() -> bool,
) -> Result<Option<(PileSnapshot, f64)>, ReadError> {
    let started = Instant::now();
    let target = pile.backing_file_metadata()?.len();
    let mut offset = pile.refreshed_len() as u64;
    shared.update(|value| {
        value.progress.phase = Phase::Replay;
        value.progress.bytes(offset, target);
    });
    let mut published = Instant::now();
    loop {
        let batch = replay_batch(&mut offset, target, &cancelled, || pile.refresh_next())?;
        if batch != Batch::Yield || published.elapsed() >= Duration::from_millis(50) {
            let observed = pile.backing_file_metadata()?.len();
            shared.update(|value| value.progress.bytes(offset, observed));
            published = Instant::now();
        }
        match batch {
            Batch::Cancelled => return Ok(None),
            Batch::CaughtUp => break,
            Batch::Yield => std::thread::yield_now(),
        }
    }
    if cancelled() {
        return Ok(None);
    }
    shared.update(|value| value.progress.phase = Phase::Snapshot);
    let snapshot = pile.snapshot()?;
    let observed = pile.backing_file_metadata()?.len();
    shared.update(|value| value.progress.bytes(snapshot.prefix_len() as u64, observed));
    if cancelled() {
        return Ok(None);
    }
    Ok(Some((snapshot, started.elapsed().as_secs_f64())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use triblespace::core::repo::{BlobStoreGet, BlobStorePut};
    use triblespace::prelude::{blobencodings::UTF8String, inlineencodings::Handle, Blob, Inline};

    fn options(path: &Path) -> PileOpen {
        PileOpen {
            path: path.to_owned(),
            host: None,
        }
    }

    fn append(path: &Path, text: &str) -> Inline<Handle<UTF8String>> {
        let mut writer = Pile::open(path).unwrap();
        let handle = writer.put::<UTF8String, _>(text.to_owned()).unwrap();
        writer.close().unwrap();
        handle
    }

    fn step(session: &mut Session, shared: &Shared, force: bool) {
        if let Err(error) = session.update(shared, &AtomicBool::new(false), force) {
            shared.fail_attempt(error, session.attempt_refresh);
        }
    }

    fn published(shared: &Shared) -> Published {
        shared.value.lock().unwrap().snapshot.clone().unwrap()
    }

    fn await_read(cell: &PileCell, ready: impl Fn(&Read) -> bool) -> Read {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut value = cell.shared.value.lock().unwrap();
        while !ready(&value) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "resource did not settle: {:?}",
                value.progress
            );
            value = cell
                .shared
                .changed
                .wait_timeout(value, remaining)
                .unwrap()
                .0;
        }
        value.clone()
    }

    #[test]
    fn shared_native_snapshot_survives_append_and_owner_drop() {
        fn send_sync<T: Send + Sync + 'static>() {}
        send_sync::<PileSnapshot>();
        send_sync::<PileCell>();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        let first = append(&path, "first");
        let shared = Shared::default();
        let mut session = Session::new(options(&path));
        step(&mut session, &shared, true);
        let before = published(&shared);
        let second = append(&path, "second");
        step(&mut session, &shared, false);
        let after = published(&shared);
        assert_eq!(before.observation.refresh, after.observation.refresh);
        assert!(after.observation.append > before.observation.append);
        drop(session);
        assert!(before.snapshot.get::<Blob<UTF8String>, _>(first).is_ok());
        assert!(before.snapshot.get::<Blob<UTF8String>, _>(second).is_err());
        assert!(after.snapshot.get::<Blob<UTF8String>, _>(second).is_ok());
    }

    #[test]
    fn unchanged_poll_does_not_publish_or_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        append(&path, "one");
        let shared = Shared::default();
        let mut session = Session::new(options(&path));
        step(&mut session, &shared, true);
        let before = published(&shared);
        let stamp = shared.value.lock().unwrap().progress.observed_at;
        for _ in 0..20 {
            step(&mut session, &shared, false);
        }
        let after = published(&shared);
        assert!(Arc::ptr_eq(&before.snapshot, &after.snapshot));
        assert_eq!(before.observation, after.observation);
        assert_eq!(stamp, shared.value.lock().unwrap().progress.observed_at);
    }

    #[test]
    fn atomic_replacement_gets_an_epoch_and_keeps_old_snapshot_alive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        let replacement = dir.path().join("replacement.pile");
        File::create(&path).unwrap();
        File::create(&replacement).unwrap();
        let first = append(&path, "first");
        let second = append(&replacement, "other");
        let shared = Shared::default();
        let mut session = Session::new(options(&path));
        step(&mut session, &shared, true);
        let before = published(&shared);
        std::fs::rename(&replacement, &path).unwrap();
        step(&mut session, &shared, false);
        let after = published(&shared);
        assert!(after.observation.refresh > before.observation.refresh);
        assert!(before.snapshot.get::<Blob<UTF8String>, _>(first).is_ok());
        assert!(after.snapshot.get::<Blob<UTF8String>, _>(second).is_ok());
        assert!(after.snapshot.get::<Blob<UTF8String>, _>(first).is_err());
    }

    #[test]
    fn replacement_during_locked_replay_never_publishes_old_file_as_new() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        let replacement = dir.path().join("replacement.pile");
        File::create(&path).unwrap();
        File::create(&replacement).unwrap();
        let old = append(&path, "old source");
        let new = append(&replacement, "new source");
        let lock = File::open(&path).unwrap();
        lock.lock().unwrap();
        let cell = PileCell::new(options(&path));
        await_read(&cell, |read| read.progress.phase == Phase::Replay);
        std::fs::rename(&replacement, &path).unwrap();
        lock.unlock().unwrap();
        let read = await_read(&cell, |read| read.progress.phase == Phase::Ready);
        let published = read.snapshot.unwrap();
        assert!(published.snapshot.get::<Blob<UTF8String>, _>(new).is_ok());
        assert!(published.snapshot.get::<Blob<UTF8String>, _>(old).is_err());
        assert_eq!(published.observation, read.observation);
    }

    #[test]
    fn partial_record_error_is_native_and_retries_only_when_requested_or_changed() {
        let dir = tempfile::tempdir().unwrap();
        let complete = dir.path().join("complete.pile");
        File::create(&complete).unwrap();
        let handle = append(&complete, "a complete frame");
        let frame = std::fs::read(&complete).unwrap();
        let path = dir.path().join("partial.pile");
        let mut writer = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .unwrap();
        writer.write_all(&frame[..16]).unwrap();
        let shared = Shared::default();
        let mut session = Session::new(options(&path));
        step(&mut session, &shared, true);
        let error = shared.value.lock().unwrap().error.clone().unwrap();
        assert!(matches!(
            &*error,
            ReadError::CorruptPile { valid_length: 0 }
        ));
        assert!(shared.value.lock().unwrap().snapshot.is_none());
        let attempt = shared.value.lock().unwrap().progress.observed_at;
        step(&mut session, &shared, false);
        assert_eq!(attempt, shared.value.lock().unwrap().progress.observed_at);
        assert_eq!(std::fs::read(&path).unwrap(), frame[..16]);
        writer.write_all(&frame[16..]).unwrap();
        step(&mut session, &shared, false);
        assert!(shared.value.lock().unwrap().error.is_none());
        assert!(published(&shared)
            .snapshot
            .get::<Blob<UTF8String>, _>(handle)
            .is_ok());
    }

    #[test]
    fn failed_append_keeps_only_the_labelled_last_good_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        let handle = append(&path, "one");
        let frame = std::fs::read(&path).unwrap();
        let shared = Shared::default();
        let mut session = Session::new(options(&path));
        step(&mut session, &shared, true);
        let before = published(&shared);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&frame[..16])
            .unwrap();
        step(&mut session, &shared, false);
        let read = shared.value.lock().unwrap().clone();
        assert_eq!(read.progress.phase, Phase::Failed);
        assert!(read.error.is_some());
        assert_ne!(read.observation, before.observation);
        let retained = read.snapshot.unwrap();
        assert_eq!(retained.observation, before.observation);
        assert!(Arc::ptr_eq(&retained.snapshot, &before.snapshot));
        assert!(retained.snapshot.get::<Blob<UTF8String>, _>(handle).is_ok());
    }

    #[test]
    fn incomplete_tail_stays_failed_without_idle_retry_spin() {
        let dir = tempfile::tempdir().unwrap();
        let complete = dir.path().join("complete.pile");
        File::create(&complete).unwrap();
        append(&complete, "one complete frame");
        let frame = std::fs::read(&complete).unwrap();
        let path = dir.path().join("partial.pile");
        File::create(&path)
            .unwrap()
            .write_all(&frame[..16])
            .unwrap();
        let cell = PileCell::with_interval(options(&path), Duration::from_millis(10));
        let before = await_read(&cell, |read| read.progress.phase == Phase::Failed);
        std::thread::sleep(Duration::from_millis(100));
        let after = cell.read().unwrap();
        assert_eq!(before.observation, after.observation);
        assert_eq!(before.progress.observed_at, after.progress.observed_at);
        assert!(Arc::ptr_eq(
            before.error.as_ref().unwrap(),
            after.error.as_ref().unwrap()
        ));
        assert!(after.snapshot.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), frame[..16]);
    }

    #[test]
    fn published_host_matches_the_native_snapshot_fold() {
        use triblespace::core::collection::CoverageRead;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        let host = ed25519_dalek::SigningKey::from_bytes(&[7; 32]).verifying_key();
        let shared = Shared::default();
        let mut session = Session::new(PileOpen {
            path,
            host: Some(host),
        });
        step(&mut session, &shared, true);
        let output = published(&shared);
        assert_eq!(output.host, Some(host));
        let native = output
            .snapshot
            .index(&std::collections::BTreeSet::new())
            .unwrap();
        assert_eq!(native.host().unwrap().raw, host.to_bytes());
    }

    #[test]
    fn missing_file_never_becomes_empty_success() {
        let dir = tempfile::tempdir().unwrap();
        let cell = PileCell::new(options(&dir.path().join("absent.pile")));
        let read = await_read(&cell, |read| read.progress.phase == Phase::Failed);
        assert!(read.snapshot.is_none());
        assert!(matches!(read.error.as_deref(), Some(ReadError::IoError(_))));
    }

    #[test]
    fn idle_owner_notices_growth_without_a_consumer_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        let cell = PileCell::with_interval(options(&path), Duration::from_millis(20));
        await_read(&cell, |read| read.progress.phase == Phase::Ready);
        let before = cell.wait().snapshot.unwrap();
        let handle = append(&path, "arrived while idle");
        let after = await_read(&cell, |read| {
            read.snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.observation.append > before.observation.append)
        });
        assert!(after
            .snapshot
            .unwrap()
            .snapshot
            .get::<Blob<UTF8String>, _>(handle)
            .is_ok());
    }

    #[test]
    fn refresh_wait_cannot_return_previous_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        let cell = Arc::new(PileCell::new(options(&path)));
        await_read(&cell, |read| read.progress.phase == Phase::Ready);
        let before = cell.wait().observation;
        let lock = File::open(&path).unwrap();
        lock.lock().unwrap();
        cell.refresh();
        let (send, result) = mpsc::channel();
        let waiting = Arc::clone(&cell);
        let waiter = std::thread::spawn(move || {
            send.send(waiting.wait()).unwrap();
        });
        let premature = result.recv_timeout(Duration::from_millis(50));
        lock.unlock().unwrap();
        assert!(matches!(premature, Err(mpsc::RecvTimeoutError::Timeout)));
        let after = result.recv_timeout(Duration::from_secs(10)).unwrap();
        waiter.join().unwrap();
        assert!(after.observation.refresh > before.refresh);
        assert_eq!(after.snapshot.unwrap().observation, after.observation);
    }

    #[test]
    fn newer_refresh_does_not_settle_with_an_older_failure() {
        let shared = Shared::default();
        shared.update(|value| value.observation.refresh = 2);
        shared.fail_attempt(std::io::Error::other("old failure").into(), 1);
        let read = shared.value.lock().unwrap();
        assert!(read.error.is_none());
        assert!(read.progress.active());
    }

    #[test]
    fn detected_shrink_is_not_reopened_as_a_fresh_valid_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        append(&path, "one");
        let shared = Shared::default();
        let mut session = Session::new(options(&path));
        step(&mut session, &shared, true);
        // Model a shrinking stat, NEVER mutate a file whose bytes are mapped.
        session.opened.as_mut().unwrap().stamp.length += 1;
        step(&mut session, &shared, true);
        assert!(session.opened.as_ref().unwrap().invalid);
        assert!(shared.value.lock().unwrap().snapshot.is_none());
        assert!(shared.value.lock().unwrap().error.is_some());
        step(&mut session, &shared, true);
        assert!(session.opened.as_ref().unwrap().invalid);
    }

    #[test]
    fn cancellation_never_publishes_a_partial_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        append(&path, "one");
        let mut session = Session::new(options(&path));
        let shared = Shared::default();
        assert!(!session
            .update(&shared, &AtomicBool::new(true), true)
            .unwrap());
        assert!(shared.value.lock().unwrap().snapshot.is_none());
        assert_eq!(session.opened.as_ref().unwrap().pile.refreshed_len(), 0);
    }

    #[test]
    fn cancellation_after_complete_replay_skips_snapshot_phase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        append(&path, "one complete record");
        let mut pile = Pile::with_host(PileFile::open_read_only(&path).unwrap(), None);
        let shared = Shared::default();
        let checks = std::cell::Cell::new(0);
        let result = prepare_snapshot(&mut pile, &shared, || {
            let check = checks.get() + 1;
            checks.set(check);
            // First check admits the record, second observes its completed
            // target, third is the separate before-snapshot cancellation seam.
            check >= 3
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(
            pile.refreshed_len() as u64,
            std::fs::metadata(&path).unwrap().len()
        );
        let read = shared.value.lock().unwrap();
        assert_eq!(read.progress.replay_fraction(), Some(1.0));
        assert_eq!(read.progress.phase, Phase::Replay);
        assert!(read.snapshot.is_none());
    }

    #[test]
    fn drop_does_not_join_a_native_file_lock_wait() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pile");
        File::create(&path).unwrap();
        append(&path, "one");
        let lock = File::open(&path).unwrap();
        lock.lock().unwrap();
        let cell = PileCell::new(options(&path));
        await_read(&cell, |read| read.progress.phase == Phase::Replay);
        let weak = Arc::downgrade(&cell.shared);
        let (send, dropped) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(cell);
            send.send(()).unwrap();
        });
        let result = dropped.recv_timeout(Duration::from_secs(2));
        lock.unlock().unwrap();
        assert!(result.is_ok(), "Drop joined a locked native read");
        dropper.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while weak.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "owner did not stop after lock released"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn gui_read_never_waits_for_publication_lock() {
        let shared = Arc::new(Shared::default());
        let cell = PileCell {
            path: PathBuf::from("/synthetic/locked-publication.pile"),
            refresh: None,
            stop: Arc::new(AtomicBool::new(false)),
            shared: Arc::clone(&shared),
        };
        let _held = shared.value.lock().unwrap();
        assert!(cell.read().is_none());
        let _ = egui::Context::default().run_ui(egui::RawInput::default(), |ui| cell.show(ui));
    }
}
