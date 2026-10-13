use readero::{
    document::{Anchor, Bookmark, DocumentRecord, Locator},
    notebook::{Notebook, Reference},
    reading_state::ReadingState,
    state::Store,
};
use rusqlite::{Connection, params};
use std::path::PathBuf;

struct Fixture {
    dir: tempfile::TempDir,
    database: PathBuf,
    state: ReadingState,
    record: DocumentRecord,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.md");
        std::fs::write(&path, "# A book").unwrap();
        let database = dir.path().join("state.sqlite3");
        let state = ReadingState::start(database.clone());
        let record = state.document(path).wait().unwrap().record;
        Self {
            dir,
            database,
            state,
            record,
        }
    }
    fn saved(&self) -> Notebook {
        ReadingState::start(self.database.clone())
            .notebook(self.record.clone())
            .wait()
            .unwrap()
    }
    fn reject_notebooks(&self) -> Connection {
        let db = Connection::open(&self.database).unwrap();
        db.execute_batch(
            "CREATE TRIGGER reject_notebook BEFORE INSERT ON notebooks
            BEGIN SELECT RAISE(FAIL, 'disk cannot save notebook'); END;",
        )
        .unwrap();
        db
    }
}
fn draft() -> Notebook {
    Notebook {
        text: "Question → café 👩‍🔬\n\n  x = 2;\nAn example.\n".into(),
        cursor: 19,
        selection: 4,
        top: 6,
        visible: true,
        width: 380,
        height: 310,
        references: vec![Reference {
            passage: None,
            id: "example-1".into(),
            label: "Worked example 1".into(),
            locator: Locator {
                version: 1,
                anchor: Anchor::Pdf {
                    page: 3,
                    x: 12.0,
                    y: 42.0,
                    viewport_y: 0.25,
                },
            },
        }],
        ..Notebook::default()
    }
}

#[test]
fn notebook_round_trip_keeps_unicode_references_and_editing_context() {
    let f = Fixture::new();
    assert_eq!(
        f.state.notebook(f.record.clone()).wait().unwrap(),
        Notebook::default()
    );
    f.state
        .save_notebook(f.record.clone(), draft())
        .wait()
        .unwrap();
    assert_eq!(f.saved(), draft());
    assert_eq!(std::fs::read_to_string(&f.record.path).unwrap(), "# A book");
}

#[test]
fn documents_are_isolated_and_reading_updates_do_not_rewrite_notebooks() {
    let mut f = Fixture::new();
    f.state
        .save_notebook(f.record.clone(), draft())
        .wait()
        .unwrap();
    let other_path = f.dir.path().join("other.md");
    std::fs::write(&other_path, "# Other").unwrap();
    let other = f.state.document(other_path).wait().unwrap().record;
    assert_eq!(
        f.state.notebook(other.clone()).wait().unwrap(),
        Notebook::default()
    );
    f.state
        .save_notebook(
            other.clone(),
            Notebook {
                text: "Other notebook".into(),
                ..Notebook::default()
            },
        )
        .wait()
        .unwrap();
    f.record.settings.font_size = 28.0;
    f.state.save(Some(f.record.clone())).wait().unwrap();
    assert_eq!(f.saved(), draft());
    assert_eq!(
        f.state.notebook(other).wait().unwrap().text,
        "Other notebook"
    );
}

#[test]
fn failed_writes_keep_the_latest_draft_and_retry_without_duplicate_references() {
    let f = Fixture::new();
    let db = f.reject_notebooks();
    let first = f.state.save_notebook(f.record.clone(), draft());
    let latest = Notebook {
        text: "Latest draft with enough characters for cursor".into(),
        ..draft()
    };
    let second = f.state.save_notebook(f.record.clone(), latest.clone());
    assert!(second.wait().is_err());
    assert_eq!(f.state.notebook(f.record.clone()).wait().unwrap(), latest);
    assert!(f.state.status().pending);
    db.execute_batch("DROP TRIGGER reject_notebook").unwrap();
    f.state.save(None).wait().unwrap();
    assert!(first.wait().is_err()); // Delayed failure cannot overwrite current status.
    assert!(!f.state.status().pending);
    assert!(f.state.status().error.is_none());
    assert_eq!(f.saved(), latest);
}

#[test]
fn queued_saves_survive_dropped_replies_and_latest_edit_wins() {
    let f = Fixture::new();
    drop(f.state.save_notebook(f.record.clone(), draft()));
    let latest = Notebook {
        text: "Newest".into(),
        ..Notebook::default()
    };
    drop(f.state.save_notebook(f.record.clone(), latest.clone()));
    f.state.save(None).wait().unwrap();
    assert_eq!(f.saved(), latest);
}

#[test]
fn recents_removal_preserves_unsaved_notebook_without_resurrecting_the_document() {
    let f = Fixture::new();
    f.state.save(Some(f.record.clone())).wait().unwrap();
    let db = f.reject_notebooks();
    assert!(
        f.state
            .save_notebook(f.record.clone(), draft())
            .wait()
            .is_err()
    );
    f.state
        .remove_recent(f.record.id.clone(), f.record.path.clone())
        .wait()
        .unwrap();
    assert!(f.state.status().pending);
    db.execute_batch("DROP TRIGGER reject_notebook").unwrap();
    f.state.save(None).wait().unwrap();
    assert!(f.state.recents().wait().unwrap().is_empty());
    assert_eq!(f.saved(), draft());
}

#[test]
fn locate_reconciles_a_provisional_notebook_without_overwriting_reading_settings() {
    let mut f = Fixture::new();
    f.record.settings.font_size = 28.0;
    f.state.save(Some(f.record.clone())).wait().unwrap();
    let provisional = DocumentRecord::new(f.record.path.clone()).unwrap();
    let db = f.reject_notebooks();
    assert!(
        f.state
            .save_notebook(provisional.clone(), draft())
            .wait()
            .is_err()
    );
    let moved = f.dir.path().join("moved.md");
    std::fs::rename(&f.record.path, &moved).unwrap();
    f.state
        .locate(f.record.id.clone(), moved.clone())
        .wait()
        .unwrap();
    std::fs::write(&f.record.path, "# Unrelated replacement").unwrap();
    let unrelated = f
        .state
        .document(f.record.path.clone())
        .wait()
        .unwrap()
        .record;
    assert_eq!(
        f.state.notebook(unrelated.clone()).wait().unwrap(),
        Notebook::default()
    );
    let other_draft = Notebook {
        text: "Unrelated notebook".into(),
        ..Notebook::default()
    };
    assert!(
        f.state
            .save_notebook(unrelated.clone(), other_draft.clone())
            .wait()
            .is_err()
    );
    assert!(f.state.save(Some(unrelated.clone())).wait().is_err());
    db.execute_batch("DROP TRIGGER reject_notebook").unwrap();
    f.state.save(None).wait().unwrap();
    let relocated = f.state.document(moved).wait().unwrap().record;
    assert_eq!(relocated.id, f.record.id);
    assert_eq!(relocated.settings.font_size, 28.0);
    assert_eq!(f.state.notebook(relocated).wait().unwrap(), draft());
    assert_eq!(f.state.notebook(unrelated).wait().unwrap(), other_draft);
    assert_eq!(f.state.notebook(provisional).wait().unwrap(), draft());
}

#[test]
fn failure_for_one_notebook_does_not_discard_or_block_success_for_another() {
    let f = Fixture::new();
    let other_path = f.dir.path().join("other.md");
    std::fs::write(&other_path, "# Other").unwrap();
    let other = f.state.document(other_path).wait().unwrap().record;
    let db = Connection::open(&f.database).unwrap();
    db.execute_batch(&format!(
        "CREATE TRIGGER reject_one BEFORE INSERT ON notebooks
        WHEN NEW.document_id='{}' BEGIN SELECT RAISE(FAIL, 'one fails'); END;",
        f.record.id
    ))
    .unwrap();
    assert!(
        f.state
            .save_notebook(f.record.clone(), draft())
            .wait()
            .is_err()
    );
    assert!(
        f.state
            .save_notebook(other.clone(), draft())
            .wait()
            .is_err()
    );
    assert_eq!(
        Store::open(&f.database).unwrap().notebook(&other).unwrap(),
        draft()
    );
    assert_eq!(f.state.notebook(f.record.clone()).wait().unwrap(), draft());
    db.execute_batch("DROP TRIGGER reject_one").unwrap();
    f.state.save(None).wait().unwrap();
    assert_eq!(f.saved(), draft());
}

#[test]
fn invalid_or_newer_payload_cannot_be_loaded_or_overwritten() {
    let f = Fixture::new();
    f.state
        .save_notebook(f.record.clone(), draft())
        .wait()
        .unwrap();
    let db = Connection::open(&f.database).unwrap();
    for json in [
        "not JSON".to_owned(),
        serde_json::to_string(&Notebook {
            version: 20,
            ..draft()
        })
        .unwrap(),
    ] {
        let state = ReadingState::start(f.database.clone());
        db.execute("UPDATE notebooks SET payload=?1", [&json])
            .unwrap();
        assert!(state.notebook(f.record.clone()).wait().is_err());
        assert!(
            state
                .save_notebook(f.record.clone(), draft())
                .wait()
                .is_err()
        );
        assert_eq!(
            db.query_row::<String, _, _>("SELECT payload FROM notebooks", [], |r| r.get(0))
                .unwrap(),
            json
        );
        // A fresh worker has no draft retained from the intentional failed write.
        assert!(
            ReadingState::start(f.database.clone())
                .notebook(f.record.clone())
                .wait()
                .is_err()
        );
    }
}

#[test]
fn long_notebook_round_trips_without_truncation() {
    let f = Fixture::new();
    let text = "Question → café 👩‍🔬 and a worked example.\n".repeat(20_000);
    let notebook = Notebook {
        cursor: text.chars().count() as i32,
        text,
        ..Notebook::default()
    };
    f.state
        .save_notebook(f.record.clone(), notebook.clone())
        .wait()
        .unwrap();
    assert_eq!(f.saved(), notebook);
}

#[test]
fn invalid_context_or_reference_is_rejected_before_replacing_saved_work() {
    let f = Fixture::new();
    f.state
        .save_notebook(f.record.clone(), draft())
        .wait()
        .unwrap();
    for invalid in [
        Notebook {
            cursor: i32::MAX,
            ..draft()
        },
        Notebook {
            selection: -1,
            ..draft()
        },
        Notebook {
            top: i32::MAX,
            ..draft()
        },
        Notebook {
            text: "\0".into(),
            ..Notebook::default()
        },
        Notebook {
            references: vec![draft().references[0].clone(); 2],
            ..draft()
        },
    ] {
        assert!(
            f.state
                .save_notebook(f.record.clone(), invalid)
                .wait()
                .is_err()
        );
        assert_eq!(f.saved(), draft());
    }
}

#[test]
fn v1_migration_preserves_identity_bookmarks_and_reading_state() {
    let f = Fixture::new();
    f.state.save(Some(f.record.clone())).wait().unwrap();
    f.state
        .bookmark(
            f.record.clone(),
            Bookmark {
                id: "saved".into(),
                document_id: f.record.id.clone(),
                label: "Passage".into(),
                locator: draft().references[0].locator.clone(),
            },
        )
        .wait()
        .unwrap();
    let db = Connection::open(&f.database).unwrap();
    db.execute_batch("DROP TABLE notebooks; PRAGMA user_version=1;")
        .unwrap();
    let migrated = Store::open(&f.database).unwrap();
    assert_eq!(migrated.document(&f.record.path).unwrap().id, f.record.id);
    assert_eq!(migrated.bookmarks(&f.record.id).unwrap().len(), 1);
    assert_eq!(migrated.notebook(&f.record).unwrap(), Notebook::default());
    assert_eq!(
        db.pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        3
    );
}

#[test]
fn failed_migration_rolls_back_without_modifying_old_tables() {
    let f = Fixture::new();
    f.state.save(Some(f.record.clone())).wait().unwrap();
    let db = Connection::open(&f.database).unwrap();
    db.execute_batch("PRAGMA user_version=1;").unwrap(); // Conflicting table makes migration fail.
    let before: String = db
        .query_row(
            "SELECT payload FROM documents WHERE id=?1",
            [&f.record.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Store::open(&f.database).is_err());
    assert_eq!(
        db.pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT payload FROM documents WHERE id=?1",
            params![f.record.id],
            |r| r.get(0)
        )
        .unwrap(),
        before
    );
}
