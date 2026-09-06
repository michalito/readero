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
                let ready = shell.gate.borrow().accepts(shell.generation.get());
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
                    report["regressions"] = shell.check_regressions().await;
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
                        report["bookmark_count"] = json!(shell.worker.call(move |store| Ok(store.bookmarks(&id)?.len())).await.unwrap_or(0));
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
    async fn check_recovery(self: &Rc<Self>, directory: &std::path::Path) -> Value {
        let mut checks = json!({});
        let Some(mut record) = self.record_for_save() else {
            return checks;
        };
        let id = record.id.clone();
        self.home();
        // Drain the save-before-home and recent-load requests.
        self.worker.call(|_| Ok(())).await.expect("state worker");
        settle(200).await;
        record.path = directory.join("missing-recent.md");
        self.render_home(&[record.clone()]);
        checks["continue_card_can_remove_missing_document"] =
            json!(click_button(&self.content(), "Remove from recents"));
        settle(250).await;
        let saved_id = id.clone();
        let (hidden, bookmarks) = self
            .worker
            .call(move |store| {
                Ok((
                    !store.recents()?.iter().any(|record| record.id == saved_id),
                    store.bookmarks(&saved_id)?.len(),
                ))
            })
            .await
            .expect("query removed recent");
        checks["continue_removal_hides_recent"] = json!(hidden);
        checks["continue_removal_keeps_bookmarks"] = json!(bookmarks == 1);

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
                .worker
                .call(move |store| store.document(&source))
                .await
                .expect("recovered state");
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
            let count = self
                .worker
                .call(move |store| Ok(store.bookmarks(&id)?.len()))
                .await
                .expect("recovered bookmarks");
            // Bookmark work is enqueued by its async UI task.
            settle(100).await;
            let id = saved.id;
            let count = if count == 1 {
                self.worker
                    .call(move |store| Ok(store.bookmarks(&id)?.len()))
                    .await
                    .expect("new bookmark")
            } else {
                count
            };
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
                generation: self.generation.get(),
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
            generation: self.generation.get(),
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
        let mut checks = json!({});
        for atomic in [false, true] {
            let generation = self.generation.get();
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
                self.gate.borrow().accepts(generation)
                    && self.content().visible_child() == surface
                    && self.record_for_save().and_then(|r| r.locator) == original.locator
            );
            checks[format!("blank_reload_keeps_watcher_{atomic}")] = json!(
                self.monitor
                    .borrow()
                    .as_ref()
                    .is_some_and(|monitor| !monitor.is_cancelled())
            );
            std::fs::write(&original.path, &source).expect("correct blank fixture");
            for _ in 0..100 {
                settle(100).await;
                if self.generation.get() != generation {
                    break;
                }
            }
            self.wait_ready().await;
            self.renderer_diagnostics().await;
            checks[format!("corrected_reload_recovers_{atomic}")] = json!(
                self.generation.get() != generation
                    && self.gate.borrow().accepts(self.generation.get())
                    && self.record_for_save().and_then(|r| r.locator) == original.locator
            );
        }
        for atomic in [false, true] {
            let generation = self.generation.get();
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
                if self.generation.get() != generation {
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
            checks[format!("{prefix}_detected")] = json!(self.generation.get() != generation);
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
            if self.gate.borrow().accepts(self.generation.get()) {
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
