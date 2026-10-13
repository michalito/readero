use readero::{
    document::{Anchor, Locator},
    notebook::{History, Notebook, Reference},
    passage::{Passage, PdfFragment, Target, unique_match},
};

fn point() -> Locator {
    Locator {
        version: 1,
        anchor: Anchor::Pdf {
            page: 0,
            x: 10.0,
            y: 20.0,
            viewport_y: 24.0,
        },
    }
}
fn notebook() -> Notebook {
    let mut n = Notebook {
        text: "Before.\n\nLinked café 👩‍🔬.\n\nAfter.".into(),
        ..Notebook::default()
    };
    n.references.push(Reference {
        id: "source".into(),
        label: "Source".into(),
        locator: point(),
        passage: None,
    });
    n.connect(9, 25, "source").unwrap();
    n
}

#[test]
fn native_replacement_transaction_keeps_section_and_undo_restores_both() {
    let mut n = notebook();
    let original = n.clone();
    let mut h = History::default();
    h.begin(&n);
    h.edit(&mut n, 9, 25, "").unwrap();
    h.edit(&mut n, 9, 9, "New 🦉 explanation").unwrap();
    h.end(&n);
    assert_eq!(n.sections.len(), 1);
    assert_eq!(n.sections[0].start, 9);
    assert_eq!(n.sections[0].end, 26);
    let replaced = n.clone();
    h.undo(&mut n, false);
    assert_eq!(n.text, original.text);
    assert_eq!(n.sections, original.sections);
    h.undo(&mut n, true);
    assert_eq!(n.text, replaced.text);
    assert_eq!(n.sections, replaced.sections);
    n.validate().unwrap();
}

#[test]
fn deleting_linked_text_keeps_quote_and_undo_restores_connection() {
    let mut n = notebook();
    let section = n.sections[0].clone();
    let mut h = History::default();
    h.edit(&mut n, section.start, section.end, "").unwrap();
    assert!(n.sections.is_empty());
    assert_eq!(n.references.len(), 1);
    h.undo(&mut n, false);
    assert_eq!(n.sections, vec![section]);
    h.undo(&mut n, true);
    assert!(n.sections.is_empty());
}

#[test]
fn insertion_gravity_excludes_edges_and_includes_interior() {
    let mut n = notebook();
    let mut h = History::default();
    h.edit(&mut n, 9, 9, "A").unwrap();
    assert_eq!((n.sections[0].start, n.sections[0].end), (10, 26));
    h.edit(&mut n, 26, 26, "B").unwrap();
    assert_eq!((n.sections[0].start, n.sections[0].end), (10, 26));
    h.edit(&mut n, 12, 12, "👩‍🔬").unwrap();
    assert_eq!((n.sections[0].start, n.sections[0].end), (10, 29));
    for _ in 0..3 {
        h.undo(&mut n, false);
    }
    assert_eq!(
        n.sections,
        notebook()
            .sections
            .into_iter()
            .map(|mut s| {
                s.id = n.sections[0].id.clone();
                s
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn connections_share_passages_reject_overlap_and_undo_metadata() {
    let mut n = notebook();
    let mut h = History::default();
    assert!(n.connect(8, 11, "source").is_err());
    assert!(n.connect(0, 4, "missing").is_err());
    n.connect(9, 25, "source").unwrap();
    assert_eq!(n.sections[0].passages.len(), 1);
    h.metadata(&mut n, |n| n.connect(0, 7, "source")).unwrap();
    assert_eq!(n.sections.len(), 2);
    let connected = n.clone();
    h.metadata(&mut n, |n| {
        n.remove_passage("source");
        Ok(())
    })
    .unwrap();
    assert!(n.sections.is_empty() && n.references.is_empty());
    h.undo(&mut n, false);
    assert_eq!(n.sections, connected.sections);
    assert_eq!(n.references, connected.references);
    h.undo(&mut n, false);
    assert_eq!(n.sections.len(), 1);
    h.edit(&mut n, 0, 0, "new ").unwrap();
    assert!(!h.can_redo());
}

#[test]
fn randomized_unicode_splices_undo_and_redo_without_anchor_drift() {
    let mut n = notebook();
    n.connect(0, 7, "source").unwrap();
    let mut h = History::default();
    let mut versions = vec![n.clone()];
    let mut seed = 19_u64;
    for i in 0..80 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let len = n.text.chars().count();
        let start = (seed as usize % (len + 1)) as i32;
        let end = (start + (i % 5)).min(len as i32);
        h.edit(&mut n, start, end, if i % 3 == 0 { "🦉é" } else { "z" })
            .unwrap();
        n.validate().unwrap();
        versions.push(n.clone());
    }
    for previous in versions.iter().rev().skip(1) {
        h.undo(&mut n, false);
        assert_eq!(n.text, previous.text);
        assert_eq!(n.sections, previous.sections);
        n.validate().unwrap();
    }
    for next in versions.iter().skip(1) {
        h.undo(&mut n, true);
        assert_eq!(n.text, next.text);
        assert_eq!(n.sections, next.sections);
        n.validate().unwrap();
    }
}

#[test]
fn payload_v1_upgrades_without_inventing_quotations_or_sections() {
    let mut legacy = notebook();
    legacy.sections.clear();
    legacy.version = 1;
    let mut json = serde_json::to_value(&legacy).unwrap();
    json.as_object_mut().unwrap().remove("sections");
    let upgraded = serde_json::from_value::<Notebook>(json)
        .unwrap()
        .upgrade()
        .unwrap();
    assert_eq!(upgraded.version, 2);
    assert_eq!(upgraded.text, legacy.text);
    assert_eq!(upgraded.references, legacy.references);
    assert!(upgraded.sections.is_empty());
}

#[test]
fn exact_quote_matching_rejects_ambiguous_and_overlapping_matches() {
    assert_eq!(unique_match("a word b word c", "word", "", ""), None);
    assert_eq!(unique_match("a word b word c", "word", "a ", " b"), Some(2));
    assert_eq!(unique_match("aaa", "aa", "", ""), None);
    assert_eq!(unique_match("é 🦉 word", "🦉", "é ", " word"), Some(3));
    assert_eq!(unique_match("word", "", "", ""), None);
}

#[test]
fn selection_validation_keeps_dom_utf16_distinct_from_notebook_characters() {
    let locator = Locator {
        version: 1,
        anchor: Anchor::Reflow {
            href: "chapter.xhtml".into(),
            cfi: "epubcfi(/6/2!/4/2/1:0)".into(),
            section: 0,
            fraction: 0.0,
            quote: String::new(),
            block: String::new(),
            block_offset: None,
            quote_offset: 0,
        },
    };
    let mut p = Passage {
        quote: "🦉".into(),
        revision: "r1".into(),
        target: Target::Reflow {
            href: "chapter.xhtml".into(),
            cfi: "epubcfi(/6/2!/4/2,/1:0,/1:2)".into(),
            start: 0,
            end: 2,
            prefix: String::new(),
            suffix: String::new(),
        },
    };
    p.validate(&locator).unwrap();
    if let Target::Reflow { end, .. } = &mut p.target {
        *end = 1;
    }
    assert!(p.validate(&locator).is_err());
    assert!(p.validate(&point()).is_err());
}

#[test]
fn pdf_quote_validation_covers_fragments_in_order_with_page_separators() {
    let fragment = |page, quote: &str| PdfFragment {
        page,
        points: [10.0, 20.0, 80.0, 30.0],
        style: 0,
        quote: quote.into(),
        prefix: String::new(),
        suffix: String::new(),
    };
    let mut p = Passage {
        quote: "First café.\n\u{c}Second 🦉.\n".into(),
        revision: "r1".into(),
        target: Target::Pdf {
            fragments: vec![fragment(0, "First café."), fragment(1, "Second 🦉.")],
        },
    };
    p.validate(&point()).unwrap();
    for invalid in [
        "Unrelated displayed quotation.",
        "Second 🦉. First café.",
        "First café.",
        "First café. unverified Second 🦉.",
        "Unverified First café. Second 🦉.",
        "First café. Second 🦉. Unverified",
    ] {
        p.quote = invalid.into();
        assert!(p.validate(&point()).is_err(), "accepted {invalid:?}");
    }
    p.target = Target::Pdf {
        fragments: vec![fragment(0, "aa"), fragment(1, "aa")],
    };
    p.quote = "aaa".into();
    assert!(p.validate(&point()).is_err(), "overlapping fragments");
    p.quote = "aa\n\naa".into();
    p.validate(&point()).unwrap();
}

#[test]
fn replacing_across_sections_disconnects_each_without_merging_identities() {
    let mut n = notebook();
    n.connect(0, 7, "source").unwrap();
    let original = n.clone();
    let mut history = History::default();
    history.begin(&n);
    history.edit(&mut n, 3, 12, "").unwrap();
    history.edit(&mut n, 3, 3, "replacement").unwrap();
    history.end(&n);
    assert!(n.sections.is_empty());
    assert_eq!(n.references, original.references);
    history.undo(&mut n, false);
    assert_eq!(n.text, original.text);
    assert_eq!(n.sections, original.sections);
    history.undo(&mut n, true);
    assert!(n.sections.is_empty());
}

#[test]
fn metadata_deltas_keep_order_and_preserve_unrelated_quotes() {
    let mut n = notebook();
    for id in ["second", "third", "fourth"] {
        n.references.push(Reference {
            id: id.into(),
            label: id.into(),
            locator: point(),
            passage: None,
        });
    }
    let before = n.clone();
    let mut history = History::default();
    history
        .metadata(&mut n, |n| {
            n.remove_passage("second");
            n.remove_passage("fourth");
            n.references[1].label = "renamed".into();
            n.references.push(Reference {
                id: "last".into(),
                label: "last".into(),
                locator: point(),
                passage: None,
            });
            Ok(())
        })
        .unwrap();
    let after = n.clone();
    history.undo(&mut n, false);
    assert_eq!(n, before);
    history.undo(&mut n, true);
    assert_eq!(n, after);
    let rejected = history.metadata(&mut n, |n| {
        n.references[0].id = "".into();
        Ok(())
    });
    assert!(rejected.is_err());
    assert_eq!(n, after);
}

#[test]
fn history_limit_evicts_oldest_actions_and_new_edit_discards_redo() {
    let mut n = notebook();
    let mut h = History::default();
    for _ in 0..105 {
        h.edit(&mut n, 0, 0, "x").unwrap();
    }
    for _ in 0..100 {
        h.undo(&mut n, false);
    }
    assert!(!h.can_undo());
    assert_eq!(n.text, format!("xxxxx{}", notebook().text));
    assert!(h.can_redo());
    h.edit(&mut n, 0, 0, "new").unwrap();
    assert!(!h.can_redo());
    n.validate().unwrap();
}

#[test]
fn atomic_notebook_save_preserves_old_quote_text_and_ranges_on_failure() {
    use readero::{reading_state::ReadingState, state::Store};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.md");
    std::fs::write(&path, "# Source\n\nQuotation.").unwrap();
    let db = dir.path().join("reading.sqlite3");
    let state = ReadingState::start(db.clone());
    let record = state.document(path).wait().unwrap().record;
    let n = notebook();
    state
        .save_notebook(record.clone(), n.clone())
        .wait()
        .unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_notebook BEFORE INSERT ON notebooks BEGIN SELECT RAISE(FAIL, 'fixture'); END;").unwrap();
    let mut edited = n.clone();
    let mut history = History::default();
    history.edit(&mut edited, 0, 0, "changed").unwrap();
    history
        .metadata(&mut edited, |n| {
            n.references[0].label = "changed".into();
            Ok(())
        })
        .unwrap();
    assert!(state.save_notebook(record.clone(), edited).wait().is_err());
    assert_eq!(Store::open(&db).unwrap().notebook(&record).unwrap(), n);
}
