//! Opt-in native integration driver, excluded from ordinary builds.
use super::*;
use serde_json::{Value, json};
use webkit6::prelude::*;

async fn settle(ms: u64) {
    glib::timeout_future(Duration::from_millis(ms)).await;
}

impl Shell {
    pub fn smoke(self: &Rc<Self>) {
        let Some(directory) = std::env::var_os("READERO_SMOKE_DIR").map(PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&directory).expect("smoke output directory");
        let shell = Rc::clone(self);
        let file = std::env::var_os("READERO_SMOKE_FILE").map(PathBuf::from);
        if let Ok(mode) = std::env::var("READERO_PROBE") {
            self.probe(directory, file.expect("probe document"), mode);
            return;
        }
        glib::MainContext::default().spawn_local(async move {
            let mut report = json!({"ready": false});
            settle(700).await;
            shell.snapshot(&directory.join("01-home.png")).await;
            if let Some(file) = file {
                if let Some(other) = std::env::var_os("READERO_SMOKE_RAPID_FILE") {
                    shell.open(PathBuf::from(other));
                }
                shell.open(file.clone());
                if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
                    report["password_dialog"] = json!(shell.unlock_fixture("incorrect-fixture-password").await);
                    settle(600).await;
                    report["incorrect_password_feedback"] = json!(shell.window.visible_dialog()
                        .and_downcast::<adw::AlertDialog>()
                        .is_some_and(|dialog| dialog.body().contains("didn’t unlock")));
                    shell.unlock_fixture(&password).await;
                }
                shell.wait_ready().await;
                settle(500).await;
                let ready = shell.is_ready();
                report["ready"] = json!(ready);
                report["requested_document_won"] = json!(shell.current.borrow().as_ref()
                    .is_some_and(|record| Some(record.path.clone()) == file.canonicalize().ok()));
                shell.snapshot(&directory.join("02-reading.png")).await;
                if std::env::var_os("READERO_SMOKE_UNSAVED").is_some() {
                    shell.turn(true);
                    shell.save();
                    settle(1000).await;
                    report["storage_error_visible"] = json!(shell.object::<adw::Banner>("save_banner").is_revealed());
                    shell.snapshot(&directory.join("08-storage-error.png")).await;
                    write_json(&directory.join("report.json"), &report);
                    shell.dispose_surface();
                    shell.window.destroy();
                    return;
                }
                if ready {
                    let action_errors = shell.check_action_errors(&directory).await;
                    report["regressions"] = shell.check_regressions().await;
                    report["regressions"].as_object_mut().expect("regression results")
                        .extend(action_errors.as_object().expect("action error results").clone());
                    if std::env::var_os("READERO_SMOKE_EDIT").is_some() {
                        report["editing"] = shell.check_editing(&directory).await;
                    }
                    shell.change_settings(|settings| settings.mode = Mode::Scroll);
                    settle(800).await;
                    for _ in 0..4 {
                        shell.turn(true);
                        settle(250).await;
                    }
                    settle(500).await;
                    report["continuous"] = shell.renderer_diagnostics().await;
                    shell.snapshot(&directory.join("03-scroll.png")).await;
                    let scroll_locator = shell.record_for_save().and_then(|r| r.locator);
                    shell.change_settings(|settings| {
                        settings.mode = Mode::Pages;
                        settings.palette = Palette::Warm;
                    });
                    settle(1000).await;
                    report["paged"] = shell.renderer_diagnostics().await;
                    report["mode_kept_locator"] = json!(shell.record_for_save().and_then(|r| r.locator) == scroll_locator);
                    shell.snapshot(&directory.join("04-pages.png")).await;
                    let target = shell.toc.borrow().last().map(|item| item.target.clone());
                    if let Some(target) = target {
                        shell.goto(target, true);
                        settle(700).await;
                        report["jump_changed_locator"] = json!(shell.record_for_save().and_then(|r| r.locator) != scroll_locator);
                        report["jump"] = shell.renderer_diagnostics().await;
                        shell.history(false);
                        settle(700).await;
                        report["back_kept_locator"] = json!(shell.record_for_save().and_then(|r| r.locator) == scroll_locator);
                    }
                    shell.bookmark();
                    shell.save();
                    shell.show_search();
                    shell.entry().set_text("reading");
                    settle(1500).await;
                    shell.snapshot(&directory.join("05-search.png")).await;
                    report["search_status"] = json!(shell.label("search_status").text().as_str());
                    shell.next_search_result(true);
                    settle(500).await;
                    report["search_jump"] = shell.renderer_diagnostics().await;
                    shell.history(false);
                    settle(500).await;
                    shell.sidebar().set_reveal_child(false);
                    shell.entry().set_text("");
                    shell.search("");
                    shell.toggle_focus();
                    settle(500).await;
                    shell.snapshot(&directory.join("06-focus.png")).await;
                    shell.toggle_focus();
                    settle(500).await;
                    if std::env::var_os("READERO_SMOKE_STRESS").is_some() {
                        let mut passes = Vec::new();
                        for index in 0..20 {
                            shell.change_settings(|settings| {
                                settings.mode = if index % 2 == 0 { Mode::Scroll } else { Mode::Pages };
                                settings.font_size = if index % 4 < 2 { 20.0 } else { 22.0 };
                            });
                            settle(450).await;
                            if index % 2 == 0 {
                                for _ in 0..3 {
                                    shell.turn(index < 10);
                                    settle(100).await;
                                }
                            }
                            passes.push(shell.renderer_diagnostics().await);
                        }
                        report["stress"] = json!(passes);
                    }
                    let before = shell.record_for_save();
                    write_json(&directory.join("state-before.json"), &before);
                    report["renderer"] = shell.renderer_diagnostics().await;
                    let view = match shell.surface.borrow().as_ref() {
                        Some(Surface::Reflow(reader)) => Some(reader.view.clone()),
                        _ => None,
                    };
                    if let Some(view) = view {
                        let result = view.evaluate_javascript_future(
                            "JSON.stringify({bridgeHidden:!window.webkit?.messageHandlers?.readero,authoredScriptRan:Array.from(window.frames).some(f=>f.document.documentElement.dataset.authored==='yes')})",
                            None, None,
                        ).await;
                        if let Ok(value) = result {
                            report["isolation"] = serde_json::from_str(&value.to_str()).unwrap_or(Value::Null);
                        }
                    }
                    shell.open(file);
                    if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
                        shell.unlock_fixture(&password).await;
                    }
                    shell.wait_ready().await;
                    settle(700).await;
                    shell.snapshot(&directory.join("07-reopened.png")).await;
                    let after = shell.record_for_save();
                    write_json(&directory.join("state-after.json"), &after);
                    report["resume_kept_locator"] = json!(before.as_ref().and_then(|r| r.locator.as_ref()) == after.as_ref().and_then(|r| r.locator.as_ref()));
                    report["resume_kept_settings"] = json!(before.as_ref().map(|r| &r.settings) == after.as_ref().map(|r| &r.settings));
                    report["reopened"] = shell.renderer_diagnostics().await;
                    if let Some(record) = after {
                        let id = record.id;
                        report["bookmark_count"] = json!(shell.reading_state.bookmarks(id).await.map_or(0, |bookmarks| bookmarks.len()));
                    }
                    if let Some(ms) = std::env::var("READERO_SMOKE_IDLE_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
                        write_json(&directory.join("idle-ready.json"), &json!({"pid":std::process::id()}));
                        settle(ms).await;
                    }
                    let recovery = shell.check_recovery(&directory).await;
                    report["regressions"].as_object_mut().expect("regression results")
                        .extend(recovery.as_object().expect("recovery results").clone());
                }
            }
            write_json(&directory.join("report.json"), &report);
            eprintln!("READERO_SMOKE report={}", directory.join("report.json").display());
            shell.close();
        });
    }
    async fn check_action_errors(self: &Rc<Self>, directory: &std::path::Path) -> Value {
        let mut checks = json!({});
        let record = self
            .record_for_save()
            .expect("ready for action error checks");
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("drain initial save");
        let connection = rusqlite::Connection::open(directory.join("state/reading.sqlite3"))
            .expect("test database");
        connection
            .execute_batch(
                "CREATE TRIGGER reject_bookmark BEFORE INSERT ON bookmarks
            BEGIN SELECT RAISE(FAIL, 'fixture rejects bookmark'); END;",
            )
            .unwrap();
        self.bookmark();
        self.save();
        // Deliberately keep GTK from dispatching either completion until the
        // later save has cleared global status. No timing race in this fixture.
        self.reading_state
            .barrier(Duration::ZERO)
            .wait()
            .expect("complete queued actions");
        assert!(self.reading_state.status().error.is_none());
        settle(300).await;
        checks["failed_bookmark_feedback_survives_later_save"] =
            json!(has_label(&self.window, "Couldn’t bookmark this passage"));
        checks["failed_bookmark_was_not_inserted"] = json!(
            self.reading_state
                .bookmarks(record.id.clone())
                .await
                .unwrap()
                .is_empty()
        );
        connection
            .execute_batch(
                "DROP TRIGGER reject_bookmark;
            CREATE TRIGGER reject_removal BEFORE UPDATE OF hidden ON documents
            WHEN NEW.hidden=1 BEGIN SELECT RAISE(FAIL, 'fixture rejects removal'); END;",
            )
            .unwrap();
        self.home();
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("load recents");
        settle(300).await;
        assert!(click_button(&self.content(), "Remove from recents"));
        self.save();
        self.reading_state
            .barrier(Duration::ZERO)
            .wait()
            .expect("complete queued removal");
        assert!(self.reading_state.status().error.is_none());
        settle(300).await;
        checks["failed_removal_feedback_survives_later_save"] = json!(has_label(
            &self.window,
            "Couldn’t remove this document from recents"
        ));
        checks["failed_removal_keeps_recent"] = json!(
            self.reading_state
                .recents()
                .await
                .unwrap()
                .iter()
                .any(|recent| recent.id == record.id)
        );
        connection
            .execute_batch("DROP TRIGGER reject_removal")
            .unwrap();
        self.open(record.path);
        if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
            self.unlock_fixture(&password).await;
        }
        self.wait_ready().await;
        checks
    }

    async fn check_recovery(self: &Rc<Self>, directory: &std::path::Path) -> Value {
        let mut checks = json!({});
        let Some(mut record) = self.record_for_save() else {
            return checks;
        };
        let id = record.id.clone();
        self.home();
        // Drain the save-before-home and recent-load requests.
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("state worker");
        settle(200).await;
        // Exercise retention through the same save operation as the shell.
        let database = directory.join("state/reading.sqlite3");
        let lock = rusqlite::Connection::open(&database).expect("test database");
        lock.execute_batch("BEGIN EXCLUSIVE")
            .expect("lock database");
        let mut retained = record.clone();
        retained.id = "provisional-removal-session".into();
        assert!(self.reading_state.save(Some(retained)).await.is_err());
        self.refresh_storage();
        lock.execute_batch("ROLLBACK").expect("unlock database");
        self.render_home(&[record.clone()]);
        checks["failed_save_recent_removal_clicked"] =
            json!(click_button(&self.content(), "Remove from recents"));
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("drain removal");
        settle(200).await;
        checks["recent_removal_cancels_retained_snapshot"] =
            json!(!self.reading_state.status().pending);
        checks["recent_removal_clears_final_save_error"] =
            json!(!self.object::<adw::Banner>("save_banner").is_revealed());
        record.path = directory.join("missing-recent.md");
        self.render_home(&[record.clone()]);
        checks["continue_card_can_remove_missing_document"] =
            json!(click_button(&self.content(), "Remove from recents"));
        settle(250).await;
        let hidden = !self
            .reading_state
            .recents()
            .await
            .expect("query recents")
            .iter()
            .any(|record| record.id == id);
        let bookmarks = self
            .reading_state
            .bookmarks(id.clone())
            .await
            .expect("query bookmarks")
            .len();
        checks["continue_removal_hides_recent"] = json!(hidden);
        checks["continue_removal_keeps_bookmarks"] = json!(bookmarks == 1);

        // Delay removal, then open a document before the callback is delivered.
        self.render_home(&[record.clone()]);
        let delay = self.reading_state.barrier(Duration::from_millis(200));
        checks["delayed_recent_removal_clicked"] =
            json!(click_button(&self.content(), "Remove from recents"));
        let source = std::env::var_os("READERO_SMOKE_FILE")
            .map(PathBuf::from)
            .expect("source");
        let source = if record.format == Format::Pdf {
            let path = directory.join("recent-race.md");
            std::fs::write(&path, "# Reading again\n\nA reading passage.").expect("race document");
            path
        } else {
            source
        };
        self.open(source.clone());
        delay.await.expect("delayed worker");
        self.wait_ready().await;
        settle(300).await;
        checks["delayed_recent_removal_keeps_new_reader"] = json!(
            self.record_for_save()
                .is_some_and(|current| current.path == source.canonicalize().expect("source path"))
                && self.content().visible_child_name().as_deref() == Some("reader")
        );
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("drain writes");
        settle(100).await;
        let database = directory.join("state/reading.sqlite3");
        let lock = rusqlite::Connection::open(&database).expect("test database");
        lock.execute_batch("BEGIN EXCLUSIVE")
            .expect("lock database");
        self.change_settings(|settings| settings.font_size = 23.0);
        let unsaved = self.record_for_save().expect("reader before Home");
        self.home();
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("drain failed save");
        settle(100).await;
        checks["save_failure_after_home_is_visible"] =
            json!(self.object::<adw::Banner>("save_banner").is_revealed());
        checks["save_failure_after_home_keeps_snapshot"] =
            json!(self.reading_state.status().pending);
        lock.execute_batch("ROLLBACK").expect("unlock database");
        self.save();
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("drain retry");
        settle(100).await;
        let path = unsaved.path.clone();
        let recovered = self
            .reading_state
            .document(path)
            .await
            .expect("saved previous document")
            .record;
        checks["retry_from_home_recovers_previous_document"] = json!(
            recovered.locator == unsaved.locator
                && recovered.settings == unsaved.settings
                && !self.reading_state.status().pending
                && !self.object::<adw::Banner>("save_banner").is_revealed()
        );

        // Exercise the actual fallback UUID and native Retry saving action.
        if record.format != Format::Pdf {
            let source = std::env::var_os("READERO_SMOKE_FILE")
                .map(PathBuf::from)
                .expect("source");
            let database = directory.join("state/reading.sqlite3");
            let lock = rusqlite::Connection::open(database).expect("test database");
            lock.execute_batch("BEGIN EXCLUSIVE")
                .expect("lock database");
            self.open(source.clone());
            self.wait_ready().await;
            checks["storage_lock_uses_provisional_identity"] =
                json!(self.record_for_save().is_some_and(|record| record.id != id));
            self.turn(true);
            self.renderer_diagnostics().await;
            self.change_settings(|settings| settings.font_size = 24.0);
            self.renderer_diagnostics().await;
            let session = self.record_for_save().expect("fallback reader");
            lock.execute_batch("ROLLBACK").expect("unlock database");
            self.save();
            let saved = self
                .reading_state
                .document(source)
                .await
                .expect("recovered state")
                .record;
            settle(200).await;
            checks["storage_retry_reconnects_identity"] = json!(
                saved.id == id
                    && self
                        .current
                        .borrow()
                        .as_ref()
                        .is_some_and(|record| record.id == id)
            );
            checks["storage_retry_keeps_session_progress"] = json!(
                saved.locator == session.locator
                    && saved.settings == session.settings
                    && self.record_for_save().and_then(|record| record.locator) == session.locator
            );
            checks["storage_retry_clears_error"] =
                json!(!self.object::<adw::Banner>("save_banner").is_revealed());
            self.bookmark();
            // Bookmark work enters the queue before this subsequent read.
            let count = self
                .reading_state
                .bookmarks(id)
                .await
                .expect("recovered bookmarks")
                .len();
            checks["storage_recovery_keeps_old_and_new_bookmarks"] = json!(count == 2);
        }
        checks
    }
    async fn check_regressions(self: &Rc<Self>) -> Value {
        let mut checks = json!({});
        let locator = self.record_for_save().and_then(|record| record.locator);
        let Some(locator) = locator else {
            return json!({"initial_locator_available": false});
        };
        let fixture_dir =
            PathBuf::from(std::env::var_os("READERO_SMOKE_DIR").expect("smoke output"))
                .join("invalid-opens");
        std::fs::create_dir_all(&fixture_dir).expect("invalid open fixtures");
        let generation = self.generation();
        let record = self.record_for_save().expect("current session");
        let original_back = self.back.borrow().clone();
        let original_forward = self.forward.borrow().clone();
        let original_sidebar = self.sidebar().reveals_child();
        for (name, bytes) in [
            ("unsupported.txt", &b"not a reading document"[..]),
            ("empty.md", &b""[..]),
            ("corrupt.epub", &b"not a zip archive"[..]),
            ("corrupt.pdf", &b"not a PDF"[..]),
        ] {
            let path = fixture_dir.join(name);
            std::fs::write(&path, bytes).expect("fixture");
            self.open(path);
            self.wait_ready().await;
            checks[format!("failed_open_preserves_{name}")] = json!(
                self.generation() == generation
                    && self
                        .record_for_save()
                        .is_some_and(|current| current.id == record.id
                            && current.locator == record.locator
                            && current.settings == record.settings)
                    && *self.back.borrow() == original_back
                    && *self.forward.borrow() == original_forward
                    && self.sidebar().reveals_child() == original_sidebar
                    && self.content().visible_child_name().as_deref() == Some("reader")
            );
        }
        // This archive passes native validation but fails during EPUB startup.
        let broken = fixture_dir.join("broken-package.epub");
        {
            use std::io::Write;
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&broken).expect("zip"));
            zip.start_file(
                "META-INF/container.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("container");
            zip.write_all(b"<container/>").expect("bad container");
            zip.finish().expect("finish zip");
        }
        self.open(broken);
        self.wait_ready().await;
        checks["renderer_failure_preserves_reader"] = json!(
            !self.is_opening()
                && self.generation() == generation
                && self.content().visible_child_name().as_deref() == Some("reader")
                && self.record_for_save().is_some_and(
                    |current| current.id == record.id && current.locator == record.locator
                )
        );
        if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
            let reader = self.content().visible_child();
            self.open(record.path.clone());
            let mut prompt = None;
            for _ in 0..100 {
                settle(50).await;
                if let Some(dialog) = self
                    .window
                    .visible_dialog()
                    .and_downcast::<adw::AlertDialog>()
                {
                    prompt = Some(dialog);
                    break;
                }
            }
            checks["superseded_password_prompt_presented"] = json!(prompt.is_some());
            self.cancel_open();
            settle(400).await;
            checks["superseded_password_prompt_closed"] =
                json!(self.window.visible_dialog().is_none());
            if let Some(dialog) = prompt {
                if let Some(entry) = dialog.extra_child().and_downcast::<gtk::PasswordEntry>() {
                    entry.set_text(&password);
                }
                // Deliver a response which was queued for the obsolete job.
                dialog.emit_by_name::<()>("response", &[&"unlock"]);
            }
            settle(400).await;
            checks["stale_password_response_cannot_replace_reader"] = json!(
                !self.is_opening() && self.content().visible_child() == reader && self.is_ready()
            );
        }
        let kind = self.object::<gtk::DropDown>("sidebar_kind");
        let previous_kind = kind.selected();
        self.show_search();
        self.entry().set_text("reading");
        for selected in [0, 1] {
            kind.set_selected(selected);
            settle(250).await;
            let row = self.list().row_at_index(0);
            let status = self.label("search_status").text();
            self.web_message(reflow::Message {
                generation: self.generation(),
                event: reflow::Event::Search {
                    query: "reading".into(),
                    items: vec![reflow::SearchItem {
                        label: "Late result".into(),
                        locator: locator.clone(),
                    }],
                    complete: true,
                },
            });
            self.pdf_search_results(1);
            checks[format!("late_search_preserves_sidebar_{selected}")] = json!(
                self.list().row_at_index(0) == row && self.label("search_status").text() == status
            );
        }
        self.entry().set_text("");
        kind.set_selected(previous_kind);
        let back = self.back.take();
        let forward = self.forward.take();
        self.sync_controls();
        self.web_message(reflow::Message {
            generation: self.generation(),
            event: reflow::Event::Jump {
                locator: Some(locator),
            },
        });
        checks["internal_link_enables_back"] =
            json!(self.button("back_button").is_sensitive() && self.back.borrow().len() == 1);
        self.back.replace(back);
        self.forward.replace(forward);
        self.sync_controls();

        let view = match self.surface.borrow().as_ref() {
            Some(Surface::Reflow(reader)) => Some(reader.view.clone()),
            _ => None,
        };
        if let Some(view) = view {
            for (script, result_name) in [
                (
                    include_str!("../../tests/anchors.js"),
                    "readeroAnchorChecks",
                ),
                (
                    include_str!("../../tests/continuous.js"),
                    "readeroContinuousChecks",
                ),
                (include_str!("../../tests/search.js"), "readeroSearchChecks"),
            ] {
                let _ = view
                    .evaluate_javascript_future(script, Some(reflow::WORLD), None)
                    .await;
                checks[result_name] = json!(false);
                for _ in 0..300 {
                    settle(50).await;
                    if let Ok(value) = view
                        .evaluate_javascript_future(
                            &format!("JSON.stringify(globalThis.{result_name} ?? null)"),
                            Some(reflow::WORLD),
                            None,
                        )
                        .await
                        && let Ok(Value::Object(results)) = serde_json::from_str(&value.to_str())
                    {
                        checks[result_name] = json!(!results.contains_key("error"));
                        for (key, value) in results {
                            checks[key] = value;
                        }
                        break;
                    }
                }
            }
        }

        let pdf = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => Some(Rc::clone(pdf)),
            _ => None,
        };
        if let Some(pdf) = pdf {
            let original = self.record_for_save().expect("ready PDF");
            for mode in [Mode::Scroll, Mode::Pages] {
                for rotation in [0, 90, 180, 270] {
                    self.change_settings(|settings| {
                        settings.mode = mode;
                        settings.rotation = rotation;
                        settings.pdf_sizing = "custom".into();
                        settings.pdf_scale = 3.0;
                    });
                    settle(500).await;
                    let horizontal = pdf.scroll.hadjustment();
                    let vertical = pdf.scroll.vadjustment();
                    let (width, height) = pdf.model.document().expect("PDF").page_size(0);
                    let width = if rotation % 180 == 0 { width } else { height };
                    // A mixed-size document's global scroll extent also includes
                    // blank gutter beside its narrower pages. Exercise readable
                    // panning positions on this page, not an entirely blank view.
                    let extent = (horizontal.upper() - horizontal.page_size())
                        .min(width * pdf.model.scale() - horizontal.page_size());
                    let mut restored = extent > 0.0;
                    for fraction in [0.15, 0.5, 0.95] {
                        vertical.set_value(240.0);
                        horizontal.set_value(extent * fraction);
                        settle(100).await;
                        let expected = horizontal.value();
                        let saved = pdf.location();
                        horizontal.set_value(0.0);
                        pdf.restore(saved);
                        settle(350).await;
                        restored &= (horizontal.value() - expected).abs() < 2.0;
                    }
                    checks[format!("pdf_horizontal_{mode:?}_{rotation}")] = json!(restored);
                }
            }
            self.change_settings(|settings| *settings = original.settings);
            pdf.restore(original.locator);
            settle(500).await;
        }
        checks
    }
    async fn check_editing(self: &Rc<Self>, directory: &std::path::Path) -> Value {
        let mut checks = self.check_reload_failure().await;
        checks.as_object_mut().expect("reload checks").extend(
            self.check_opening_lifecycle(directory)
                .await
                .as_object()
                .expect("opening checks")
                .clone(),
        );
        self.change_settings(|settings| settings.mode = Mode::Scroll);
        self.renderer_diagnostics().await;
        for _ in 0..3 {
            self.turn(true);
            settle(200).await;
        }
        self.renderer_diagnostics().await;
        if !self.focus.get() {
            self.sidebar().set_reveal_child(true);
            self.toggle_focus();
        }
        settle(400).await;
        self.renderer_diagnostics().await;
        let original = self.record_for_save().expect("editable fixture ready");
        assert!(original.path.starts_with(directory));
        assert_eq!(original.format, Format::Markdown);
        let source = std::fs::read_to_string(&original.path).expect("read generated fixture");
        self.push_history(original.locator.clone().expect("position"));
        let history_size = self.back.borrow().len();
        for outcome in ["failure", "cancel", "invalid"] {
            let generation = self.generation();
            // Queue a reload first, then start the candidate before its async
            // result can be delivered. No further file event rescues the reload.
            self.reload(original.path.clone());
            let delay = self.reading_state.barrier(Duration::from_millis(1200));
            self.open(directory.join(if outcome == "invalid" {
                "invalid-candidate.txt"
            } else {
                "missing-candidate.md"
            }));
            if outcome == "cancel" {
                self.cancel_open();
            }
            delay.await.expect("delayed candidate");
            for _ in 0..100 {
                settle(100).await;
                if self.generation() != generation {
                    break;
                }
            }
            self.wait_ready().await;
            checks[format!("inflight_reload_recovers_{outcome}")] = json!(
                self.generation() != generation
                    && self.is_ready()
                    && self
                        .record_for_save()
                        .is_some_and(|r| r.path == original.path)
            );
        }
        for cancel in [false, true] {
            let generation = self.generation();
            // Hold the candidate open across the watcher's debounce interval.
            let delay = self.reading_state.barrier(Duration::from_millis(1800));
            self.open(directory.join("missing-candidate.md"));
            std::fs::write(&original.path, &source).expect("edit while opening");
            settle(1100).await;
            checks[format!("reload_waits_for_candidate_{cancel}")] = json!(
                self.is_opening() && self.generation() == generation && self.reload_pending()
            );
            if cancel {
                self.cancel_open();
            }
            delay.await.expect("delayed candidate");
            for _ in 0..100 {
                settle(100).await;
                if self.generation() != generation {
                    break;
                }
            }
            self.wait_ready().await;
            self.renderer_diagnostics().await;
            checks[format!("deferred_reload_recovers_{cancel}")] = json!(
                self.generation() != generation
                    && self.is_ready()
                    && self
                        .record_for_save()
                        .is_some_and(|r| r.path == original.path)
            );
        }
        for atomic in [false, true] {
            let generation = self.generation();
            let surface = self.content().visible_child();
            if atomic {
                let staging = directory.join("blank-replacement.md");
                std::fs::write(&staging, " ").expect("write blank fixture");
                std::fs::rename(staging, &original.path).expect("replace with blank fixture");
            } else {
                std::fs::write(&original.path, " ").expect("blank fixture");
            }
            settle(1400).await;
            checks[format!("blank_reload_keeps_reader_{atomic}")] = json!(
                self.is_current(generation)
                    && self.is_ready()
                    && self.content().visible_child() == surface
                    && self.record_for_save().and_then(|r| r.locator) == original.locator
            );
            checks[format!("blank_reload_keeps_watcher_{atomic}")] = json!(self.watching_source());
            std::fs::write(&original.path, &source).expect("correct blank fixture");
            for _ in 0..100 {
                settle(100).await;
                if self.generation() != generation {
                    break;
                }
            }
            self.wait_ready().await;
            self.renderer_diagnostics().await;
            checks[format!("corrected_reload_recovers_{atomic}")] = json!(
                self.generation() != generation
                    && self.is_ready()
                    && self.record_for_save().and_then(|r| r.locator) == original.locator
            );
        }
        for atomic in [false, true] {
            let generation = self.generation();
            let content = format!(
                "An inserted paragraph {}.\n\n{source}",
                if atomic {
                    "after an atomic replacement"
                } else {
                    "during reading"
                }
            );
            if atomic {
                let staging = directory.join("replacement.md");
                std::fs::write(&staging, content).expect("write isolated fixture");
                std::fs::rename(staging, &original.path).expect("replace isolated fixture");
            } else {
                std::fs::write(&original.path, content).expect("edit isolated fixture");
            }
            for _ in 0..100 {
                settle(100).await;
                if self.generation() != generation {
                    break;
                }
            }
            self.wait_ready().await;
            let diagnostic = self.renderer_diagnostics().await;
            let saved = self.record_for_save();
            write_json(
                &directory.join(format!("edit-{atomic}.json")),
                &json!({"before":original,"after":saved,"renderer":diagnostic}),
            );
            let same_passage = match (
                original.locator.as_ref().map(|l| &l.anchor),
                saved
                    .as_ref()
                    .and_then(|r| r.locator.as_ref())
                    .map(|l| &l.anchor),
            ) {
                (
                    Some(Anchor::Reflow {
                        block: a,
                        block_offset: x,
                        ..
                    }),
                    Some(Anchor::Reflow {
                        block: b,
                        block_offset: y,
                        ..
                    }),
                ) => a == b && x == y,
                _ => false,
            };
            let prefix = if atomic { "atomic_edit" } else { "source_edit" };
            checks[format!("{prefix}_detected")] = json!(self.generation() != generation);
            checks[format!("{prefix}_keeps_passage")] =
                json!(same_passage && diagnostic["anchorVisible"] == true);
            checks[format!("{prefix}_keeps_focus")] = json!(
                self.focus.get()
                    && !self.object::<adw::HeaderBar>("header").is_visible()
                    && !self.object::<gtk::Box>("footer").is_visible()
            );
            checks[format!("{prefix}_keeps_history")] =
                json!(self.back.borrow().len() == history_size);
        }
        self.toggle_focus();
        checks["focus_exit_restores_sidebar"] = json!(self.sidebar().reveals_child());
        checks["markdown_has_reading_percentage"] =
            json!(self.label("position_label").text().ends_with("% read"));
        let before_link = self.record_for_save().expect("edited document ready");
        let related = format!(
            "# Earlier material\n\n{}\n\n# Linked passage\n\nThe requested example begins here.\n\n{}",
            "An earlier paragraph.\n\n".repeat(50),
            "More reading.\n\n".repeat(20)
        );
        std::fs::write(directory.join("related.md"), related).expect("write related fixture");
        self.related("related.md#linked-passage");
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        let linked = self.record_for_save().expect("related fixture ready");
        let view = match self.surface.borrow().as_ref() {
            Some(Surface::Reflow(r)) => Some(r.view.clone()),
            _ => None,
        };
        if let Some(view) = view {
            let result = view.evaluate_javascript_future("JSON.stringify(globalThis.readeroDiagnostics().locator.block === Array.from(window.frames).map(f => f.document.getElementById('linked-passage')?.closest('[data-reader-block]')?.id).find(Boolean))", Some(reflow::WORLD), None).await;
            checks["related_link_opens_heading"] =
                json!(result.is_ok_and(|value| value.to_str() == "true"));
        }
        self.history(false);
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        checks["cross_document_back_keeps_passage"] =
            json!(self.record_for_save().and_then(|r| r.locator) == before_link.locator);
        self.history(true);
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        checks["cross_document_forward_keeps_passage"] =
            json!(self.record_for_save().and_then(|r| r.locator) == linked.locator);
        self.history(false);
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        self.back.borrow_mut().clear();
        self.forward.borrow_mut().clear();
        self.sync_controls();
        checks
    }

    async fn check_reload_failure(self: &Rc<Self>) -> Value {
        self.renderer_diagnostics().await;
        let original = self.record_for_save().expect("ready before reload failure");
        let surface = self.content().visible_child();
        reflow::fail_next_start();
        self.reload(original.path.clone());
        for _ in 0..100 {
            settle(50).await;
            if has_label(&self.window, "Injected renderer startup failure") {
                break;
            }
        }
        let retained = self.record_for_save();
        let checks = json!({
            "reload_startup_failure_reported": has_label(&self.window, "Injected renderer startup failure"),
            "reload_startup_failure_keeps_reader": self.content().visible_child() == surface,
            "reload_startup_failure_keeps_reading_state": retained.as_ref().is_some_and(|r|
                r.id == original.id && r.locator == original.locator && r.settings == original.settings),
        });
        // Restore the fixture even in the red run so unrelated checks continue.
        self.open(original.path);
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        checks
    }

    async fn check_opening_lifecycle(self: &Rc<Self>, directory: &std::path::Path) -> Value {
        let original = self.record_for_save().expect("ready for opening checks");
        let source = std::fs::read_to_string(&original.path).expect("source fixture");
        let other = directory.join("other-opening.md");
        std::fs::write(
            &other,
            "# Another document\n\nIndependent reading material.",
        )
        .expect("candidate fixture");
        let mut checks = json!({});

        let delay = self.reading_state.barrier(Duration::from_millis(800));
        self.open(original.path.clone());
        self.change_settings(|settings| settings.font_size = 25.0);
        delay.await.expect("delayed same-document preparation");
        self.wait_ready().await;
        checks["same_document_open_captures_latest_settings"] = json!(
            self.record_for_save()
                .is_some_and(|r| r.settings.font_size == 25.0)
        );
        self.change_settings(|settings| *settings = original.settings.clone());
        self.renderer_diagnostics().await;

        for reload in [false, true] {
            let mut late = self.record_for_save().unwrap().locator.unwrap();
            if let Anchor::Reflow { fraction, .. } = &mut late.anchor {
                *fraction = if reload { 0.3456 } else { 0.2345 };
            }
            reflow::delay_next_start(900);
            if reload {
                self.reload(original.path.clone());
            } else {
                self.open(original.path.clone());
            }
            checks[format!("same_document_candidate_mapped_{reload}")] =
                json!(self.wait_candidate().await);
            self.web_message(reflow::Message {
                generation: self.generation(),
                event: reflow::Event::Location {
                    locator: late.clone(),
                    section: 1,
                    total: 1,
                },
            });
            self.wait_ready().await;
            let renderer = self.renderer_diagnostics().await;
            checks[format!("same_document_renderer_keeps_late_location_{reload}")] =
                json!(renderer["locator"] == json!(late));
            checks[format!("same_document_keeps_late_location_{reload}")] =
                json!(self.record_for_save().and_then(|r| r.locator) == Some(late.clone()));
            self.save();
            let saved = self
                .reading_state
                .document(original.path.clone())
                .await
                .unwrap();
            checks[format!("same_document_persists_late_location_{reload}")] =
                json!(saved.record.locator == Some(late));
        }

        // The native candidate is mapped but has not emitted Ready. A second
        // source change must survive its successful same-document commitment.
        let old_generation = self.generation();
        reflow::delay_next_start(900);
        self.reload(original.path.clone());
        checks["reload_candidate_is_mapped"] = json!(self.wait_candidate().await);
        let newer = format!("{source}\n\nAn edit made during replacement startup.\n");
        std::fs::write(&original.path, &newer).expect("newer source");
        let revision = Publication::open(&original.path, Format::Markdown)
            .expect("new publication")
            .revision;
        checks["edit_during_reload_reaches_reader"] = json!(self.wait_revision(&revision).await);
        let reader = self.content().visible_child();
        settle(1100).await;
        checks["reload_stops_after_latest_edit"] = json!(self.content().visible_child() == reader);

        let current = self.record_for_save().expect("latest revision ready");
        self.web_message(reflow::Message {
            generation: old_generation,
            event: reflow::Event::Ready {
                title: "Obsolete reader".into(),
                toc: Vec::new(),
                locator: None,
                total: 1,
            },
        });
        checks["stale_ready_cannot_replace_reading_state"] =
            json!(self.record_for_save().is_some_and(|r| r.id == current.id
                && r.title == current.title
                && r.locator == current.locator
                && r.revision == current.revision));

        reflow::delay_next_start(900);
        self.open(other.clone());
        checks["replacement_candidate_is_mapped"] = json!(self.wait_candidate().await);
        let mut late_locator = current.locator.clone().expect("active reader location");
        if let Anchor::Reflow { fraction, .. } = &mut late_locator.anchor {
            *fraction = 0.4321;
        }
        self.web_message(reflow::Message {
            generation: self.generation(),
            event: reflow::Event::Location {
                locator: late_locator.clone(),
                section: 1,
                total: 1,
            },
        });
        checks["mapped_candidate_retains_active_location"] =
            json!(self.record_for_save().and_then(|r| r.locator) == Some(late_locator.clone()));
        let bookmarks = self
            .reading_state
            .bookmarks(original.id.clone())
            .await
            .expect("existing bookmarks");
        self.bookmark();
        let after = self
            .reading_state
            .bookmarks(original.id.clone())
            .await
            .expect("bookmarks after blocked action");
        checks["mapped_candidate_cannot_bookmark_retained_document"] =
            json!(after.len() == bookmarks.len());
        for added in after
            .iter()
            .filter(|b| !bookmarks.iter().any(|old| old.id == b.id))
        {
            self.reading_state
                .remove_bookmark(added.id.clone())
                .await
                .expect("remove red-run bookmark");
        }
        let newer = format!("{source}\n\nAn edit made while another document opens.\n");
        std::fs::write(&original.path, newer).expect("edit retained reader");
        self.wait_ready().await;
        settle(1100).await;
        checks["replacement_discards_previous_document_reload"] =
            json!(self.record_for_save().is_some_and(|r| r.path == other));
        let saved = self
            .reading_state
            .document(original.path.clone())
            .await
            .expect("previous reader persisted");
        checks["replacement_saves_late_active_location"] =
            json!(saved.record.locator == Some(late_locator));

        self.open(original.path.clone());
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        let before = self.record_for_save().expect("return to source");
        let reader = self.content().visible_child();
        self.push_history(before.locator.clone().expect("history target"));
        let delay = self.reading_state.barrier(Duration::from_millis(900));
        self.open(other.clone());
        self.history(false);
        delay.await.expect("delayed candidate");
        settle(800).await;
        checks["active_navigation_supersedes_candidate"] = json!(
            !self.is_opening()
                && self.content().visible_child() == reader
                && self
                    .record_for_save()
                    .is_some_and(|r| r.path == original.path)
        );

        let delay = self.reading_state.barrier(Duration::from_millis(700));
        self.open(other);
        self.home();
        delay.await.expect("delayed Home completion");
        settle(700).await;
        checks["home_rejects_late_open_completion"] = json!(
            !self.is_ready()
                && self.record_for_save().is_none()
                && self.content().visible_child_name().as_deref() == Some("home")
        );
        for forward in [false, true] {
            self.home();
            let delay = self.reading_state.barrier(Duration::from_millis(700));
            self.open(original.path.clone());
            self.history(forward);
            checks[format!("empty_history_keeps_first_open_{forward}")] = json!(self.is_opening());
            delay.await.expect("delayed first open");
            if self.is_opening() {
                self.wait_ready().await;
            }
            checks[format!("empty_history_first_open_finishes_{forward}")] = json!(self.is_ready());
        }
        self.open(original.path.clone());
        self.wait_ready().await;
        self.renderer_diagnostics().await;
        self.reading_state
            .barrier(Duration::ZERO)
            .await
            .expect("drain saves");

        // Let a watcher event arrive while a slow close is waiting for a save
        // which will fail. Choosing Keep open must retain that source change.
        let database = rusqlite::Connection::open(directory.join("state/reading.sqlite3"))
            .expect("test database");
        database.execute_batch("CREATE TRIGGER reject_close BEFORE INSERT ON documents BEGIN SELECT RAISE(FAIL, 'fixture rejects close save'); END;").expect("reject close save");
        let delay = self.reading_state.barrier(Duration::from_millis(900));
        let before_close = self.record_for_save().and_then(|r| r.locator);
        self.turn(true);
        self.close();
        let newer = format!("{source}\n\nAn edit made while closing.\n");
        std::fs::write(&original.path, newer).expect("edit during close");
        let revision = Publication::open(&original.path, Format::Markdown)
            .expect("close-time publication")
            .revision;
        delay.await.expect("slow close");
        let mut kept_open = false;
        for _ in 0..100 {
            settle(50).await;
            if let Some(dialog) = self
                .window
                .visible_dialog()
                .and_downcast::<adw::AlertDialog>()
            {
                let diagnostic = self.renderer_diagnostics().await;
                let checkpoint = serde_json::from_value::<Locator>(diagnostic["locator"].clone())
                    .expect("renderer checkpoint locator");
                checks["close_checkpoint_includes_pending_navigation"] =
                    json!(Some(&checkpoint) != before_close.as_ref());
                checks["failed_close_keeps_latest_checkpoint"] = json!(
                    self.record_for_save().and_then(|r| r.locator).as_ref() == Some(&checkpoint)
                );
                self.open(directory.join("other-opening.md"));
                checks["failed_close_waits_for_choice_before_opening"] = json!(!self.is_opening());
                database
                    .execute_batch("DROP TRIGGER reject_close")
                    .expect("allow saves again");
                kept_open = click_button(&dialog, "Keep open");
                break;
            }
        }
        checks["failed_close_offers_keep_open"] = json!(kept_open);
        checks["failed_close_retains_source_change"] = json!(self.wait_revision(&revision).await);
        // Restore the fixture for the existing passage and focus tests.
        std::fs::write(&original.path, &source).expect("restore source");
        let revision = Publication::open(&original.path, Format::Markdown)
            .expect("restored publication")
            .revision;
        checks["source_recovers_after_lifecycle_checks"] =
            json!(self.wait_revision(&revision).await);
        self.back.borrow_mut().clear();
        self.forward.borrow_mut().clear();
        self.sync_controls();
        checks
    }

    async fn wait_candidate(&self) -> bool {
        for _ in 0..100 {
            if self.content().visible_child_name().as_deref() == Some("candidate") {
                return true;
            }
            settle(20).await;
        }
        false
    }

    async fn wait_revision(&self, revision: &str) -> bool {
        for _ in 0..120 {
            settle(50).await;
            if !self.is_opening()
                && self
                    .record_for_save()
                    .is_some_and(|r| r.revision == revision)
            {
                self.renderer_diagnostics().await;
                return true;
            }
        }
        false
    }

    async fn unlock_fixture(&self, password: &str) -> bool {
        for _ in 0..120 {
            if let Some(dialog) = self
                .window
                .visible_dialog()
                .and_downcast::<adw::AlertDialog>()
                && let Some(entry) = dialog.extra_child().and_downcast::<gtk::PasswordEntry>()
            {
                entry.set_text(password);
                return click_button(&dialog, "Unlock");
            }
            settle(50).await;
        }
        false
    }
    async fn wait_ready(&self) {
        for _ in 0..80 {
            settle(250).await;
            if !self.is_opening() && self.is_ready() {
                break;
            }
        }
    }
    async fn renderer_diagnostics(&self) -> Value {
        let view = match self.surface.borrow().as_ref() {
            Some(Surface::Reflow(r)) => Some(r.view.clone()),
            _ => None,
        };
        let mut latest = Value::Null;
        if let Some(view) = view {
            let expected = self.current.borrow().as_ref().map(|r| r.settings.mode);
            let expected = if expected == Some(Mode::Pages) {
                "pages"
            } else {
                "scroll"
            };
            let mut settled = 0;
            for _ in 0..100 {
                if let Ok(value) = view
                    .evaluate_javascript_future(
                        "JSON.stringify(globalThis.readeroDiagnostics?.())",
                        Some(reflow::WORLD),
                        None,
                    )
                    .await
                {
                    latest = serde_json::from_str(&value.to_str()).unwrap_or(Value::Null);
                    if latest["stable"] == true && latest["mode"] == expected {
                        settled += 1;
                        if settled >= 2 {
                            return latest;
                        }
                    } else {
                        settled = 0;
                    }
                }
                settle(80).await;
            }
        }
        latest
    }
    async fn snapshot(&self, path: &std::path::Path) {
        for _ in 0..3 {
            self.window.queue_draw();
            settle(60).await;
            let paintable = gtk::WidgetPaintable::new(Some(&self.window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                f64::from(self.window.width()),
                f64::from(self.window.height()),
            );
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), self.window.renderer()) {
                renderer
                    .render_texture(&node, None)
                    .save_to_png(path)
                    .expect("save screenshot");
                return;
            }
        }
        eprintln!("READERO_SMOKE screenshot unavailable: {}", path.display());
    }
}

fn write_json(path: &std::path::Path, value: &impl serde::Serialize) {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize smoke evidence"),
    )
    .expect("write smoke evidence");
}

fn click_button(widget: &impl IsA<gtk::Widget>, label: &str) -> bool {
    if let Some(button) = widget.as_ref().downcast_ref::<gtk::Button>()
        && (button.label().is_some_and(|text| text == label)
            || button.tooltip_text().is_some_and(|text| text == label))
    {
        button.emit_clicked();
        return true;
    }
    let mut child = widget.as_ref().first_child();
    while let Some(current) = child {
        if click_button(&current, label) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

fn has_label(widget: &impl IsA<gtk::Widget>, text: &str) -> bool {
    if let Some(label) = widget.as_ref().downcast_ref::<gtk::Label>()
        && label.is_mapped()
        && label.text().contains(text)
    {
        return true;
    }
    let mut child = widget.as_ref().first_child();
    while let Some(current) = child {
        if has_label(&current, text) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}
