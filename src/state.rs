//! A single-owner database. The desktop layer runs this on its state worker.
use crate::document::*;
use crate::notebook::Notebook;
use rusqlite::{Connection, OptionalExtension, params};
use std::{cell::RefCell, collections::HashMap, path::Path, time::Duration};

pub struct Store {
    connection: Connection,
    session_ids: RefCell<HashMap<String, String>>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        // Notebook text and quotations share this database. Restrict the
        // directory (including SQLite's auxiliary files) before opening it.
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
            // Create privately and repair existing stores before SQLite can
            // initialize, migrate, or recover them. Permission failures abort.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 3 {
            return Err(Error::Invalid(
                "This reading database was created by a newer Readero. It has been left unchanged."
                    .into(),
            ));
        }
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(Error::Invalid(
                "The reading database needs recovery. It has been left unchanged.".into(),
            ));
        }
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE documents (id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE, payload TEXT NOT NULL,
                    last_opened INTEGER NOT NULL, hidden INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE bookmarks (id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(id),
                    payload TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT (unixepoch()));
                CREATE INDEX bookmarks_document ON bookmarks(document_id);
                PRAGMA user_version=1;
                COMMIT;")?;
        }
        if version < 2 {
            let transaction = connection.unchecked_transaction()?;
            transaction.execute_batch(
                "CREATE TABLE notebooks (
                document_id TEXT PRIMARY KEY REFERENCES documents(id), payload TEXT NOT NULL);
                PRAGMA user_version=2;",
            )?;
            transaction.commit()?;
        }
        if version < 3 {
            let transaction = connection.unchecked_transaction()?;
            transaction.pragma_update(None, "user_version", 3)?;
            transaction.commit()?;
        }
        Ok(Self {
            connection,
            session_ids: RefCell::new(HashMap::new()),
        })
    }
    pub fn document(&self, path: &Path) -> Result<DocumentRecord> {
        let path = std::fs::canonicalize(path)?;
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM documents WHERE path=?1",
                [path.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .optional()?;
        match saved {
            Some(json) => {
                let mut record: DocumentRecord = serde_json::from_str(&json)?;
                record.settings.normalize();
                if let Some(loc) = &record.locator {
                    loc.validate()?;
                }
                Ok(record)
            }
            None => DocumentRecord::new(path),
        }
    }
    pub fn save(&self, record: &DocumentRecord) -> Result<()> {
        if let Some(loc) = &record.locator {
            loc.validate()?;
        }
        self.connection.execute("INSERT INTO documents(id,path,payload,last_opened) VALUES(?1,?2,?3,?4)
            ON CONFLICT(id) DO UPDATE SET path=excluded.path,payload=excluded.payload,last_opened=excluded.last_opened,hidden=0",
            params![record.id, record.path.to_string_lossy(), serde_json::to_string(record)?, record.last_opened])?;
        Ok(())
    }
    pub fn recents(&self) -> Result<Vec<DocumentRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT payload FROM documents WHERE hidden=0 ORDER BY last_opened DESC LIMIT 30",
        )?;
        let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    /// Reconnect a session opened while storage was unavailable. Only identity
    /// and the authoritative path come from storage; the reader's current
    /// progress and settings win. A retained snapshot must not undo a relink.
    pub fn save_session(&self, record: &DocumentRecord) -> Result<String> {
        self.save_session_record(record).map(|record| record.id)
    }
    pub(crate) fn save_session_record(&self, record: &DocumentRecord) -> Result<DocumentRecord> {
        let session_id = record.id.clone();
        let transaction = self.connection.unchecked_transaction()?;
        let existing = self.stored_identity(record)?;
        let mut record = record.clone();
        if let Some((id, path)) = existing {
            record.id = id;
            record.path = path.into();
        }
        self.save(&record)?;
        transaction.commit()?;
        self.session_ids
            .borrow_mut()
            .insert(session_id, record.id.clone());
        Ok(record)
    }

    pub(crate) fn stored_identity(
        &self,
        record: &DocumentRecord,
    ) -> Result<Option<(String, String)>> {
        let session_ids = self.session_ids.borrow();
        let id = session_ids.get(&record.id).unwrap_or(&record.id);
        Ok(self
            .connection
            .query_row(
                "SELECT id, path FROM documents WHERE id=?2 OR path=?1
                    ORDER BY (id=?2) DESC LIMIT 1",
                params![record.path.to_string_lossy(), id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn notebook(&self, record: &DocumentRecord) -> Result<Notebook> {
        let Some((id, _)) = self.stored_identity(record)? else {
            return Ok(Notebook::default());
        };
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM notebooks WHERE document_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let notebook: Notebook = json
            .map(|json| serde_json::from_str(&json))
            .transpose()?
            .unwrap_or_default();
        notebook.upgrade()
    }

    /// Notebook retries must not resurrect a removed recent or overwrite newer
    /// reading progress. Only create a document when its identity is unknown.
    pub(crate) fn save_notebook(
        &self,
        record: &DocumentRecord,
        notebook: &Notebook,
    ) -> Result<DocumentRecord> {
        notebook.validate()?;
        let transaction = self.connection.unchecked_transaction()?;
        self.notebook(record)?; // Never replace an unreadable/newer payload.
        let mut saved = record.clone();
        if let Some((id, path)) = self.stored_identity(record)? {
            saved.id = id;
            saved.path = path.into();
        } else {
            self.save(&saved)?;
        }
        self.connection.execute(
            "INSERT INTO notebooks(document_id,payload) VALUES(?1,?2)
             ON CONFLICT(document_id) DO UPDATE SET payload=excluded.payload",
            params![saved.id, serde_json::to_string(notebook)?],
        )?;
        transaction.commit()?;
        self.session_ids
            .borrow_mut()
            .insert(record.id.clone(), saved.id.clone());
        Ok(saved)
    }
    pub fn hide(&self, id: &str) -> Result<()> {
        self.connection
            .execute("UPDATE documents SET hidden=1 WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn bookmarks(&self, id: &str) -> Result<Vec<Bookmark>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM bookmarks WHERE document_id=?1 ORDER BY created_at,id")?;
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
    pub fn relink(&self, id: &str, path: &Path) -> Result<DocumentRecord> {
        self.relink_sessions(id, path, &[])
            .map(|(record, _)| record)
    }
    /// Bind retained provisional sessions while the old path still identifies
    /// the document. Keep these aliases for queued retries in this worker.
    pub fn relink_sessions(
        &self,
        id: &str,
        path: &Path,
        sessions: &[DocumentRecord],
    ) -> Result<(DocumentRecord, Vec<String>)> {
        let transaction = self.connection.unchecked_transaction()?;
        let json: String =
            self.connection
                .query_row("SELECT payload FROM documents WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
        let mut record: DocumentRecord = serde_json::from_str(&json)?;
        let path = std::fs::canonicalize(path)?;
        if Format::from_path(&path)? != record.format {
            return Err(Error::Invalid(
                "Choose a document of the same format.".into(),
            ));
        }
        let sessions = sessions
            .iter()
            .filter(|session| session.id == id || session.path == record.path)
            .collect::<Vec<_>>();
        if let Some(session) = sessions.last() {
            record.locator = session.locator.clone();
            record.settings = session.settings.clone();
            record.revision = session.revision.clone();
            record.last_opened = session.last_opened;
        }
        record.path = path;
        self.save(&record)?;
        transaction.commit()?;
        let session_ids = sessions.iter().map(|session| session.id.clone()).collect();
        for session in sessions {
            self.session_ids
                .borrow_mut()
                .insert(session.id.clone(), id.to_owned());
        }
        Ok((record, session_ids))
    }
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
        let store = Store::open(&database).unwrap();
        let record = store.document(&source).unwrap();
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
        let mut record = store.document(&source).unwrap();
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
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.document(&source).unwrap().locator, record.locator);
        let moved = temp.path().join("renamed.md");
        std::fs::rename(source, &moved).unwrap();
        assert_eq!(reopened.relink(&record.id, &moved).unwrap().id, record.id);
    }
    #[test]
    fn temporary_lock_reconnects_identity_without_losing_session_progress() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("book.md");
        std::fs::write(&source, "# A book").unwrap();
        let path = temp.path().join("state.sqlite3");
        let store = Store::open(&path).unwrap();
        store
            .connection
            .busy_timeout(Duration::from_millis(20))
            .unwrap();
        let original = store.document(&source).unwrap();
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
        let recovered_id = store.save_session(&session).unwrap();
        assert_eq!(recovered_id, original.id);
        let recovered = store.document(&source).unwrap();
        assert_eq!(recovered.locator, session.locator);
        assert_eq!(recovered.settings, session.settings);
        assert_eq!(recovered.revision, session.revision);
        // A second save may already be queued with the provisional ID.
        assert_eq!(store.save_session(&session).unwrap(), original.id);
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
        let store = Store::open(&database).unwrap();
        store
            .connection
            .busy_timeout(Duration::from_millis(20))
            .unwrap();
        let mut snapshot = store.document(&source).unwrap();
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
        store.relink(&snapshot.id, &moved).unwrap();

        // The old path may even belong to another document by retry time.
        std::fs::write(&source, "# Another book").unwrap();
        let other = store.document(&source).unwrap();
        store.save(&other).unwrap();
        for _ in 0..2 {
            assert_eq!(store.save_session(&snapshot).unwrap(), snapshot.id);
            let recovered = store.document(&moved).unwrap();
            assert_eq!(recovered.id, snapshot.id);
            assert_eq!(recovered.path, std::fs::canonicalize(&moved).unwrap());
            assert_eq!(recovered.locator, snapshot.locator);
            assert_eq!(recovered.settings, snapshot.settings);
            assert_eq!(store.bookmarks(&recovered.id).unwrap().len(), 1);
            assert_eq!(store.document(&source).unwrap().id, other.id);
        }
    }

    #[test]
    fn provisional_retry_follows_relink_even_when_old_path_is_reused() {
        for reuse in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("book.md");
            let moved = temp.path().join("moved.md");
            std::fs::write(&source, "# A book").unwrap();
            let database = temp.path().join("state.sqlite3");
            let store = Store::open(&database).unwrap();
            store
                .connection
                .busy_timeout(Duration::from_millis(20))
                .unwrap();
            let original = store.document(&source).unwrap();
            store.save(&original).unwrap();
            let locked = Connection::open(&database).unwrap();
            locked.execute_batch("BEGIN EXCLUSIVE").unwrap();
            assert!(store.document(&source).is_err());
            let mut snapshot = DocumentRecord::new(source.clone()).unwrap();
            snapshot.settings.font_size = 25.0;
            snapshot.locator = Some(Locator {
                version: 1,
                anchor: Anchor::Pdf {
                    page: 5,
                    x: 0.0,
                    y: 0.0,
                    viewport_y: 0.0,
                },
            });
            assert!(store.save_session(&snapshot).is_err());
            locked.execute_batch("ROLLBACK").unwrap();
            std::fs::rename(&source, &moved).unwrap();
            store
                .add_bookmark(&Bookmark {
                    id: "original-bookmark".into(),
                    document_id: original.id.clone(),
                    label: "Saved passage".into(),
                    locator: snapshot.locator.clone().unwrap(),
                })
                .unwrap();
            // A rejected relocation must leave storage and retry identity alone.
            assert!(
                store
                    .relink_sessions(
                        &original.id,
                        &temp.path().join("missing.md"),
                        &[snapshot.clone()]
                    )
                    .is_err()
            );
            assert!(store.session_ids.borrow().is_empty());
            store
                .relink_sessions(&original.id, &moved, &[snapshot.clone()])
                .unwrap();
            assert_eq!(store.document(&moved).unwrap().locator, snapshot.locator);
            let other = if reuse {
                std::fs::write(&source, "# Another book").unwrap();
                let other = store.document(&source).unwrap();
                store.save(&other).unwrap();
                Some(other)
            } else {
                None
            };
            for _ in 0..2 {
                assert_eq!(store.save_session(&snapshot).unwrap(), original.id);
                let recovered = store.document(&moved).unwrap();
                assert_eq!(recovered.locator, snapshot.locator);
                assert_eq!(recovered.settings, snapshot.settings);
                assert_eq!(store.bookmarks(&recovered.id).unwrap().len(), 1);
                assert_eq!(recovered.path, moved);
                assert_eq!(store.recents().unwrap().len(), if reuse { 2 } else { 1 });
                if let Some(other) = &other {
                    let unchanged = store.document(&source).unwrap();
                    assert_eq!(unchanged.id, other.id);
                    assert_eq!(unchanged.locator, other.locator);
                    assert_eq!(unchanged.settings, other.settings);
                }
            }
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
