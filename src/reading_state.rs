//! Reading-state coordination, including retained saves and document identity.
//!
//! Every operation enters one worker queue when called, before its reply is
//! awaited. Dropping or delaying a reply never cancels the operation. SQLite,
//! retained snapshots, and identity reconciliation have the same owner. The
//! desktop supplies stable snapshots and owns its debounce and native widgets.
use crate::{document::*, state::Store};
use std::{
    collections::HashMap,
    future::{Future, IntoFuture},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, mpsc},
};

type Work = Box<dyn FnOnce(&mut State) + Send>;

/// A reply to an already queued operation. Both waiting forms observe the same
/// operation; `wait` is useful for callers without an asynchronous event loop.
pub struct Reply<T> {
    receiver: async_channel::Receiver<Result<T>>,
    observed: Arc<Mutex<Observed>>,
}
impl<T> Reply<T> {
    pub fn wait(self) -> Result<T> {
        self.receiver.recv_blocking().map_err(|_| self.stopped())?
    }
    fn stopped(&self) -> Error {
        let error = stopped();
        self.observed
            .lock()
            .expect("reading-state observation")
            .status
            .error = Some(error.to_string());
        error
    }
}
impl<T: Send + 'static> IntoFuture for Reply<T> {
    type Output = Result<T>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.receiver.recv().await.map_err(|_| self.stopped())? })
    }
}

/// Opening remains possible during a storage failure. The error accompanies
/// the provisional document so the desktop can explain that saving is impaired.
pub struct OpenedDocument {
    pub record: DocumentRecord,
    pub storage_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct SaveStatus {
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct Observed {
    status: SaveStatus,
    identities: HashMap<String, (String, PathBuf)>,
}

#[derive(Clone)]
pub struct ReadingState {
    sender: mpsc::Sender<Work>,
    observed: Arc<Mutex<Observed>>,
}
impl ReadingState {
    pub fn start(database: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel::<Work>();
        let observed = Arc::new(Mutex::new(Observed::default()));
        let shared = Arc::clone(&observed);
        std::thread::Builder::new()
            .name("readero-state".into())
            .spawn(move || {
                let mut state = State {
                    store: Store::open(&database),
                    database,
                    pending: Vec::new(),
                    observed: shared,
                };
                while let Ok(work) = receiver.recv() {
                    if state.store.is_err() {
                        state.store = Store::open(&state.database);
                    }
                    work(&mut state);
                }
            })
            .expect("The reading-state worker could not be started");
        Self { sender, observed }
    }

    pub fn document(&self, path: PathBuf) -> Reply<OpenedDocument> {
        self.call(move |state| state.document(&path))
    }

    /// Retain the supplied stable snapshot and retry every unsaved document.
    /// `None` retries from Home; awaiting completion also supplies the shutdown
    /// flush. Successful documents are acknowledged even if another save fails.
    pub fn save(&self, record: Option<DocumentRecord>) -> Reply<()> {
        self.call(move |state| {
            if let Some(record) = record {
                state.retain(record);
            }
            let result = state.flush();
            state.publish(&result);
            result
        })
    }

    pub fn recents(&self) -> Reply<Vec<DocumentRecord>> {
        self.call(|state| state.store()?.recents())
    }

    pub fn locate(&self, id: String, path: PathBuf) -> Reply<DocumentRecord> {
        self.call(move |state| {
            let result = state.locate(&id, &path);
            // A rejected file choice does not mean progress saving failed.
            if result.is_ok() {
                state.publish(&result);
            }
            result
        })
    }

    /// A successful removal discards retained retries for this document only.
    /// Failure leaves them intact, and later shutdown cannot undo a removal.
    pub fn remove_recent(&self, id: String, path: PathBuf) -> Reply<()> {
        self.call(move |state| {
            let result = state.store().and_then(|store| store.hide(&id));
            if result.is_ok() {
                state.pending.retain(|r| r.id != id && r.path != path);
            }
            state.publish(&result);
            result
        })
    }

    /// Persist identity and position before adding the bookmark. A failed
    /// bookmark is reported, never silently retried as another bookmark.
    pub fn bookmark(&self, record: DocumentRecord, mut bookmark: Bookmark) -> Reply<()> {
        self.call(move |state| {
            let record = state.retain(record);
            let result = (|| {
                let saved = state.persist(&record)?;
                state
                    .pending
                    .retain(|r| r.id != record.id && r.path != record.path);
                bookmark.document_id = saved.id;
                state.store()?.add_bookmark(&bookmark)
            })();
            state.publish(&result);
            result
        })
    }

    pub fn bookmarks(&self, id: String) -> Reply<Vec<Bookmark>> {
        self.call(move |state| state.store()?.bookmarks(&state.identity(&id)))
    }

    pub fn remove_bookmark(&self, id: String) -> Reply<()> {
        self.call(move |state| state.store()?.remove_bookmark(&id))
    }

    /// Always read the latest completed state, not the state of a delayed reply.
    pub fn status(&self) -> SaveStatus {
        self.observed
            .lock()
            .expect("reading-state observation")
            .status
            .clone()
    }

    /// Apply only acknowledged identity, never overwrite the reader's newer
    /// position or appearance with a completed save's snapshot.
    pub fn reconcile(&self, record: &mut DocumentRecord) -> bool {
        reconcile(
            &self.observed.lock().expect("reading-state observation"),
            record,
        )
    }

    fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut State) -> Result<T> + Send + 'static,
    ) -> Reply<T> {
        let (sender, receiver) = async_channel::bounded(1);
        let failure = sender.clone();
        if self
            .sender
            .send(Box::new(move |state| {
                let _ = sender.send_blocking(work(state));
            }))
            .is_err()
        {
            self.observed
                .lock()
                .expect("reading-state observation")
                .status
                .error = Some(stopped().to_string());
            let _ = failure.send_blocking(Err(stopped()));
        }
        Reply {
            receiver,
            observed: Arc::clone(&self.observed),
        }
    }

    /// Native smoke probes may drain or delay the queue, but cannot mutate the
    /// store or retained snapshots behind the production interface.
    #[cfg(feature = "smoke")]
    pub fn barrier(&self, delay: std::time::Duration) -> Reply<()> {
        self.call(move |_| {
            std::thread::sleep(delay);
            Ok(())
        })
    }
}

struct State {
    database: PathBuf,
    store: Result<Store>,
    // Oldest first. Replacement appends, so Locate selects the newest session.
    pending: Vec<DocumentRecord>,
    observed: Arc<Mutex<Observed>>,
}
impl State {
    fn store(&self) -> Result<&Store> {
        self.store
            .as_ref()
            .map_err(|error| Error::Invalid(error.to_string()))
    }

    fn identity(&self, id: &str) -> String {
        self.observed
            .lock()
            .expect("reading-state observation")
            .identities
            .get(id)
            .map_or_else(|| id.to_owned(), |(id, _)| id.clone())
    }

    fn document(&self, path: &Path) -> Result<OpenedDocument> {
        let loaded = self.store().and_then(|store| store.document(path));
        let (mut record, storage_error) = match loaded {
            Ok(record) => (record, None),
            Err(error) if path.is_file() => (
                DocumentRecord::new(path.to_path_buf())?,
                Some(error.to_string()),
            ),
            Err(error) => return Err(error),
        };
        if let Some(retained) = self.pending.iter().rev().find(|r| r.path == record.path) {
            record.locator = retained.locator.clone();
            record.settings = retained.settings.clone();
        }
        Ok(OpenedDocument {
            record,
            storage_error,
        })
    }

    fn retain(&mut self, mut record: DocumentRecord) -> DocumentRecord {
        reconcile(
            &self.observed.lock().expect("reading-state observation"),
            &mut record,
        );
        self.pending
            .retain(|r| r.id != record.id && r.path != record.path);
        self.pending.push(record.clone());
        record
    }

    fn persist(&self, record: &DocumentRecord) -> Result<DocumentRecord> {
        let saved = self.store()?.save_session_record(record)?;
        self.acknowledge(record, &saved);
        Ok(saved)
    }

    fn acknowledge(&self, session: &DocumentRecord, saved: &DocumentRecord) {
        let mut observed = self.observed.lock().expect("reading-state observation");
        for (id, path) in observed.identities.values_mut() {
            if id == &saved.id {
                *path = saved.path.clone();
            }
        }
        let identity = (saved.id.clone(), saved.path.clone());
        observed
            .identities
            .insert(session.id.clone(), identity.clone());
        observed.identities.insert(saved.id.clone(), identity);
    }

    fn flush(&mut self) -> Result<()> {
        self.store()?;
        let mut error = None;
        for record in std::mem::take(&mut self.pending) {
            if let Err(failed) = self.persist(&record) {
                self.pending.push(record);
                if error.is_none() {
                    error = Some(failed);
                }
            }
        }
        error.map_or(Ok(()), Err)
    }

    fn locate(&mut self, id: &str, path: &Path) -> Result<DocumentRecord> {
        let (record, sessions) = self.store()?.relink_sessions(id, path, &self.pending)?;
        for session in &self.pending {
            if session.id == record.id || sessions.contains(&session.id) {
                self.acknowledge(session, &record);
            }
        }
        self.acknowledge(&record, &record);
        // Locate has already durably written the newest retained state.
        self.pending
            .retain(|r| r.id != record.id && !sessions.contains(&r.id));
        Ok(record)
    }

    fn publish<T>(&self, result: &Result<T>) {
        let mut observed = self.observed.lock().expect("reading-state observation");
        observed.status.pending = !self.pending.is_empty();
        match result {
            Err(error) => observed.status.error = Some(error.to_string()),
            Ok(_) if self.pending.is_empty() => observed.status.error = None,
            Ok(_) => {}
        }
    }
}

fn reconcile(observed: &Observed, record: &mut DocumentRecord) -> bool {
    if let Some((id, path)) = observed.identities.get(&record.id) {
        let changed = &record.id != id || &record.path != path;
        record.id = id.clone();
        record.path = path.clone();
        changed
    } else {
        false
    }
}
fn stopped() -> Error {
    Error::Invalid("The reading-state worker stopped.".into())
}
