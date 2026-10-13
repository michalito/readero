use super::*;

impl Shell {
    pub(in crate::desktop::shell) async fn prepare_notebook_idle(self: &Rc<Self>) {
        self.wait_notebook().await;
        if !self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        self.notebook
            .editor
            .buffer()
            .set_text(&"Question → café 👩‍🔬\n\nWorked example 1:\n    x = 2;\n\n".repeat(2000));
        self.seed_connections(1000);
        self.save();
        self.reading_state.save(None).await.unwrap();
    }

    fn seed_connections(self: &Rc<Self>, count: i32) {
        if self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .unwrap()
            .references
            .is_empty()
        {
            self.attach_passage();
        }
        self.notebook_metadata(|d| {
            let passage = d.references[0].id.clone();
            let stride = d.text.chars().count() as i32 / count;
            d.sections = (0..count)
                .map(|i| readero::notebook::Section {
                    id: format!("performance-{i}"),
                    start: i * stride,
                    end: i * stride + 12,
                    passages: vec![passage.clone()],
                })
                .collect();
            Ok(())
        });
    }

    pub(in crate::desktop::shell) async fn settled_connections(&self) {
        let (send, receive) = async_channel::bounded(1);
        glib::idle_add_local_full(super::super::notebook::decoration_priority(), move || {
            let _ = send.try_send(());
            glib::ControlFlow::Break
        });
        let _ = receive.recv().await;
    }

    pub(in crate::desktop::shell) async fn notebook_performance(self: &Rc<Self>) -> Value {
        self.wait_notebook().await;
        if !self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        self.window.set_default_size(1280, 820);
        settle(300).await;
        let mut cases = Vec::new();
        for (name, text) in [
            (
                "multiline_124kb",
                "Question → café 👩‍🔬\n\nWorked example 1:\n    x = 2;\n\n".repeat(2000),
            ),
            (
                "multiline_1mb",
                "A notebook paragraph → café with a worked example.\n".repeat(20000),
            ),
            (
                "paragraph_64kb",
                "A long wrapped paragraph → café with a worked example. ".repeat(1200),
            ),
            (
                "paragraph_256kb",
                "A long wrapped paragraph → café with a worked example. ".repeat(4800),
            ),
        ] {
            let buffer = self.notebook.editor.buffer();
            let start = Instant::now();
            buffer.set_text(&text);
            let replace_ms = start.elapsed().as_secs_f64() * 1000.0;
            settle(500).await;
            let adjustment = self.notebook.scroll.vadjustment();
            adjustment.set_value((adjustment.upper() - adjustment.page_size()) * 0.8);
            settle(100).await;
            let mut anchor_ms = Vec::new();
            let mut edit_ms = Vec::new();
            let mut cursor_ms = Vec::new();
            let mut queue_ms = Vec::new();
            let mut durable_ms = Vec::new();
            for _ in 0..10 {
                let start = Instant::now();
                let top = self.notebook.top();
                anchor_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                buffer.place_cursor(&buffer.iter_at_offset(top));
                cursor_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                buffer.insert_at_cursor("x");
                edit_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                self.queue_notebook();
                queue_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                self.reading_state.save(None).await.unwrap();
                durable_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                settle(30).await;
            }
            cases.push(json!({
                "case": name, "utf8_bytes": text.len(), "replace_ms": replace_ms,
                "anchor_ms": anchor_ms, "cursor_ms": cursor_ms, "edit_ms": edit_ms,
                "queue_ms": queue_ms, "durable_ms": durable_ms,
            }));
        }
        for (repetitions, connections) in [(2000, 100), (20000, 1000)] {
            let buffer = self.notebook.editor.buffer();
            let text = "Question → café 👩‍🔬

Worked example 1:
    x = 2;

"
            .repeat(repetitions);
            buffer.set_text(&text);
            self.seed_connections(connections);
            settle(500).await;
            let mut cursor_ms = Vec::new();
            let mut editing = Vec::new();
            let mut linking = Vec::new();
            for _ in 0..20 {
                let started = Instant::now();
                buffer.place_cursor(&buffer.iter_at_offset(5));
                cursor_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                let started = Instant::now();
                buffer.insert_at_cursor("x");
                self.settled_connections().await;
                editing.push(started.elapsed().as_secs_f64() * 1000.0);
                let started = Instant::now();
                self.notebook_metadata(|d| {
                    let id = d.sections[0].id.clone();
                    let passage = d.references[0].id.clone();
                    d.disconnect(&id, &passage);
                    Ok(())
                });
                self.notebook_undo(false);
                self.settled_connections().await;
                linking.push(started.elapsed().as_secs_f64() * 1000.0);
                settle(30).await;
            }
            cases.push(json!({"case":"connected_notebook", "utf8_bytes":text.len(), "connections":connections,
                "cursor_ms":cursor_ms, "edit_and_decoration_ms":editing, "disconnect_undo_and_decoration_ms":linking}));
        }
        json!({"scope": "Native synthetic notebook measurements; milliseconds; ten baseline samples and twenty connected samples; excludes frame presentation", "cases": cases})
    }

    pub(super) async fn check_notebook_restart(self: &Rc<Self>) -> Value {
        self.wait_notebook().await;
        let draft = self
            .notebook
            .draft
            .borrow()
            .clone()
            .expect("restarted notebook");
        let mut checks = json!({
            "notebook_process_restart_restores_text": draft.text == "Restarted notebook → café 👩‍🔬\nWorked example 1.\n",
            "notebook_process_restart_saves_selection": draft.cursor == 12 && draft.selection == 3,
            "notebook_process_restart_restores_references": draft.references.len() == 2 && draft.references[0].label == "Worked example 1",
        });
        self.toggle_notebook();
        settle(700).await;
        checks["notebook_process_restart_restores_selection"] = json!(
            self.notebook.editor.buffer().cursor_position() == 12
                && self
                    .notebook
                    .editor
                    .buffer()
                    .iter_at_mark(&self.notebook.editor.buffer().selection_bound())
                    .offset()
                    == 3
        );
        let target = draft.references[0].locator.clone();
        self.goto(Destination::Locator(target.clone()), true);
        settle(800).await;
        checks["notebook_process_restart_reference_navigates"] =
            json!(self.record_for_save().and_then(|r| r.locator) == Some(target));
        checks
    }

    pub(super) async fn check_notebook(self: &Rc<Self>, directory: &std::path::Path) -> Value {
        let mut checks = json!({});
        self.wait_notebook().await;
        let original = self.current.borrow().clone().expect("notebook document");
        if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
            let target = papers_document::Link::new(
                Some("Passage without an origin"),
                &papers_document::LinkAction::new_dest(&papers_document::LinkDest::new_page(0)),
            );
            // Papers can emit a missing origin during passage restoration.
            // Exercise the real connected signal, including Rust conversion.
            pdf.view
                .emit_by_name::<()>("handle-link", &[&target, &Option::<glib::Object>::None]);
            checks["notebook_pdf_link_accepts_missing_origin"] = json!(true);
        }
        if !self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        settle(800).await;
        for _ in 0..10 {
            let buffer = self.notebook.editor.buffer();
            buffer.set_text(&"A line of notebook text.\n".repeat(2000));
            settle(25).await;
            self.notebook
                .editor
                .scroll_to_iter(&mut buffer.end_iter(), 0.0, true, 0.0, 0.0);
            settle(30).await;
            self.activate_notebook(None);
            self.activate_notebook(Some(original.clone()));
            self.wait_notebook().await;
            if !self.notebook.panel.is_visible() {
                self.toggle_notebook();
            }
        }
        checks["notebook_reset_during_scroll_animation"] = json!(true);
        let buffer = self.notebook.editor.buffer();
        let body = "Question → café 👩‍🔬\n\nWorked example 1:\n    x = 2;\n\n".repeat(2000);
        buffer.begin_user_action();
        buffer.insert_at_cursor(&body);
        buffer.end_user_action();
        self.notebook_undo(false);
        checks["notebook_native_undo"] = json!(buffer.char_count() == 0);
        self.notebook_undo(true);
        checks["notebook_native_redo"] =
            json!(buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) == body);
        buffer.select_range(&buffer.iter_at_offset(51), &buffer.iter_at_offset(10));
        self.notebook.editor.grab_focus();
        self.button("next_button").emit_clicked();
        settle(800).await;
        checks["notebook_turn_keeps_focus_and_selection"] = json!(
            gtk::prelude::RootExt::focus(&self.window).as_ref()
                == Some(self.notebook.editor.upcast_ref())
                && !self.button("next_button").gets_focus_on_click()
                && buffer.cursor_position() == 51
                && buffer.iter_at_mark(&buffer.selection_bound()).offset() == 10
        );
        if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
            checks["notebook_pdf_turn_tracks_visible_page"] = json!(
                matches!((pdf.location(), pdf.stable_location()),
                    (Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }), Some(Locator { anchor: Anchor::Pdf { page: saved, .. }, .. }))
                    if page == saved)
            );
        }
        self.attach_passage();
        let reference = self.notebook.draft.borrow().as_ref().unwrap().references[0].clone();
        self.turn(true);
        settle(800).await;
        self.attach_passage();
        checks["notebook_references_are_snapshots"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .unwrap()
                .references
                .len()
                == 2
                && self.notebook.draft.borrow().as_ref().unwrap().references[0] == reference
        );
        let row = self
            .notebook
            .references
            .row_at_index(0)
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap();
        let name = row
            .first_child()
            .unwrap()
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap()
            .first_child()
            .unwrap()
            .downcast::<gtk::Entry>()
            .unwrap();
        name.set_text("Worked example 1");
        checks["notebook_reference_can_be_named"] = json!(
            self.notebook.draft.borrow().as_ref().unwrap().references[0].label
                == "Worked example 1"
        );
        let row = self
            .notebook
            .references
            .row_at_index(1)
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap();
        row.last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        checks["notebook_reference_removal_is_explicit"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .unwrap()
                .references
                .len()
                == 1
        );
        self.attach_passage();
        self.change_settings(|settings| {
            settings.mode = if settings.mode == Mode::Pages {
                Mode::Scroll
            } else {
                Mode::Pages
            };
            settings.font_size = 23.0;
        });
        settle(800).await;
        let before = self.record_for_save().and_then(|r| r.locator);
        self.goto(Destination::Locator(reference.locator.clone()), true);
        settle(900).await;
        checks["notebook_reference_returns_after_layout_change"] = json!(
            self.record_for_save().and_then(|r| r.locator) == Some(reference.locator.clone())
        );
        if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
            checks["notebook_pdf_reference_reaches_visible_page"] = json!(
                matches!((pdf.location(), &reference.locator.anchor),
                    (Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }), Anchor::Pdf { page: expected, .. })
                    if page == *expected)
            );
        }
        self.history(false);
        settle(800).await;
        checks["notebook_reference_has_back_history"] =
            json!(self.record_for_save().and_then(|r| r.locator) == before);
        self.show_search();
        self.entry().set_text("reading");
        settle(900).await;
        self.next_search_result(true);
        settle(400).await;
        let page_before_close = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => pdf.model.page(),
            _ => -1,
        };
        // Clearing via the search entry keeps keyboard focus in the sidebar;
        // deleting a programmatically focused result row is a different action.
        self.entry().grab_focus();
        self.entry().set_text("");
        self.search("");
        self.sidebar().set_reveal_child(false);
        checks["notebook_closing_sidebar_returns_focus"] = json!(
            gtk::prelude::RootExt::focus(&self.window).as_ref()
                == Some(self.button("sidebar_button").upcast_ref())
        );
        if page_before_close >= 0 {
            settle(350).await;
            if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
                checks["notebook_pdf_closing_search_keeps_visible_page"] = json!(
                    matches!((pdf.location(), pdf.stable_location()),
                        (Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }), Some(Locator { anchor: Anchor::Pdf { page: saved, .. }, .. }))
                        if page == page_before_close && saved == page_before_close)
                );
            }
        }
        checks["notebook_navigation_keeps_draft"] = json!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) == body
                && buffer.cursor_position() == 51
        );
        let pdf_at_start = matches!(self.surface.borrow().as_ref(),
            Some(Surface::Pdf(pdf)) if pdf.model.page() == 0);
        if pdf_at_start {
            self.turn(true);
            settle(400).await;
        }
        let page_before_focus = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => pdf.model.page(),
            _ => -1,
        };
        self.notebook.editor.grab_focus();
        self.toggle_notebook();
        checks["notebook_hiding_editor_returns_focus"] = json!(
            gtk::prelude::RootExt::focus(&self.window).as_ref()
                == Some(self.button("notebook_button").upcast_ref())
        );
        self.toggle_notebook();
        checks["notebook_hide_show_keeps_undo_and_selection"] = json!(
            self.notebook.history.borrow().can_undo()
                && buffer.cursor_position() == 51
                && buffer.iter_at_mark(&buffer.selection_bound()).offset() == 10
        );
        self.toggle_focus();
        checks["notebook_focus_hides_panel"] = json!(!self.notebook.panel.is_visible());
        checks["notebook_focus_mode_focuses_reader"] = json!(
            gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|widget| widget.is_ancestor(&self.content()))
        );
        self.toggle_focus();
        checks["notebook_focus_restores_panel"] = json!(self.notebook.panel.is_visible());
        self.window.set_default_size(760, 820);
        settle(700).await;
        checks["notebook_narrow_layout_stacks"] = json!(
            self.notebook.split.orientation() == gtk::Orientation::Vertical
                && self.content().height() >= 150
                && self.notebook.editor.height() >= 60
        );
        self.snapshot(&directory.join("notebook-narrow.png")).await;
        self.window.set_default_size(760, 480);
        settle(500).await;
        checks["notebook_short_window_keeps_both_surfaces_usable"] =
            json!(self.content().height() >= 180 && self.notebook.editor.height() >= 60);
        self.snapshot(&directory.join("notebook-short.png")).await;
        self.window.set_default_size(1280, 820);
        settle(700).await;
        checks["notebook_wide_layout_is_beside_reader"] =
            json!(self.notebook.split.orientation() == gtk::Orientation::Horizontal);
        self.notebook.scroll.vadjustment().set_value(440.0);
        settle(100).await;
        self.save();
        self.reading_state.save(None).await.expect("notebook flush");
        let saved = self
            .reading_state
            .notebook(original.clone())
            .await
            .expect("saved notebook");
        checks["notebook_long_text_and_context_saved"] = json!(
            saved.text == body
                && saved.cursor == 51
                && saved.selection == 10
                && saved.top > 0
                && saved.references.len() == 2
        );
        if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
            checks["notebook_pdf_focus_and_resize_keep_visible_page"] =
                json!(matches!(pdf.location(),
                    Some(Locator { anchor: Anchor::Pdf { page, .. }, .. })
                    if page == page_before_focus));
            checks["notebook_pdf_current_passage_matches_visible_page"] = json!(
                matches!((pdf.location(), pdf.stable_location()),
                    (Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }), Some(Locator { anchor: Anchor::Pdf { page: saved, .. }, .. }))
                    if page == saved && page == pdf.model.page())
            );
            if checks["notebook_pdf_focus_and_resize_keep_visible_page"] != true
                || checks["notebook_pdf_current_passage_matches_visible_page"] != true
            {
                eprintln!(
                    "PDF notebook position mismatch: before={page_before_focus}, model={}, visible={:?}, saved={:?}",
                    pdf.model.page(),
                    pdf.location(),
                    pdf.stable_location()
                );
            }
        }
        self.snapshot(&directory.join("notebook-wide.png")).await;
        self.home();
        self.open(original.path.clone());
        if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
            self.unlock_fixture(&password).await;
        }
        self.wait_ready().await;
        self.wait_notebook().await;
        settle(700).await;
        checks["notebook_reopen_restores_editing_context"] = json!(
            self.notebook.editor.buffer().text(
                &self.notebook.editor.buffer().start_iter(),
                &self.notebook.editor.buffer().end_iter(),
                true
            ) == body
                && self.notebook.editor.buffer().cursor_position() == 51
                && self.notebook.top() == saved.top
                && self.notebook.panel.is_visible()
        );
        checks["notebook_reopen_restores_selection"] = json!(
            self.notebook
                .editor
                .buffer()
                .iter_at_mark(&self.notebook.editor.buffer().selection_bound())
                .offset()
                == saved.selection
        );
        if checks["notebook_reopen_restores_editing_context"] != true {
            eprintln!(
                "NOTEBOOK_REOPEN cursor={} selection={} top={} expected_top={} visible={} chars={}",
                self.notebook.editor.buffer().cursor_position(),
                self.notebook
                    .editor
                    .buffer()
                    .iter_at_mark(&self.notebook.editor.buffer().selection_bound())
                    .offset(),
                self.notebook.top(),
                saved.top,
                self.notebook.panel.is_visible(),
                self.notebook.editor.buffer().char_count()
            );
        }
        if original.format == Format::Markdown {
            let buffer_before = self.notebook.editor.buffer();
            self.reload(original.path.clone());
            self.wait_ready().await;
            settle(500).await;
            checks["notebook_reload_retains_live_buffer"] = json!(
                self.notebook.editor.buffer() == buffer_before
                    && buffer_before.cursor_position() == 51
                    && buffer_before.char_count() == body.chars().count() as i32
            );
        }
        let data = std::env::var_os("READERO_DATA_DIR").expect("isolated smoke state");
        let database =
            rusqlite::Connection::open(PathBuf::from(data).join("reading.sqlite3")).unwrap();
        database
            .execute_batch(
                "CREATE TRIGGER notebook_failure BEFORE INSERT ON notebooks
            BEGIN SELECT RAISE(FAIL, 'notebook test failure'); END;",
            )
            .unwrap();
        let buffer = self.notebook.editor.buffer();
        buffer.place_cursor(&buffer.end_iter());
        buffer.insert_at_cursor("Retained after a failed save.");
        self.save();
        let _ = self.reading_state.save(None).await;
        settle(100).await;
        checks["notebook_save_failure_visible"] = json!(
            self.object::<adw::Banner>("save_banner").is_revealed()
                && self.notebook.retry.is_visible()
                && self.notebook.editor.is_editable()
        );
        let other = directory.join("notebook-other.md");
        std::fs::write(&other, "# Other notebook\n\nIndependent document.").unwrap();
        self.open(other);
        self.wait_ready().await;
        self.wait_notebook().await;
        checks["notebook_switch_isolates_documents"] =
            json!(self.notebook.editor.buffer().char_count() == 0);
        self.open(original.path.clone());
        if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
            self.unlock_fixture(&password).await;
        }
        self.wait_ready().await;
        self.wait_notebook().await;
        checks["notebook_unsaved_draft_survives_switch"] = json!(
            self.notebook
                .editor
                .buffer()
                .text(
                    &self.notebook.editor.buffer().start_iter(),
                    &self.notebook.editor.buffer().end_iter(),
                    true
                )
                .ends_with("Retained after a failed save.")
        );
        self.close();
        let mut kept_open = false;
        for _ in 0..100 {
            settle(50).await;
            if let Some(dialog) = self
                .window
                .visible_dialog()
                .and_downcast::<adw::AlertDialog>()
            {
                checks["notebook_failed_close_explains_draft_loss"] = json!(
                    dialog.body().contains("notebook") && !self.notebook.panel.is_sensitive()
                );
                kept_open = click_button(&dialog, "Keep open");
                break;
            }
        }
        checks["notebook_failed_close_keeps_editor_and_draft"] = json!(
            kept_open
                && self.notebook.editor.is_editable()
                && self
                    .notebook
                    .editor
                    .buffer()
                    .text(
                        &self.notebook.editor.buffer().start_iter(),
                        &self.notebook.editor.buffer().end_iter(),
                        true
                    )
                    .ends_with("Retained after a failed save.")
        );
        database
            .execute_batch("DROP TRIGGER notebook_failure")
            .unwrap();
        self.save();
        self.reading_state
            .save(None)
            .await
            .expect("notebook recovery");
        settle(100).await;
        checks["notebook_retry_clears_failure"] = json!(
            !self.notebook.retry.is_visible()
                && !self.object::<adw::Banner>("save_banner").is_revealed()
        );
        let saved = self.reading_state.notebook(original.clone()).await.unwrap();
        let json = serde_json::to_string(&saved).unwrap();
        self.home();
        self.reading_state.save(None).await.unwrap();
        database
            .execute(
                "UPDATE notebooks SET payload='unreadable' WHERE document_id=?1",
                [&original.id],
            )
            .unwrap();
        self.open(original.path.clone());
        if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
            self.unlock_fixture(&password).await;
        }
        self.wait_ready().await;
        settle(300).await;
        self.toggle_notebook();
        checks["notebook_failed_load_is_not_editable_empty_text"] = json!(
            self.notebook.draft.borrow().is_none()
                && !self.notebook.editor.is_editable()
                && self.notebook.retry.is_visible()
        );
        database
            .execute(
                "UPDATE notebooks SET payload=?1 WHERE document_id=?2",
                rusqlite::params![json, original.id],
            )
            .unwrap();
        self.notebook.retry.emit_clicked();
        self.wait_notebook().await;
        checks["notebook_load_retry_recovers_original"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.text == saved.text)
        );
        // A soft-wrapped paragraph has only one logical line. Restore the
        // visible passage within it, rather than jumping to the paragraph start.
        if !self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        self.notebook
            .editor
            .buffer()
            .set_text(&"A long wrapped paragraph → café with a worked example. ".repeat(1000));
        settle(300).await;
        self.notebook.scroll.vadjustment().set_value(440.0);
        settle(100).await;
        self.save();
        self.reading_state.save(None).await.unwrap();
        let wrapped = self.reading_state.notebook(original.clone()).await.unwrap();
        checks["notebook_wrapped_paragraph_saves_visible_passage"] = json!(wrapped.top > 0);
        self.home();
        self.open(original.path.clone());
        if let Ok(password) = std::env::var("READERO_SMOKE_PASSWORD") {
            self.unlock_fixture(&password).await;
        }
        self.wait_ready().await;
        self.wait_notebook().await;
        settle(500).await;
        checks["notebook_wrapped_paragraph_restores_visible_passage"] =
            json!((self.notebook.scroll.vadjustment().value() - 440.0).abs() < 30.0);

        for (name, paragraph) in [
            ("rtl", "דוגמה ארוכה של פסקה במחברת עם מחשבות והערות "),
            ("indented", "    A paragraph with spaces, café, and 👩‍🔬. "),
        ] {
            self.notebook
                .editor
                .buffer()
                .set_text(&paragraph.repeat(1000));
            settle(300).await;
            self.notebook.scroll.vadjustment().set_value(440.0);
            settle(100).await;
            let top = self.notebook.top();
            let y = self
                .notebook
                .editor
                .iter_location(&self.notebook.editor.buffer().iter_at_offset(top))
                .y();
            checks[format!("notebook_{name}_wrapped_anchor_is_visible")] =
                json!(top > 0 && (y - 440).abs() < 30);
        }
        let short_end = format!(
            "{}\nx\n{}",
            "דוגמה ארוכה במחברת ".repeat(1000),
            "\n".repeat(100)
        );
        let buffer = self.notebook.editor.buffer();
        buffer.set_text(&short_end);
        settle(300).await;
        let mut end = buffer.start_iter();
        end.forward_to_line_end();
        end.backward_char();
        let y = self.notebook.editor.iter_location(&end).y();
        self.notebook
            .scroll
            .vadjustment()
            .set_value(f64::from(y + self.notebook.editor.top_margin()));
        settle(100).await;
        let top = self.notebook.top();
        checks["notebook_short_rtl_wrap_keeps_paragraph_end"] = json!(
            top > 0
                && (self
                    .notebook
                    .editor
                    .iter_location(&buffer.iter_at_offset(top))
                    .y()
                    - y)
                    .abs()
                    < 30
        );
        self.save();
        self.reading_state.save(None).await.unwrap();

        // Queue a real load, then express visibility intent before its UI reply.
        // This is deterministic even when the database worker is fast.
        self.activate_notebook(None);
        self.activate_notebook(Some(original.clone()));
        self.toggle_notebook();
        self.toggle_notebook();
        self.wait_notebook().await;
        checks["notebook_hide_during_load_wins"] = json!(
            !self.notebook.panel.is_visible()
                && !self.notebook.draft.borrow().as_ref().unwrap().visible
        );
        self.save();
        self.reading_state.save(None).await.unwrap();
        checks["notebook_hide_during_load_is_saved"] = json!(
            !self
                .reading_state
                .notebook(original.clone())
                .await
                .unwrap()
                .visible
        );
        if !self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        // Input arriving before deferred restoration must win over saved context.
        let buffer = self.notebook.editor.buffer();
        buffer.place_cursor(&buffer.end_iter());
        buffer.insert_at_cursor("New input wins.");
        let cursor = buffer.cursor_position();
        settle(150).await;
        checks["notebook_restore_yields_to_new_input"] = json!(
            buffer.cursor_position() == cursor
                && buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .ends_with("New input wins.")
        );
        // Leave a small, known durable notebook for restart checks and ordinary
        // regression flows, rather than making every later layout use long text.
        self.notebook
            .editor
            .buffer()
            .set_text("Restarted notebook → café 👩‍🔬\nWorked example 1.\n");
        self.notebook.editor.buffer().select_range(
            &self.notebook.editor.buffer().iter_at_offset(12),
            &self.notebook.editor.buffer().iter_at_offset(3),
        );
        self.save();
        self.reading_state.save(None).await.unwrap();
        let fresh = ReadingState::start(directory.join("state/reading.sqlite3"));
        checks["notebook_fresh_worker_restores_text_and_links"] =
            json!(fresh.notebook(original.clone()).await.is_ok_and(|d| {
                d.text.starts_with("Restarted notebook")
                    && d.cursor == 12
                    && d.selection == 3
                    && d.references.len() == 2
            }));
        if self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        self.change_settings(|settings| *settings = original.settings.clone());
        if let Some(locator) = original.locator {
            self.goto(Destination::Locator(locator), false);
        }
        self.window.set_default_size(1080, 820);
        settle(600).await;
        checks
    }

    pub(super) async fn wait_notebook(&self) {
        for _ in 0..100 {
            if self.notebook.draft.borrow().is_some() {
                return;
            }
            settle(50).await;
        }
    }
}
