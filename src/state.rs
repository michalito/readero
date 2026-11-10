//! SQLite persistence for documents, bookmarks and notebooks.
//!
//! The store is a stateless repository owned by the reading-state worker. It
//! resolves identity only from stored data: an id match beats a path match,
//! and the stored path is authoritative. Mapping provisional sessions to their
//! stored identities is the worker's job, done before records reach the store.
use crate::document::*;
use crate::notebook::Notebook;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Readero refuses databases with a newer schema and leaves them unchanged.
///
/// Bump this, with a migration (even an empty one), whenever a stored payload
/// changes shape. Older builds read an unreadable reading position as the
/// start of the document, so an unfenced format change would let them
/// overwrite newer data with defaults.
const SCHEMA_VERSION: usize = 3;

/// `MIGRATIONS[n]` upgrades schema version `n` to `n + 1`.
const MIGRATIONS: [&str; SCHEMA_VERSION] = [
    "CREATE TABLE documents (id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE, payload TEXT NOT NULL,
        last_opened INTEGER NOT NULL, hidden INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE bookmarks (id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(id),
        payload TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT (unixepoch()));
    CREATE INDEX bookmarks_document ON bookmarks(document_id);",
    "CREATE TABLE notebooks (
        document_id TEXT PRIMARY KEY REFERENCES documents(id), payload TEXT NOT NULL);",
    // No table changes: this fences builds that predate notebook payload
    // version 2 (sections and captured passages).
    "",
];

const DOCUMENT_COLUMNS: &str = "id, path, payload, last_opened";

/// Where a stored document lives: its permanent id and current source path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub path: PathBuf,
}
impl Identity {
    fn of(record: &DocumentRecord) -> Self {
        Self {
            id: record.id.clone(),
            path: record.path.clone(),
        }
    }
}

/// Saved reading data that could not be decoded and was replaced by defaults.
/// Identity is unaffected, so bookmarks and the notebook stay attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repair {
    /// Only the reading position was unusable.
    Position(String),
    /// The whole record was unusable: position and appearance were reset.
    Record(String),
}
impl Repair {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Position(_) => {
                "The saved reading position couldn’t be read, so this document opens at the beginning."
            }
            Self::Record(_) => {
                "The saved reading position and appearance couldn’t be read, so this document opens at the beginning with default appearance."
            }
        }
    }

    /// The underlying decoding or validation failure, for diagnostics.
    pub fn detail(&self) -> &str {
        match self {
            Self::Position(detail) | Self::Record(detail) => detail,
        }
    }
}

pub struct LoadedDocument {
    pub record: DocumentRecord,
    pub repair: Option<Repair>,
}

pub(crate) struct Relinked {
    pub record: DocumentRecord,
    /// Ids of the retained sessions that Locate merged into `record`.
    pub sessions: Vec<String>,
    /// The duplicate that owned the path and was merged into `record`.
    pub absorbed: Option<String>,
}

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        restrict_permissions(path)?;
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        // Nothing is written before the version and integrity checks pass.
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let version = usize::try_from(version)
            .ok()
            .filter(|version| *version <= SCHEMA_VERSION)
            .ok_or_else(|| {
                Error::Invalid(
                    "This reading database was created by a newer Readero. It has been left unchanged."
                        .into(),
                )
            })?;
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(Error::Invalid(
                "The reading database needs recovery. It has been left unchanged.".into(),
            ));
        }
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        for (from, migration) in MIGRATIONS.iter().enumerate().skip(version) {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(migration)?;
            transaction.pragma_update(None, "user_version", from as i64 + 1)?;
            transaction.commit()?;
        }
        Ok(Self { connection })
    }

    /// The stored document at `path`, or a new unsaved record for it.
    pub fn document(&self, path: &Path) -> Result<LoadedDocument> {
        let path = std::fs::canonicalize(path)?;
        let stored = self
            .connection
            .query_row(
                &format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE path=?1"),
                [path.to_string_lossy()],
                StoredRow::read,
            )
            .optional()?;
        match stored {
            Some(row) => row.decode(),
            None => Ok(LoadedDocument {
                record: DocumentRecord::new(path)?,
                repair: None,
            }),
        }
    }

    pub fn recents(&self) -> Result<Vec<DocumentRecord>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {DOCUMENT_COLUMNS} FROM documents WHERE hidden=0
                ORDER BY last_opened DESC LIMIT 30"
        ))?;
        let rows = statement.query_map([], StoredRow::read)?;
        rows.map(|row| Ok(row?.decode()?.record)).collect()
    }

    /// The stored identity a record belongs to: its id if that is stored,
    /// otherwise whichever document is stored at its path.
    pub fn resolve(&self, record: &DocumentRecord) -> Result<Option<Identity>> {
        resolve(&self.connection, record)
    }

    /// Store a record under its own id and restore it to recents.
    pub fn save(&self, record: &DocumentRecord) -> Result<()> {
        upsert(&self.connection, record)
    }

    /// Save a session under the identity it resolves to. The stored id and
    /// path win; the session's position and appearance win.
    pub fn save_session(&mut self, session: &DocumentRecord) -> Result<Identity> {
        let transaction = self.write()?;
        let mut record = session.clone();
        if let Some(identity) = resolve(&transaction, session)? {
            record.id = identity.id;
            record.path = identity.path;
        }
        upsert(&transaction, &record)?;
        transaction.commit()?;
        Ok(Identity::of(&record))
    }

    pub fn notebook(&self, record: &DocumentRecord) -> Result<Notebook> {
        match resolve(&self.connection, record)? {
            Some(identity) => stored_notebook(&self.connection, &identity.id),
            None => Ok(Notebook::default()),
        }
    }

    /// Save the notebook of the document a record resolves to. A document is
    /// created only when its identity is unknown, so a retry can neither
    /// restore a removed recent nor overwrite newer reading progress. A stored
    /// notebook that is unreadable or newer is never replaced.
    pub fn save_notebook(
        &mut self,
        record: &DocumentRecord,
        notebook: &Notebook,
    ) -> Result<Identity> {
        notebook.validate()?;
        let transaction = self.write()?;
        let identity = match resolve(&transaction, record)? {
            Some(identity) => identity,
            None => {
                upsert(&transaction, record)?;
                Identity::of(record)
            }
        };
        stored_notebook(&transaction, &identity.id)?;
        transaction.execute(
            "INSERT INTO notebooks(document_id,payload) VALUES(?1,?2)
             ON CONFLICT(document_id) DO UPDATE SET payload=excluded.payload",
            params![identity.id, serde_json::to_string(notebook)?],
        )?;
        transaction.commit()?;
        Ok(identity)
    }

    pub fn hide(&self, id: &str) -> Result<()> {
        self.connection
            .execute("UPDATE documents SET hidden=1 WHERE id=?1", [id])?;
        Ok(())
    }

    /// Bookmarks in creation order. `created_at` has one-second resolution;
    /// the rowid orders bookmarks created within the same second.
    pub fn bookmarks(&self, id: &str) -> Result<Vec<Bookmark>> {
        let mut statement = self.connection.prepare(
            "SELECT payload FROM bookmarks WHERE document_id=?1 ORDER BY created_at, rowid",
        )?;
        let rows = statement.query_map([id], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn add_bookmark(&self, bookmark: &Bookmark) -> Result<()> {
        bookmark.locator.validate()?;
        self.connection.execute(
            "INSERT INTO bookmarks(id,document_id,payload) VALUES(?1,?2,?3)",
            params![
                bookmark.id,
                bookmark.document_id,
                serde_json::to_string(bookmark)?
            ],
        )?;
        Ok(())
    }

    pub fn remove_bookmark(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM bookmarks WHERE id=?1", [id])?;
        Ok(())
    }

    /// Locate: reconnect document `id` to the file at `path`.
    ///
    /// If another stored document already owns `path`, because the reader
    /// opened the file from its new place first, it is a duplicate. It is
    /// merged in when it holds nothing the reader created: no bookmarks and no
    /// notebook content. `drafts` are other documents' retained notebook drafts
    /// with content; one for the file at `path` also prevents merging.
    ///
    /// `sessions` are retained snapshots that have not been saved. Those for
    /// this document, the duplicate, or the file at `path` are resolved here
    /// and returned in `Relinked::sessions`. The most recently opened progress
    /// wins, whether stored, the duplicate's, or retained; a retained snapshot
    /// wins a tie because it is the unsaved state of that same session.
    pub(crate) fn relink(
        &mut self,
        id: &str,
        path: &Path,
        sessions: &[DocumentRecord],
        drafts: &[DocumentRecord],
    ) -> Result<Relinked> {
        let target = std::fs::canonicalize(path)?;
        let transaction = self.write()?;
        let mut record = transaction
            .query_row(
                &format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE id=?1"),
                [id],
                StoredRow::read,
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("This document is no longer in your library.".into()))?
            .decode()?
            .record;
        if Format::from_path(&target)? != record.format {
            return Err(Error::Invalid(
                "Choose a document of the same format.".into(),
            ));
        }
        let duplicate = transaction
            .query_row(
                &format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE path=?1 AND id<>?2"),
                params![target.to_string_lossy(), id],
                StoredRow::read,
            )
            .optional()?
            .map(|row| row.decode().map(|loaded| loaded.record))
            .transpose()?;
        let mut created = drafts.iter().any(|draft| draft.path == target);
        if let Some(duplicate) = &duplicate {
            created = created
                || transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE document_id=?1)",
                    [&duplicate.id],
                    |row| row.get(0),
                )?
                // An unreadable notebook counts as the reader's content.
                || stored_notebook(&transaction, &duplicate.id)
                    .map_or(true, |notebook| notebook.has_content());
        }
        if created {
            return Err(Error::Invalid(
                "This file is already in your library as a separate document with its own bookmarks or notebook, so Locate can’t combine them. Open the file directly to continue reading that document."
                    .into(),
            ));
        }
        if let Some(duplicate) = &duplicate {
            transaction.execute(
                "DELETE FROM notebooks WHERE document_id=?1",
                [&duplicate.id],
            )?;
            transaction.execute("DELETE FROM documents WHERE id=?1", [&duplicate.id])?;
            if duplicate.locator.is_some() && duplicate.last_opened > record.last_opened {
                adopt_progress(&mut record, duplicate);
            }
        }
        let merged: Vec<&DocumentRecord> = sessions
            .iter()
            .filter(|session| {
                session.id == record.id
                    || session.path == record.path
                    || session.path == target
                    || duplicate.as_ref().is_some_and(|d| session.id == d.id)
            })
            .collect();
        if let Some(newest) = merged.iter().max_by_key(|session| session.last_opened)
            && newest.last_opened >= record.last_opened
        {
            adopt_progress(&mut record, newest);
        }
        record.path = target;
        upsert(&transaction, &record)?;
        transaction.commit()?;
        Ok(Relinked {
            sessions: merged.iter().map(|session| session.id.clone()).collect(),
            absorbed: duplicate.map(|duplicate| duplicate.id),
            record,
        })
    }

    /// Writes take the database lock up front, so a read inside the
    /// transaction cannot be invalidated before its dependent write.
    fn write(&mut self) -> Result<Transaction<'_>> {
        Ok(self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?)
    }
}

/// Notebook text and quotations share this database. Restrict the directory
/// (including SQLite's auxiliary files) and the database before SQLite can
/// initialize, migrate, or recover them. Permission failures abort opening.
fn restrict_permissions(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut directory = std::fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A `documents` row before its payload is decoded.
struct StoredRow {
    id: String,
    path: String,
    payload: String,
    last_opened: i64,
}
impl StoredRow {
    fn read(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            path: row.get(1)?,
            payload: row.get(2)?,
            last_opened: row.get(3)?,
        })
    }

    /// The columns are authoritative for identity. An unreadable payload
    /// loses only reconstructible reading state, never the identity that
    /// bookmarks and the notebook belong to.
    fn decode(self) -> Result<LoadedDocument> {
        let path = PathBuf::from(self.path);
        let (mut record, repair) = match serde_json::from_str::<DocumentRecord>(&self.payload) {
            Ok(mut record) => {
                record.settings.normalize();
                let invalid = record.locator.as_ref().and_then(|l| l.validate().err());
                if invalid.is_some() {
                    record.locator = None;
                }
                (
                    record,
                    invalid.map(|error| Repair::Position(error.to_string())),
                )
            }
            Err(error) => (
                DocumentRecord::new(path.clone())?,
                Some(Repair::Record(error.to_string())),
            ),
        };
        record.id = self.id;
        record.path = path;
        record.last_opened = self.last_opened;
        Ok(LoadedDocument { record, repair })
    }
}

fn resolve(connection: &Connection, record: &DocumentRecord) -> Result<Option<Identity>> {
    Ok(connection
        .query_row(
            "SELECT id, path FROM documents WHERE id=?1 OR path=?2
                ORDER BY (id=?1) DESC LIMIT 1",
            params![record.id, record.path.to_string_lossy()],
            |row| {
                Ok(Identity {
                    id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                })
            },
        )
        .optional()?)
}

fn upsert(connection: &Connection, record: &DocumentRecord) -> Result<()> {
    if let Some(locator) = &record.locator {
        locator.validate()?;
    }
    connection.execute(
        "INSERT INTO documents(id,path,payload,last_opened) VALUES(?1,?2,?3,?4)
            ON CONFLICT(id) DO UPDATE SET path=excluded.path, payload=excluded.payload,
                last_opened=excluded.last_opened, hidden=0",
        params![
            record.id,
            record.path.to_string_lossy(),
            serde_json::to_string(record)?,
            record.last_opened
        ],
    )?;
    Ok(())
}

fn stored_notebook(connection: &Connection, id: &str) -> Result<Notebook> {
    let json: Option<String> = connection
        .query_row(
            "SELECT payload FROM notebooks WHERE document_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|json| serde_json::from_str::<Notebook>(&json))
        .transpose()?
        .unwrap_or_default()
        .upgrade()
}

/// Take another snapshot's reading progress, keeping this record's identity.
fn adopt_progress(record: &mut DocumentRecord, newer: &DocumentRecord) {
    record.locator = newer.locator.clone();
    record.settings = newer.settings.clone();
    record.revision = newer.revision.clone();
    record.last_opened = newer.last_opened;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn new_database_is_private_without_changing_ancestor_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let app = temp.path().join("readero");
        let database = app.join("reading.sqlite3");
        let store = Store::open(&database).unwrap();
        assert!(store.recents().unwrap().is_empty());
        for (path, mode) in [
            (temp.path(), 0o755),
            (app.as_path(), 0o700),
            (database.as_path(), 0o600),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn opening_repairs_public_permissions_and_preserves_notebook() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("book.md");
        std::fs::write(&source, "# A book").unwrap();
        let app = temp.path().join("readero");
        let database = app.join("reading.sqlite3");
        let mut store = Store::open(&database).unwrap();
        let record = store.document(&source).unwrap().record;
        let notebook = Notebook {
            text: "Private notebook text".into(),
            ..Notebook::default()
        };
        store.save_notebook(&record, &notebook).unwrap();
        drop(store);
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
        let store = Store::open(&database).unwrap();
        assert_eq!(store.notebook(&record).unwrap(), notebook);
        assert_eq!(
            std::fs::metadata(&app).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn progress_survives_reopen_and_recents_removal_keeps_bookmarks() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("book.md");
        std::fs::write(&source, "# A book").unwrap();
        let path = temp.path().join("state.sqlite3");
        let store = Store::open(&path).unwrap();
        let mut record = store.document(&source).unwrap().record;
        record.locator = Some(Locator {
            version: 1,
            anchor: Anchor::Reflow {
                href: "content.xhtml".into(),
                cfi: "epubcfi(/6/2!/4/2)".into(),
                section: 0,
                fraction: 0.4,
                quote: "a passage".into(),
                block: "b1".into(),
                block_offset: None,
                quote_offset: 0,
            },
        });
        store.save(&record).unwrap();
        let bookmark = Bookmark {
            id: "bookmark".into(),
            document_id: record.id.clone(),
            label: "A passage".into(),
            locator: record.locator.clone().unwrap(),
        };
        store.add_bookmark(&bookmark).unwrap();
        store.hide(&record.id).unwrap();
        assert!(store.recents().unwrap().is_empty());
        assert_eq!(store.bookmarks(&record.id).unwrap().len(), 1);
        drop(store);
        let mut reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened.document(&source).unwrap().record.locator,
            record.locator
        );
        let moved = temp.path().join("renamed.md");
        std::fs::rename(source, &moved).unwrap();
        assert_eq!(
            reopened
                .relink(&record.id, &moved, &[], &[])
                .unwrap()
                .record
                .id,
            record.id
        );
    }
    #[test]
    fn temporary_lock_reconnects_identity_without_losing_session_progress() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("book.md");
        std::fs::write(&source, "# A book").unwrap();
        let path = temp.path().join("state.sqlite3");
        let mut store = Store::open(&path).unwrap();
        store
            .connection
            .busy_timeout(Duration::from_millis(20))
            .unwrap();
        let original = store.document(&source).unwrap().record;
        store.save(&original).unwrap();
        let bookmark = Bookmark {
            id: "existing-bookmark".into(),
            document_id: original.id.clone(),
            label: "Saved before the lock".into(),
            locator: Locator {
                version: 1,
                anchor: Anchor::Pdf {
                    page: 2,
                    x: 0.0,
                    y: 0.0,
                    viewport_y: 0.0,
                },
            },
        };
        store.add_bookmark(&bookmark).unwrap();
        let locked = Connection::open(&path).unwrap();
        locked.execute_batch("BEGIN EXCLUSIVE").unwrap();
        assert!(store.document(&source).is_err());
        let mut session = DocumentRecord::new(source.clone()).unwrap();
        assert_ne!(session.id, original.id);
        session.locator = Some(Locator {
            version: 1,
            anchor: Anchor::Pdf {
                page: 5,
                x: 0.0,
                y: 0.0,
                viewport_y: 0.0,
            },
        });
        session.settings.font_size = 25.0;
        session.revision = "current-session-revision".into();
        assert!(store.save_session(&session).is_err());
        locked.execute_batch("ROLLBACK").unwrap();
        let recovered_id = store.save_session(&session).unwrap().id;
        assert_eq!(recovered_id, original.id);
        let recovered = store.document(&source).unwrap().record;
        assert_eq!(recovered.locator, session.locator);
        assert_eq!(recovered.settings, session.settings);
        assert_eq!(recovered.revision, session.revision);
        // A second save may already be queued with the provisional ID.
        assert_eq!(store.save_session(&session).unwrap().id, original.id);
        store
            .add_bookmark(&Bookmark {
                id: "new-bookmark".into(),
                document_id: recovered_id,
                ..bookmark
            })
            .unwrap();
        assert_eq!(store.bookmarks(&original.id).unwrap().len(), 2);
        assert_eq!(store.recents().unwrap().len(), 1);
    }

    #[test]
    fn failed_snapshot_retry_preserves_relinked_path_and_bookmarks() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("book.md");
        let moved = temp.path().join("moved.md");
        std::fs::write(&source, "# A book").unwrap();
        let database = temp.path().join("state.sqlite3");
        let mut store = Store::open(&database).unwrap();
        store
            .connection
            .busy_timeout(Duration::from_millis(20))
            .unwrap();
        let mut snapshot = store.document(&source).unwrap().record;
        store.save(&snapshot).unwrap();
        let locator = Locator {
            version: 1,
            anchor: Anchor::Reflow {
                href: "content.xhtml".into(),
                cfi: "epubcfi(/6/2!/4/2)".into(),
                section: 0,
                fraction: 0.4,
                quote: "a passage".into(),
                block: "b1".into(),
                block_offset: None,
                quote_offset: 0,
            },
        };
        store
            .add_bookmark(&Bookmark {
                id: "bookmark".into(),
                document_id: snapshot.id.clone(),
                label: "A passage".into(),
                locator: locator.clone(),
            })
            .unwrap();
        snapshot.locator = Some(locator);
        snapshot.settings.font_size = 25.0;
        let locked = Connection::open(&database).unwrap();
        locked.execute_batch("BEGIN EXCLUSIVE").unwrap();
        assert!(store.save_session(&snapshot).is_err());
        locked.execute_batch("ROLLBACK").unwrap();
        std::fs::rename(&source, &moved).unwrap();
        store.relink(&snapshot.id, &moved, &[], &[]).unwrap();

        // The old path may even belong to another document by retry time.
        std::fs::write(&source, "# Another book").unwrap();
        let other = store.document(&source).unwrap().record;
        store.save(&other).unwrap();
        for _ in 0..2 {
            assert_eq!(store.save_session(&snapshot).unwrap().id, snapshot.id);
            let recovered = store.document(&moved).unwrap().record;
            assert_eq!(recovered.id, snapshot.id);
            assert_eq!(recovered.path, std::fs::canonicalize(&moved).unwrap());
            assert_eq!(recovered.locator, snapshot.locator);
            assert_eq!(recovered.settings, snapshot.settings);
            assert_eq!(store.bookmarks(&recovered.id).unwrap().len(), 1);
            assert_eq!(store.document(&source).unwrap().record.id, other.id);
        }
    }

    #[test]
    fn newer_database_is_not_recreated() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 20).unwrap();
        drop(connection);
        assert!(Store::open(&path).is_err());
        assert_eq!(
            Connection::open(path)
                .unwrap()
                .pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            20
        );
    }
}
