use readero::{document::*, resource::Publication, state::Store};
use std::io::Write;

#[test]
fn epub_literal_percent_filenames_open_and_read_without_double_decoding() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("literal-percent.epub");
    let resources = [
        ("Text/100%FF.xhtml", "Text/100%25FF.xhtml"),
        ("Text/literal%00.xhtml", "Text/literal%2500.xhtml"),
        ("%2e%2e/chapter.xhtml", "%252e%252e/chapter.xhtml"),
        ("Text/with%20space.xhtml", "Text/with%2520space.xhtml"),
        ("Text/with space.xhtml", "Text/with%20space.xhtml"),
        ("Text/café.xhtml", "Text/caf%C3%A9.xhtml"),
    ];
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file(
        "META-INF/container.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<container/>").unwrap();
    for (name, _) in resources {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(name.as_bytes()).unwrap();
    }
    zip.finish().unwrap();

    let mut publication = Publication::open(&path, Format::Epub).unwrap();
    for (name, request) in resources {
        assert!(publication.entries.iter().any(|entry| entry.name == name));
        assert_eq!(publication.read(request).unwrap(), name.as_bytes());
    }
    for request in [
        "../outside.txt",
        "%2e%2e/chapter.xhtml",
        "Text/literal%00.xhtml",
    ] {
        assert!(publication.read(request).is_err(), "{request}");
    }
}

#[test]
fn malformed_and_escaping_archives_fail_without_extracting_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bad.epub");
    std::fs::write(&path, b"not a zip file").unwrap();
    assert!(Publication::open(&path, Format::Epub).is_err());
    for name in ["../outside.txt", "/outside.txt", "Text\\outside.txt"] {
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "META-INF/container.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"<container/>").unwrap();
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"untrusted").unwrap();
        zip.finish().unwrap();
        assert!(Publication::open(&path, Format::Epub).is_err(), "{name}");
    }
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

/// Rewrites the uncompressed size an archive declares for one entry.
fn declare_size(path: &std::path::Path, name: &str, size: u32) {
    let mut bytes = std::fs::read(path).unwrap();
    // Central directory and local headers: signature, size and name offsets.
    for (signature, size_at, name_at) in [(b"PK\x01\x02", 24, 46), (b"PK\x03\x04", 22, 30)] {
        let headers: Vec<usize> = bytes
            .windows(4)
            .enumerate()
            .filter_map(|(at, window)| (window == signature).then_some(at))
            .collect();
        for header in headers {
            if bytes.get(header + name_at..header + name_at + name.len()) == Some(name.as_bytes()) {
                bytes[header + size_at..header + size_at + 4].copy_from_slice(&size.to_le_bytes());
            }
        }
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn an_understated_entry_size_cannot_exceed_the_read_limit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("understated.epub");
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
    // The archive's own headers are untrusted: they can claim any size.
    declare_size(&path, "large.xhtml", 1000);

    let mut publication = Publication::open(&path, Format::Epub).unwrap();
    assert!(
        publication
            .entries
            .iter()
            .any(|entry| entry.name == "large.xhtml" && entry.size == 1000)
    );
    let error = publication.read("large.xhtml").unwrap_err();
    assert!(error.to_string().contains("too large"), "{error}");
    assert_eq!(
        publication.read("META-INF/container.xml").unwrap(),
        b"<container/>"
    );
}

#[test]
fn markdown_lists_its_generated_resources_and_reads_sidecars_beside_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("notes.md");
    std::fs::write(&source, "# Notes\n\n![figure](figures/one.png)\n").unwrap();
    std::fs::create_dir(temp.path().join("figures")).unwrap();
    std::fs::write(temp.path().join("figures/one.png"), b"image bytes").unwrap();

    let mut publication = Publication::open(&source, Format::Markdown).unwrap();
    let names: Vec<_> = publication
        .entries
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "META-INF/container.xml",
            "package.opf",
            "content.xhtml",
            "nav.xhtml"
        ]
    );
    for entry in publication.entries.clone() {
        assert_eq!(
            publication.read(&entry.name).unwrap().len() as u64,
            entry.size
        );
    }
    assert_eq!(publication.read("figures/one.png").unwrap(), b"image bytes");
    assert_eq!(
        publication.read("figures%2Fone.png").unwrap(),
        b"image bytes"
    );
    assert!(publication.read("figures/missing.png").is_err());
}

#[test]
fn a_sidecar_that_is_not_a_regular_file_is_refused_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("notes.md");
    std::fs::write(&source, "# Notes").unwrap();
    std::fs::create_dir(temp.path().join("folder")).unwrap();
    let pipe = temp.path().join("pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&pipe).status();
    assert!(
        made.is_ok_and(|status| status.success()),
        "mkfifo is required"
    );

    let mut publication = Publication::open(&source, Format::Markdown).unwrap();
    assert!(publication.read("folder").is_err());
    // Opening a pipe for reading waits for a writer that never comes.
    let (done, result) = std::sync::mpsc::channel();
    std::thread::spawn(move || done.send(publication.read("pipe.png").is_err()));
    assert_eq!(
        result.recv_timeout(std::time::Duration::from_secs(10)),
        Ok(true)
    );
}

#[test]
fn an_unreadable_markdown_source_is_not_reported_as_an_encoding_problem() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("notes.md");
    std::fs::write(&source, b"\xff\xfe#\0 \0x\0").unwrap();
    let encoding = Publication::open(&source, Format::Markdown).err().unwrap();
    assert!(encoding.to_string().contains("UTF-8"), "{encoding}");

    std::fs::write(&source, "# Notes").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&source).is_ok() {
        return; // A privileged user can read the file regardless.
    }
    let denied = Publication::open(&source, Format::Markdown).err().unwrap();
    assert!(matches!(denied, Error::Io(_)), "{denied}");
    assert!(!denied.to_string().contains("UTF-8"), "{denied}");
}

#[test]
fn a_failed_or_cancelled_open_does_not_enter_recents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reading.MD");
    std::fs::write(&path, "# Text").unwrap();
    let store = Store::open(&temp.path().join("reading.sqlite3")).unwrap();
    let record = store.document(&path).unwrap().record;
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
    let mut record = store.document(&source).unwrap().record;
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
        store
            .document(&source)
            .unwrap()
            .record
            .locator
            .unwrap()
            .anchor,
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
