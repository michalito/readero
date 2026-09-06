use readero::{document::*, resource::Publication, state::Store};
use std::io::Write;

#[test]
fn malformed_and_escaping_archives_fail_without_extracting_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bad.epub");
    std::fs::write(&path, b"not a zip file").unwrap();
    assert!(Publication::open(&path, Format::Epub).is_err());
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("../outside.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"untrusted").unwrap();
    zip.finish().unwrap();
    assert!(Publication::open(&path, Format::Epub).is_err());
    assert!(!temp.path().parent().unwrap().join("outside.txt").exists());
}

#[test]
fn oversized_expansion_is_rejected_before_a_resource_is_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("oversized.epub");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file(
        "META-INF/container.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<container/>").unwrap();
    zip.start_file(
        "large.xhtml",
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )
    .unwrap();
    let block = vec![b'a'; 1024 * 1024];
    for _ in 0..65 {
        zip.write_all(&block).unwrap();
    }
    zip.finish().unwrap();
    assert!(Publication::open(&path, Format::Epub).is_err());
}

#[test]
fn a_failed_or_cancelled_open_does_not_enter_recents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reading.MD");
    std::fs::write(&path, "# Text").unwrap();
    let store = Store::open(&temp.path().join("reading.sqlite3")).unwrap();
    let record = store.document(&path).unwrap();
    assert_eq!(record.format, Format::Markdown);
    assert!(store.recents().unwrap().is_empty());
    store.save(&record).unwrap();
    assert_eq!(store.recents().unwrap().len(), 1);
}

#[test]
fn failing_transaction_keeps_the_previous_saved_location() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("reading.pdf");
    std::fs::write(&source, b"fixture").unwrap();
    let path = temp.path().join("reading.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut record = store.document(&source).unwrap();
    record.locator = Some(Locator {
        version: 1,
        anchor: Anchor::Pdf {
            page: 4,
            x: 10.0,
            y: 200.0,
            viewport_y: 24.0,
        },
    });
    store.save(&record).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_save BEFORE UPDATE ON documents BEGIN SELECT RAISE(ABORT,'simulated storage failure'); END;").unwrap();
    record.locator = Some(Locator {
        version: 1,
        anchor: Anchor::Pdf {
            page: 9,
            x: 10.0,
            y: 200.0,
            viewport_y: 24.0,
        },
    });
    assert!(store.save(&record).is_err());
    assert!(matches!(
        store.document(&source).unwrap().locator.unwrap().anchor,
        Anchor::Pdf { page: 4, .. }
    ));
}

#[test]
fn corrupt_database_is_not_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reading.sqlite3");
    let data = b"corrupted data kept for recovery";
    std::fs::write(&path, data).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), data);
}
