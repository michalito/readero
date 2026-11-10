//! Reading-state coordination, including retained saves and document identity.
//!
//! Every operation enters one worker queue when called, before its reply is
//! awaited. Dropping or delaying a reply never cancels the operation. SQLite,
//! retained snapshots, and identity reconciliation have the same owner. The
//! desktop supplies stable snapshots and owns its debounce and native widgets.
//!
//! A document opened while storage is unavailable gets a provisional record
//! with a random id. When one of its snapshots reaches the store, the worker
//! records the stored identity that session belongs to. Every record is
//! reconciled with these identities before it reaches the store, so a session
//! keeps following its document through Locate, even if another file later
//! takes its old path.
use crate::{
    document::*,
    notebook::Notebook,
    state::{Identity, LoadedDocument, Repair, Store},
};
use std::{
    collections::HashMap,
    future::{Future, IntoFuture},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, mpsc},
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
        lock(&self.observed).status.error = Some(error.to_string());
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
    /// Saved reading data that could not be read and was reset. Saving works.
    pub repair: Option<Repair>,
}

#[derive(Clone, Default)]
pub struct SaveStatus {
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct Observed {
    status: SaveStatus,
    /// Session id → the stored identity it belongs to. Stored ids map to
    /// themselves, so Locate can update the path every session follows.
    identities: HashMap<String, Identity>,
}

impl Observed {
    /// Apply only acknowledged identity, never a snapshot's progress.
    fn reconcile(&self, record: &mut DocumentRecord) -> bool {
        let Some(identity) = self.identities.get(&record.id) else {
            return false;
        };
        let changed = record.id != identity.id || record.path != identity.path;
        record.id.clone_from(&identity.id);
        record.path.clone_from(&identity.path);
        changed
    }

    fn same_document(&self, a: &DocumentRecord, b: &DocumentRecord) -> bool {
        let (a_id, a_path) = self.identity(a);
        let (b_id, b_path) = self.identity(b);
        a_id == b_id || a_path == b_path
    }

    fn identity<'a>(&'a self, record: &'a DocumentRecord) -> (&'a str, &'a Path) {
        self.identities
            .get(&record.id)
            .map_or((&record.id, record.path.as_path()), |identity| {
                (&identity.id, identity.path.as_path())
            })
    }
}

fn lock(observed: &Mutex<Observed>) -> MutexGuard<'_, Observed> {
    observed.lock().expect("reading-state observation")
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
                    notebooks: Vec::new(),
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

    /// A failed load is never an empty notebook. Retained drafts take precedence
    /// over disk, including after a provisional document has been reconciled.
    pub fn notebook(&self, record: DocumentRecord) -> Reply<Notebook> {
        self.call(move |state| {
            let record = state.reconciled(record);
            let observed = lock(&state.observed);
            if let Some((_, notebook)) = state
                .notebooks
                .iter()
                .rev()
                .find(|(draft, _)| observed.same_document(draft, &record))
            {
                return Ok(notebook.clone());
            }
            drop(observed);
            state.store()?.notebook(&record)
        })
    }

    /// Retain the latest complete draft before attempting any writes. Replies
    /// acknowledge queue completion, never ownership of the live editor buffer.
    pub fn save_notebook(&self, record: DocumentRecord, notebook: Notebook) -> Reply<()> {
        self.call(move |state| {
            notebook.validate()?;
            let record = state.reconciled(record);
            let observed = lock(&state.observed);
            state
                .notebooks
                .retain(|(draft, _)| !observed.same_document(draft, &record));
            drop(observed);
            state.notebooks.push((record, notebook));
            let result = state.flush();
            state.publish(&result);
            result
        })
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
        self.call(move |state| {
            let id = state.identity(&id);
            state.store()?.bookmarks(&id)
        })
    }

    pub fn remove_bookmark(&self, id: String) -> Reply<()> {
        self.call(move |state| state.store()?.remove_bookmark(&id))
    }

    /// Always read the latest completed state, not the state of a delayed reply.
    pub fn status(&self) -> SaveStatus {
        lock(&self.observed).status.clone()
    }

    /// Apply only acknowledged identity, never overwrite the reader's newer
    /// position or appearance with a completed save's snapshot.
    pub fn reconcile(&self, record: &mut DocumentRecord) -> bool {
        lock(&self.observed).reconcile(record)
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
            lock(&self.observed).status.error = Some(stopped().to_string());
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
    notebooks: Vec<(DocumentRecord, Notebook)>,
    observed: Arc<Mutex<Observed>>,
}
impl State {
    fn store(&mut self) -> Result<&mut Store> {
        self.store.as_mut().map_err(|error| unavailable(error))
    }

    fn reconciled(&self, mut record: DocumentRecord) -> DocumentRecord {
        lock(&self.observed).reconcile(&mut record);
        record
    }

    fn identity(&self, id: &str) -> String {
        lock(&self.observed)
            .identities
            .get(id)
            .map_or_else(|| id.to_owned(), |identity| identity.id.clone())
    }

    fn document(&mut self, path: &Path) -> Result<OpenedDocument> {
        let (mut record, mut repair, storage_error) =
            match self.store().and_then(|store| store.document(path)) {
                Ok(LoadedDocument { record, repair }) => (record, repair, None),
                // Reading stays possible. Stored paths are canonical, so the
                // provisional record must be too for later saves to find it.
                Err(error) if path.is_file() => (
                    DocumentRecord::new(std::fs::canonicalize(path)?)?,
                    None,
                    Some(error.to_string()),
                ),
                Err(error) => return Err(error),
            };
        let observed = lock(&self.observed);
        if let Some(retained) = self
            .pending
            .iter()
            .rev()
            .find(|retained| observed.same_document(retained, &record))
        {
            record.locator = retained.locator.clone();
            record.settings = retained.settings.clone();
            repair = None;
        }
        Ok(OpenedDocument {
            record,
            storage_error,
            repair,
        })
    }

    fn retain(&mut self, record: DocumentRecord) -> DocumentRecord {
        let record = self.reconciled(record);
        self.pending
            .retain(|r| r.id != record.id && r.path != record.path);
        self.pending.push(record.clone());
        record
    }

    fn persist(&mut self, record: &DocumentRecord) -> Result<Identity> {
        let saved = self.store()?.save_session(record)?;
        self.acknowledge(&record.id, &saved);
        Ok(saved)
    }

    /// Bind `session` to a stored identity. Sessions already bound to that
    /// identity follow its path, and sessions bound to `session` itself (when
    /// it is a stored id merged into another document) follow it there.
    fn acknowledge(&self, session: &str, saved: &Identity) {
        let mut observed = lock(&self.observed);
        for identity in observed.identities.values_mut() {
            if identity.id == saved.id || identity.id == session {
                identity.clone_from(saved);
            }
        }
        observed
            .identities
            .insert(session.to_owned(), saved.clone());
        observed.identities.insert(saved.id.clone(), saved.clone());
    }

    fn flush(&mut self) -> Result<()> {
        self.store()?;
        let mut first_error = None;
        for record in std::mem::take(&mut self.pending) {
            // Identity may have been acknowledged since this was retained.
            let record = self.reconciled(record);
            if let Err(error) = self.persist(&record) {
                self.pending.push(record);
                first_error.get_or_insert(error);
            }
        }
        for (record, notebook) in std::mem::take(&mut self.notebooks) {
            let record = self.reconciled(record);
            match self
                .store()
                .and_then(|store| store.save_notebook(&record, &notebook))
            {
                Ok(saved) => self.acknowledge(&record.id, &saved),
                Err(error) => {
                    self.notebooks.push((record, notebook));
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn locate(&mut self, id: &str, path: &Path) -> Result<DocumentRecord> {
        let pending = std::mem::take(&mut self.pending);
        self.pending = pending.into_iter().map(|r| self.reconciled(r)).collect();
        let notebooks = std::mem::take(&mut self.notebooks);
        self.notebooks = notebooks
            .into_iter()
            .map(|(draft, notebook)| (self.reconciled(draft), notebook))
            .collect();
        let store = self.store.as_mut().map_err(|error| unavailable(error))?;
        // Drafts are bound by stored identity before the path changes.
        let owned = self
            .notebooks
            .iter()
            .map(|(draft, _)| Ok(store.resolve(draft)?.is_some_and(|owner| owner.id == id)))
            .collect::<Result<Vec<bool>>>()?;
        let others_written: Vec<DocumentRecord> = self
            .notebooks
            .iter()
            .zip(&owned)
            .filter(|((_, notebook), own)| !**own && notebook.has_content())
            .map(|((draft, _), _)| draft.clone())
            .collect();
        let relinked = store.relink(id, path, &self.pending, &others_written)?;
        let located = Identity {
            id: relinked.record.id.clone(),
            path: relinked.record.path.clone(),
        };
        self.acknowledge(id, &located);
        for session in relinked.sessions.iter().chain(&relinked.absorbed) {
            self.acknowledge(session, &located);
        }
        // Locate has durably written the newest retained progress.
        self.pending.retain(|r| !relinked.sessions.contains(&r.id));
        let notebooks = std::mem::take(&mut self.notebooks);
        for ((draft, notebook), own) in notebooks.into_iter().zip(owned) {
            if own {
                self.acknowledge(&draft.id, &located);
                self.notebooks.push((draft, notebook));
            } else if draft.path != located.path {
                self.notebooks.push((draft, notebook));
            }
            // Otherwise the draft belonged to the merged duplicate and had no
            // content. Saving it would replace this document's notebook.
        }
        Ok(relinked.record)
    }

    fn publish<T>(&self, result: &Result<T>) {
        let mut observed = lock(&self.observed);
        observed.status.pending = !self.pending.is_empty() || !self.notebooks.is_empty();
        match result {
            Err(error) => observed.status.error = Some(error.to_string()),
            Ok(_) if !observed.status.pending => observed.status.error = None,
            Ok(_) => {}
        }
    }
}

fn unavailable(error: &Error) -> Error {
    Error::Invalid(error.to_string())
}

fn stopped() -> Error {
    Error::Invalid("The reading-state worker stopped.".into())
}
