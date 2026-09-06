//! A single-owner database. The desktop layer runs this on its state worker.
use crate::document::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::Path, time::Duration};

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
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
        Ok(Self { connection })
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
    /// comes from storage; the reader's current progress and settings win.
    pub fn save_session(&self, record: &DocumentRecord) -> Result<String> {
        let transaction = self.connection.unchecked_transaction()?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM documents WHERE path=?1
                    AND NOT EXISTS (SELECT 1 FROM documents WHERE id=?2)",
                params![record.path.to_string_lossy(), record.id],
                |row| row.get(0),
            )
            .optional()?;
        let mut record = record.clone();
        if let Some(id) = existing {
            record.id = id;
        }
        self.save(&record)?;
        transaction.commit()?;
        Ok(record.id)
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
        record.path = path;
        self.save(&record)?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
