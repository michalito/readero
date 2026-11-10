use readero::{
    document::{Anchor, Bookmark, DocumentRecord, Locator},
    reading_state::ReadingState,
    state::Repair,
};
use rusqlite::Connection;
use std::path::PathBuf;

struct Reading {
    dir: tempfile::TempDir,
    database: PathBuf,
    state: ReadingState,
    record: DocumentRecord,
}
impl Reading {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("reading.md");
        std::fs::write(&source, "# Reading\n\nA passage to remember.").unwrap();
        let database = dir.path().join("reading.sqlite3");
        let state = ReadingState::start(database.clone());
        let record = state.document(source).wait().unwrap().record;
        Self {
            dir,
            database,
            state,
            record,
        }
    }
    fn fail_saves(&self) -> Connection {
        let connection = Connection::open(&self.database).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_save BEFORE INSERT ON documents
            BEGIN SELECT RAISE(FAIL, 'fixture cannot save'); END;",
            )
            .unwrap();
        connection
    }
    fn persisted(&self, path: PathBuf) -> DocumentRecord {
        ReadingState::start(self.database.clone())
            .document(path)
            .wait()
            .unwrap()
            .record
    }
}
fn passage(record: &mut DocumentRecord, section: usize) {
    record.locator = Some(Locator {
        version: 1,
        anchor: Anchor::Reflow {
            href: "content.xhtml".into(),
            cfi: format!("epubcfi(/6/2!/4/{})", section * 2 + 2),
            section,
            fraction: 0.4,
            quote: format!("Passage {section}"),
            block: format!("b{section}"),
            block_offset: Some(3),
            quote_offset: 0,
        },
    });
    record.settings.font_size = 20.0 + section as f64;
}
fn bookmark(record: &DocumentRecord, id: &str) -> Bookmark {
    Bookmark {
        id: id.into(),
        document_id: record.id.clone(),
        label: id.into(),
        locator: record.locator.clone().unwrap(),
    }
}

#[test]
fn saves_are_queued_before_open_even_when_replies_are_dropped_or_awaited_late() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    let old = reading.state.save(Some(reading.record.clone()));
    passage(&mut reading.record, 2);
    drop(reading.state.save(Some(reading.record.clone())));
    let opened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap();
    assert_eq!(opened.record.locator, reading.record.locator);
    old.wait().unwrap();
    assert!(!reading.state.status().pending);
    assert!(reading.state.status().error.is_none());
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        reading.record.locator
    );
}

#[test]
fn home_retry_keeps_the_latest_snapshot_of_every_failed_document() {
    let mut reading = Reading::new();
    let connection = reading.fail_saves();
    passage(&mut reading.record, 1);
    assert!(
        reading
            .state
            .save(Some(reading.record.clone()))
            .wait()
            .is_err()
    );
    passage(&mut reading.record, 2);
    assert!(
        reading
            .state
            .save(Some(reading.record.clone()))
            .wait()
            .is_err()
    );
    let other_path = reading.dir.path().join("other.md");
    std::fs::write(&other_path, "# Other").unwrap();
    let mut other = reading
        .state
        .document(other_path.clone())
        .wait()
        .unwrap()
        .record;
    passage(&mut other, 3);
    assert!(reading.state.save(Some(other.clone())).wait().is_err());
    assert!(reading.state.status().pending);
    assert!(reading.state.status().error.is_some());
    // Reopening during failure uses retained progress and appearance.
    let reopened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap()
        .record;
    assert_eq!(reopened.locator, reading.record.locator);
    assert_eq!(reopened.settings, reading.record.settings);
    connection.execute_batch("DROP TRIGGER fail_save").unwrap();
    reading.state.save(None).wait().unwrap();
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        reading.record.locator
    );
    assert_eq!(reading.persisted(other_path).locator, other.locator);
    assert!(!reading.state.status().pending);
    assert!(reading.state.status().error.is_none());
}

#[test]
fn a_late_failed_reply_does_not_replace_the_latest_successful_status() {
    let mut reading = Reading::new();
    let connection = reading.fail_saves();
    passage(&mut reading.record, 1);
    let failed = reading.state.save(Some(reading.record.clone()));
    // A read proves the earlier operation completed without consuming its reply.
    reading.state.recents().wait().unwrap();
    assert!(reading.state.status().error.is_some());
    connection.execute_batch("DROP TRIGGER fail_save").unwrap();
    passage(&mut reading.record, 2);
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    assert!(failed.wait().is_err());
    assert!(reading.state.status().error.is_none());
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        reading.record.locator
    );
}

#[test]
fn remove_recent_discards_provisional_retries_and_preserves_bookmarks_on_flush() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    reading
        .state
        .bookmark(reading.record.clone(), bookmark(&reading.record, "saved"))
        .wait()
        .unwrap();
    let connection = reading.fail_saves();
    let mut provisional = reading.record.clone();
    provisional.id = "provisional".into();
    passage(&mut provisional, 2);
    assert!(reading.state.save(Some(provisional)).wait().is_err());
    connection.execute_batch("DROP TRIGGER fail_save").unwrap();
    // Queue shutdown retry without waiting for the removal acknowledgement.
    let removed = reading
        .state
        .remove_recent(reading.record.id.clone(), reading.record.path.clone());
    reading.state.save(None).wait().unwrap();
    removed.wait().unwrap();
    assert!(reading.state.recents().wait().unwrap().is_empty());
    assert_eq!(
        reading
            .state
            .bookmarks(reading.record.id.clone())
            .wait()
            .unwrap()
            .len(),
        1
    );
    assert!(!reading.state.status().pending);
    assert!(reading.state.status().error.is_none());
}

#[test]
fn failed_recent_removal_retains_progress_for_later_retry() {
    let mut reading = Reading::new();
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    let connection = reading.fail_saves();
    passage(&mut reading.record, 2);
    assert!(
        reading
            .state
            .save(Some(reading.record.clone()))
            .wait()
            .is_err()
    );
    connection
        .execute_batch(
            "CREATE TRIGGER fail_hide BEFORE UPDATE OF hidden ON documents
        WHEN NEW.hidden=1 BEGIN SELECT RAISE(FAIL, 'fixture cannot remove'); END;",
        )
        .unwrap();
    assert!(
        reading
            .state
            .remove_recent(reading.record.id.clone(), reading.record.path.clone())
            .wait()
            .is_err()
    );
    connection
        .execute_batch("DROP TRIGGER fail_save; DROP TRIGGER fail_hide;")
        .unwrap();
    reading.state.save(None).wait().unwrap();
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        reading.record.locator
    );
    assert_eq!(reading.state.recents().wait().unwrap().len(), 1);
}

#[test]
fn locate_keeps_the_latest_provisional_state_and_old_path_reuse_cannot_steal_it() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    reading
        .state
        .bookmark(
            reading.record.clone(),
            bookmark(&reading.record, "original"),
        )
        .wait()
        .unwrap();
    let connection = reading.fail_saves();
    let mut provisional = reading.record.clone();
    provisional.id = "provisional".into();
    passage(&mut provisional, 2);
    assert!(
        reading
            .state
            .save(Some(provisional.clone()))
            .wait()
            .is_err()
    );
    let missing = reading.dir.path().join("missing.md");
    assert!(
        reading
            .state
            .locate(reading.record.id.clone(), missing)
            .wait()
            .is_err()
    );
    assert!(reading.state.status().pending);
    let moved = reading.dir.path().join("moved.md");
    std::fs::rename(&reading.record.path, &moved).unwrap();
    connection.execute_batch("DROP TRIGGER fail_save").unwrap();
    let located = reading
        .state
        .locate(reading.record.id.clone(), moved.clone())
        .wait()
        .unwrap();
    assert_eq!(located.locator, provisional.locator);
    assert_eq!(located.settings, provisional.settings);
    assert_eq!(located.id, reading.record.id);
    std::fs::write(&reading.record.path, "# A different document").unwrap();
    let other = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap()
        .record;
    reading.state.save(Some(other.clone())).wait().unwrap();
    assert_ne!(other.id, located.id);
    // A snapshot may have been captured before Locate's reply reached the shell.
    passage(&mut provisional, 3);
    reading
        .state
        .save(Some(provisional.clone()))
        .wait()
        .unwrap();
    let saved = reading.persisted(moved.clone());
    assert_eq!(saved.id, located.id);
    assert_eq!(saved.locator, provisional.locator);
    let untouched = reading.persisted(other.path);
    assert_eq!(untouched.id, other.id);
    assert!(untouched.locator.is_none());
    assert!(reading.state.reconcile(&mut provisional));
    assert_eq!(provisional.path, moved);
    assert_eq!(provisional.id, located.id);
    assert_eq!(reading.state.bookmarks(located.id).wait().unwrap().len(), 1);
}

#[test]
fn acknowledged_provisional_identity_follows_a_later_locate_without_pending_saves() {
    let mut reading = Reading::new();
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    let mut provisional = reading.record.clone();
    provisional.id = "provisional".into();
    reading
        .state
        .save(Some(provisional.clone()))
        .wait()
        .unwrap();
    assert!(
        reading
            .state
            .locate(
                reading.record.id.clone(),
                reading.dir.path().join("missing.md")
            )
            .wait()
            .is_err()
    );
    assert!(
        reading.state.status().error.is_none(),
        "a rejected Locate must not report a save failure"
    );
    let moved = reading.dir.path().join("moved.md");
    std::fs::rename(&reading.record.path, &moved).unwrap();
    reading
        .state
        .locate(reading.record.id.clone(), moved.clone())
        .wait()
        .unwrap();
    std::fs::write(&reading.record.path, "# Reused path").unwrap();
    passage(&mut provisional, 3);
    reading
        .state
        .bookmark(provisional.clone(), bookmark(&provisional, "after-move"))
        .wait()
        .unwrap();
    assert!(reading.state.reconcile(&mut provisional));
    assert_eq!(provisional.path, moved);
    assert_eq!(provisional.id, reading.record.id);
    assert_eq!(
        reading
            .state
            .bookmarks(reading.record.id.clone())
            .wait()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(reading.persisted(moved).locator, provisional.locator);
    passage(&mut reading.record, 4);
    assert!(reading.state.reconcile(&mut reading.record));
    assert_ne!(
        reading.record.locator, provisional.locator,
        "acknowledgement must not replace live progress"
    );
}

#[test]
fn locked_open_can_read_and_later_reconnect_without_losing_old_bookmarks() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    reading
        .state
        .bookmark(
            reading.record.clone(),
            bookmark(&reading.record, "original"),
        )
        .wait()
        .unwrap();
    let connection = Connection::open(&reading.database).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let opened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap();
    assert!(opened.storage_error.is_some());
    let mut provisional = opened.record;
    assert_ne!(provisional.id, reading.record.id);
    connection.execute_batch("ROLLBACK").unwrap();
    passage(&mut provisional, 2);
    reading
        .state
        .bookmark(provisional.clone(), bookmark(&provisional, "new"))
        .wait()
        .unwrap();
    assert!(reading.state.reconcile(&mut provisional));
    assert_eq!(provisional.id, reading.record.id);
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        provisional.locator
    );
    assert_eq!(
        reading
            .state
            .bookmarks(provisional.id)
            .wait()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn an_unavailable_database_reopens_and_flushes_a_retained_session() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("not-yet-a-directory");
    std::fs::write(&parent, "blocked").unwrap();
    let source = dir.path().join("reading.md");
    std::fs::write(&source, "# Reading").unwrap();
    let state = ReadingState::start(parent.join("reading.sqlite3"));
    assert!(state.save(None).wait().is_err());
    assert!(state.status().error.is_some());
    let opened = state.document(source.clone()).wait().unwrap();
    assert!(opened.storage_error.is_some());
    let mut record = opened.record;
    passage(&mut record, 2);
    assert!(state.save(Some(record.clone())).wait().is_err());
    std::fs::remove_file(&parent).unwrap();
    state.save(None).wait().unwrap();
    assert_eq!(
        state.document(source).wait().unwrap().record.locator,
        record.locator
    );
    assert!(!state.status().pending);
}

#[test]
fn failed_shutdown_flush_keeps_pending_documents_and_reports_partial_success() {
    let mut reading = Reading::new();
    let other_path = reading.dir.path().join("other.md");
    std::fs::write(&other_path, "# Other").unwrap();
    let mut other = reading
        .state
        .document(other_path.clone())
        .wait()
        .unwrap()
        .record;
    let connection = reading.fail_saves();
    passage(&mut reading.record, 1);
    passage(&mut other, 2);
    assert!(
        reading
            .state
            .save(Some(reading.record.clone()))
            .wait()
            .is_err()
    );
    assert!(reading.state.save(Some(other.clone())).wait().is_err());
    connection.execute_batch("DROP TRIGGER fail_save;").unwrap();
    connection
        .execute(
            "CREATE TRIGGER fail_one BEFORE INSERT ON documents WHEN NEW.path LIKE '%/other.md'
        BEGIN SELECT RAISE(FAIL, 'fixture cannot save this document'); END;",
            [],
        )
        .unwrap();
    // Fail just the second document while the first can be committed.
    assert!(reading.state.save(Some(other)).wait().is_err());
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        reading.record.locator
    );
    assert!(reading.state.status().pending);
    connection.execute_batch("DROP TRIGGER fail_one").unwrap();
    reading.state.save(None).wait().unwrap();
    assert!(reading.persisted(other_path).locator.is_some());
    assert!(!reading.state.status().pending);
}

#[test]
fn cancelled_open_does_not_enter_recents_and_failed_bookmark_is_not_retried() {
    let mut reading = Reading::new();
    assert!(reading.state.recents().wait().unwrap().is_empty());
    passage(&mut reading.record, 1);
    let connection = reading.fail_saves();
    assert!(
        reading
            .state
            .bookmark(reading.record.clone(), bookmark(&reading.record, "failed"))
            .wait()
            .is_err()
    );
    connection.execute_batch("DROP TRIGGER fail_save").unwrap();
    reading.state.save(None).wait().unwrap();
    assert!(
        reading
            .state
            .bookmarks(reading.record.id.clone())
            .wait()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        reading.record.locator
    );
}

#[test]
fn bookmarks_keep_their_creation_order_within_the_same_second() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    for id in ["zeta", "alpha", "mu"] {
        reading
            .state
            .bookmark(reading.record.clone(), bookmark(&reading.record, id))
            .wait()
            .unwrap();
    }
    // Creation times have one-second resolution. Make them equal explicitly.
    Connection::open(&reading.database)
        .unwrap()
        .execute("UPDATE bookmarks SET created_at=0", [])
        .unwrap();
    let ids: Vec<_> = reading
        .state
        .bookmarks(reading.record.id.clone())
        .wait()
        .unwrap()
        .into_iter()
        .map(|bookmark| bookmark.id)
        .collect();
    assert_eq!(ids, ["zeta", "alpha", "mu"]);
}

#[cfg(unix)]
#[test]
fn a_locked_open_through_a_symlink_reconnects_to_the_same_document() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    reading
        .state
        .bookmark(
            reading.record.clone(),
            bookmark(&reading.record, "original"),
        )
        .wait()
        .unwrap();
    let alias = reading.dir.path().join("alias");
    std::os::unix::fs::symlink(reading.record.path.parent().unwrap(), &alias).unwrap();
    let lock = Connection::open(&reading.database).unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let opened = reading
        .state
        .document(alias.join("reading.md"))
        .wait()
        .unwrap();
    assert!(opened.storage_error.is_some());
    let mut provisional = opened.record;
    assert_eq!(provisional.path, reading.record.path);
    lock.execute_batch("ROLLBACK").unwrap();
    passage(&mut provisional, 2);
    reading
        .state
        .save(Some(provisional.clone()))
        .wait()
        .unwrap();
    assert_eq!(reading.state.recents().wait().unwrap().len(), 1);
    assert!(reading.state.reconcile(&mut provisional));
    assert_eq!(provisional.id, reading.record.id);
    assert_eq!(
        reading.persisted(reading.record.path.clone()).locator,
        provisional.locator
    );
}

#[test]
fn a_retained_provisional_session_follows_locate_even_when_the_old_path_is_reused() {
    for reuse in [false, true] {
        let mut reading = Reading::new();
        passage(&mut reading.record, 1);
        reading
            .state
            .bookmark(
                reading.record.clone(),
                bookmark(&reading.record, "original"),
            )
            .wait()
            .unwrap();
        let old_path = reading.record.path.clone();
        let mut provisional = DocumentRecord::new(old_path.clone()).unwrap();
        passage(&mut provisional, 2);
        let connection = reading.fail_saves();
        assert!(
            reading
                .state
                .save(Some(provisional.clone()))
                .wait()
                .is_err()
        );
        let moved = reading.dir.path().join("moved.md");
        std::fs::rename(&old_path, &moved).unwrap();
        connection.execute_batch("DROP TRIGGER fail_save").unwrap();
        // A rejected Locate leaves the retained session unbound.
        assert!(
            reading
                .state
                .locate(
                    reading.record.id.clone(),
                    reading.dir.path().join("missing.md")
                )
                .wait()
                .is_err()
        );
        assert!(!reading.state.reconcile(&mut provisional.clone()));
        let located = reading
            .state
            .locate(reading.record.id.clone(), moved.clone())
            .wait()
            .unwrap();
        assert_eq!(located.locator, provisional.locator);
        let other = reuse.then(|| {
            std::fs::write(&old_path, "# Another document").unwrap();
            let other = reading
                .state
                .document(old_path.clone())
                .wait()
                .unwrap()
                .record;
            reading.state.save(Some(other.clone())).wait().unwrap();
            other
        });
        for _ in 0..2 {
            reading
                .state
                .save(Some(provisional.clone()))
                .wait()
                .unwrap();
            let recovered = reading.persisted(moved.clone());
            assert_eq!(recovered.id, reading.record.id);
            assert_eq!(recovered.locator, provisional.locator);
            assert_eq!(recovered.settings, provisional.settings);
            assert_eq!(
                reading.state.bookmarks(recovered.id).wait().unwrap().len(),
                1
            );
            assert_eq!(
                reading.state.recents().wait().unwrap().len(),
                if reuse { 2 } else { 1 }
            );
            if let Some(other) = &other {
                let unchanged = reading.persisted(old_path.clone());
                assert_eq!(unchanged.id, other.id);
                assert!(unchanged.locator.is_none());
            }
        }
    }
}

#[test]
fn an_unreadable_saved_position_opens_at_the_start_and_keeps_identity() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    reading
        .state
        .bookmark(reading.record.clone(), bookmark(&reading.record, "kept"))
        .wait()
        .unwrap();
    let db = Connection::open(&reading.database).unwrap();
    db.execute(
        "UPDATE documents SET payload=json_set(payload, '$.locator.fraction', 7)",
        [],
    )
    .unwrap();
    let opened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap();
    assert!(opened.storage_error.is_none());
    assert!(matches!(opened.repair, Some(Repair::Position(_))));
    assert_eq!(opened.record.id, reading.record.id);
    assert!(opened.record.locator.is_none());
    assert_eq!(opened.record.settings, reading.record.settings);

    db.execute("UPDATE documents SET payload='not JSON'", [])
        .unwrap();
    let opened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap();
    assert!(opened.storage_error.is_none());
    assert!(matches!(opened.repair, Some(Repair::Record(_))));
    assert_eq!(opened.record.id, reading.record.id);
    assert_eq!(opened.record.path, reading.record.path);
    assert!(opened.record.locator.is_none());
    // One unreadable row neither blanks Home nor detaches bookmarks.
    let recents = reading.state.recents().wait().unwrap();
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].id, reading.record.id);
    assert_eq!(
        reading
            .state
            .bookmarks(reading.record.id.clone())
            .wait()
            .unwrap()
            .len(),
        1
    );
    // The next save replaces the unreadable row.
    let mut record = opened.record;
    passage(&mut record, 2);
    reading.state.save(Some(record.clone())).wait().unwrap();
    assert!(reading.state.status().error.is_none());
    let reopened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap();
    assert!(reopened.repair.is_none());
    assert_eq!(reopened.record.locator, record.locator);
}

#[test]
fn a_retained_snapshot_supersedes_an_unreadable_saved_position() {
    let mut reading = Reading::new();
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    let connection = reading.fail_saves();
    passage(&mut reading.record, 2);
    assert!(
        reading
            .state
            .save(Some(reading.record.clone()))
            .wait()
            .is_err()
    );
    connection
        .execute("UPDATE documents SET payload='not JSON'", [])
        .unwrap();
    let opened = reading
        .state
        .document(reading.record.path.clone())
        .wait()
        .unwrap();
    assert!(opened.repair.is_none());
    assert_eq!(opened.record.locator, reading.record.locator);
    assert_eq!(opened.record.settings, reading.record.settings);
}

/// The reader moves the source, then opens it from its new place, which
/// creates a separate document because nothing connects the two yet.
fn open_moved_copy(reading: &Reading, read: bool) -> (PathBuf, DocumentRecord) {
    let moved = reading.dir.path().join("moved.md");
    std::fs::rename(&reading.record.path, &moved).unwrap();
    let mut duplicate = reading.state.document(moved.clone()).wait().unwrap().record;
    assert_ne!(duplicate.id, reading.record.id);
    duplicate.last_opened = reading.record.last_opened + 1;
    if read {
        passage(&mut duplicate, 3);
    }
    reading.state.save(Some(duplicate.clone())).wait().unwrap();
    (duplicate.path.clone(), duplicate)
}

#[test]
fn locate_absorbs_a_duplicate_opened_at_the_new_place() {
    let mut reading = Reading::new();
    reading.record.last_opened = 1_000;
    passage(&mut reading.record, 1);
    reading
        .state
        .bookmark(
            reading.record.clone(),
            bookmark(&reading.record, "original"),
        )
        .wait()
        .unwrap();
    let (moved, duplicate) = open_moved_copy(&reading, true);
    let located = reading
        .state
        .locate(reading.record.id.clone(), moved.clone())
        .wait()
        .unwrap();
    assert_eq!(located.id, reading.record.id);
    assert_eq!(located.path, moved);
    // The duplicate was read after the move, so its position is the newest.
    assert_eq!(located.locator, duplicate.locator);
    assert_eq!(located.settings, duplicate.settings);
    assert_eq!(reading.state.recents().wait().unwrap().len(), 1);
    assert_eq!(reading.persisted(moved).id, reading.record.id);
    // A session that still holds the duplicate follows the located document.
    assert_eq!(
        reading
            .state
            .bookmarks(duplicate.id.clone())
            .wait()
            .unwrap()
            .len(),
        1
    );
    let mut live = duplicate.clone();
    assert!(reading.state.reconcile(&mut live));
    assert_eq!(live.id, reading.record.id);
    assert!(reading.state.status().error.is_none());
}

#[test]
fn locate_keeps_its_own_position_over_an_unread_duplicate() {
    let mut reading = Reading::new();
    reading.record.last_opened = 1_000;
    passage(&mut reading.record, 1);
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    let (moved, _) = open_moved_copy(&reading, false);
    let located = reading
        .state
        .locate(reading.record.id.clone(), moved)
        .wait()
        .unwrap();
    assert_eq!(located.locator, reading.record.locator);
    assert_eq!(located.settings, reading.record.settings);
}

#[test]
fn locate_refuses_to_combine_a_duplicate_that_has_bookmarks() {
    let mut reading = Reading::new();
    passage(&mut reading.record, 1);
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    let (moved, duplicate) = open_moved_copy(&reading, true);
    reading
        .state
        .bookmark(duplicate.clone(), bookmark(&duplicate, "duplicate"))
        .wait()
        .unwrap();
    let error = reading
        .state
        .locate(reading.record.id.clone(), moved.clone())
        .wait()
        .unwrap_err()
        .to_string();
    assert!(error.contains("separate document"), "{error}");
    assert!(!error.contains("UNIQUE"), "{error}");
    assert!(reading.state.status().error.is_none());
    assert_eq!(reading.persisted(moved).id, duplicate.id);
    assert_eq!(reading.state.recents().wait().unwrap().len(), 2);
}

#[test]
fn locate_prefers_a_newer_duplicate_over_an_older_retained_snapshot() {
    let mut reading = Reading::new();
    reading.record.last_opened = 1_000;
    reading
        .state
        .save(Some(reading.record.clone()))
        .wait()
        .unwrap();
    let connection = Connection::open(&reading.database).unwrap();
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER fail_original BEFORE INSERT ON documents WHEN NEW.id='{}'
            BEGIN SELECT RAISE(FAIL, 'fixture cannot save the original'); END;",
            reading.record.id
        ))
        .unwrap();
    passage(&mut reading.record, 1);
    assert!(
        reading
            .state
            .save(Some(reading.record.clone()))
            .wait()
            .is_err()
    );
    let moved = reading.dir.path().join("moved.md");
    std::fs::rename(&reading.record.path, &moved).unwrap();
    let mut duplicate = reading.state.document(moved.clone()).wait().unwrap().record;
    duplicate.last_opened = 2_000;
    passage(&mut duplicate, 3);
    // The duplicate is saved; the retry for the original still fails.
    assert!(reading.state.save(Some(duplicate.clone())).wait().is_err());
    assert_eq!(reading.persisted(moved.clone()).id, duplicate.id);
    connection
        .execute_batch("DROP TRIGGER fail_original")
        .unwrap();
    let located = reading
        .state
        .locate(reading.record.id.clone(), moved.clone())
        .wait()
        .unwrap();
    assert_eq!(located.locator, duplicate.locator);
    // The superseded snapshot is not retried over the newer position.
    reading.state.save(None).wait().unwrap();
    assert!(!reading.state.status().pending);
    assert_eq!(reading.persisted(moved).locator, duplicate.locator);
}
