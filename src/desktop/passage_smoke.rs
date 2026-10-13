//! Real renderer selections and native notebook actions, in isolated test data.
use super::*;

const NOTES: &str = "My explanation → café 👩‍🔬.\n\nAnother thought.\n";

impl Shell {
    async fn passage_script(&self, script: &str) -> Value {
        let view = match self.surface.borrow().as_ref() {
            Some(Surface::Reflow(reader)) => Some(reader.view.clone()),
            _ => None,
        };
        if let Some(view) = view {
            return view
                .evaluate_javascript_future(script, Some(reflow::WORLD), None)
                .await
                .ok()
                .and_then(|v| serde_json::from_str(&v.to_str()).ok())
                .unwrap_or(Value::Null);
        }
        Value::Null
    }

    async fn select_source_fixture(&self) -> String {
        let selected = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => {
                pdf.view.select_all();
                Some(pdf.view.selected_text().unwrap_or_default().to_string())
            }
            _ => None,
        };
        if let Some(selected) = selected {
            return selected;
        }
        self.passage_script(r#"JSON.stringify((() => {
            const docs = document.querySelector('foliate-paginator')?.getContents().map(c => c.doc) ?? Array.from(window.frames).map(f => f.document);
            const doc = docs.find(d => d.querySelector('p'));
            const range = doc.createRange();
            range.selectNodeContents(doc.querySelector('p'));
            const selection = doc.getSelection();
            selection.removeAllRanges(); selection.addRange(range);
            return range.toString();
        })())"#).await.as_str().unwrap_or_default().to_owned()
    }

    async fn passage_is_highlighted(&self) -> bool {
        let pdf = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => Some(pdf.passage_highlighted()),
            _ => None,
        };
        if let Some(highlighted) = pdf {
            return highlighted;
        }
        self.passage_script("JSON.stringify((document.querySelector('foliate-paginator')?.getContents().map(c => c.doc.defaultView) ?? Array.from(window.frames)).some(f => f.CSS?.highlights?.has('readero-passage')))").await == true
    }

    pub(super) async fn check_passages(
        self: &Rc<Self>,
        directory: &std::path::Path,
        restart: bool,
    ) -> Value {
        self.wait_notebook().await;
        if !self.notebook.panel.is_visible() {
            self.toggle_notebook();
        }
        settle(650).await;
        let mut checks = json!({});
        if restart {
            let draft = self.notebook.draft.borrow().clone().unwrap();
            checks["passage_restart_keeps_text_quote_and_connection"] = json!(
                draft.text == NOTES
                    && draft.references.len() == 1
                    && draft.references[0].passage.is_some()
                    && draft.sections.len() == 1
                    && draft.sections[0].passages == [draft.references[0].id.clone()]
            );
            if let Some(reference) = draft.references.first() {
                self.follow_saved_passage(&reference.id);
                settle(1200).await;
                checks["passage_restart_navigates_and_highlights"] = json!(
                    self.passage_is_highlighted().await && self.window.visible_dialog().is_none()
                );
            }
            return checks;
        }
        if matches!(self.surface.borrow().as_ref(), Some(Surface::Reflow(_))) {
            self.passage_script(include_str!("../../tests/passages.js"))
                .await;
            for _ in 0..100 {
                let result = self
                    .passage_script("JSON.stringify(globalThis.readeroPassageChecks ?? null)")
                    .await;
                if let Some(results) = result.as_object() {
                    checks["range_resolution_checks_completed"] =
                        json!(!results.contains_key("error"));
                    checks.as_object_mut().unwrap().extend(results.clone());
                    break;
                }
                settle(50).await;
            }
        }
        let buffer = self.notebook.editor.buffer();
        buffer.set_text(NOTES);
        let expected = self.select_source_fixture().await;
        settle(250).await;
        self.save_selected_passage();
        settle(650).await;
        let captured = self.notebook.draft.borrow().clone().unwrap();
        checks["selected_quote_captured_exactly"] = json!(
            !expected.is_empty()
                && captured.references.len() == 1
                && captured.references[0]
                    .passage
                    .as_ref()
                    .is_some_and(|p| p.quote == expected)
        );
        checks["capture_keeps_notebook_text"] = json!(captured.text == NOTES);
        let Some(reference) = captured.references.first().cloned() else {
            self.snapshot(&directory.join("capture-failed.png")).await;
            return checks;
        };
        checks["saved_source_location_is_visible"] = json!(has_label(
            &self.notebook.references,
            &reference.locator.label()
        ));
        // Put the original passage outside the visible list viewport. Saving
        // it again must reveal that exact row without disturbing the notes.
        self.notebook_metadata(|d| {
            for index in 0..20 {
                d.references.push(readero::notebook::Reference {
                    id: format!("scroll-fixture-{index}"),
                    label: format!("Scroll fixture {index}"),
                    locator: reference.locator.clone(),
                    passage: None,
                });
            }
            Ok(())
        });
        self.render_references();
        settle(200).await;
        let list_adjustment = self
            .notebook
            .references
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("saved passage scroll area")
            .vadjustment();
        list_adjustment.set_value(list_adjustment.upper() - list_adjustment.page_size());
        checks["duplicate_fixture_starts_offscreen"] = json!(list_adjustment.value() > 0.0);
        buffer.select_range(&buffer.iter_at_offset(5), &buffer.iter_at_offset(1));
        self.select_source_fixture().await;
        settle(200).await;
        // X11's primary selection can collapse the editor selection when the
        // source acquires it. Capture must preserve the state at invocation.
        let notebook_selection = (
            buffer.cursor_position(),
            buffer.iter_at_mark(&buffer.selection_bound()).offset(),
        );
        self.save_selected_passage();
        settle(400).await;
        checks["identical_selection_deduplicated"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .unwrap()
                .references
                .len()
                == 21
        );
        checks["duplicate_capture_selects_existing_passage"] = json!(
            self.notebook
                .references
                .selected_row()
                .is_some_and(|row| row.index() == 0)
        );
        checks["duplicate_capture_reveals_existing_passage"] = json!(
            self.notebook
                .references
                .row_at_index(0)
                .and_then(|row| row.compute_bounds(&self.notebook.references))
                .is_some_and(|bounds| f64::from(bounds.y()) >= list_adjustment.value()
                    && f64::from(bounds.y() + bounds.height())
                        <= list_adjustment.value() + list_adjustment.page_size())
        );
        checks["duplicate_capture_preserves_notebook_selection"] = json!(
            (
                buffer.cursor_position(),
                buffer.iter_at_mark(&buffer.selection_bound()).offset(),
            ) == notebook_selection
        );
        checks["location_only_passage_is_distinguished"] =
            json!(has_label(&self.notebook.references, "no quotation"));
        self.notebook_metadata(|d| {
            d.references.retain(|r| r.id == reference.id);
            Ok(())
        });
        self.render_references();
        let end = NOTES
            .find('\n')
            .map(|at| NOTES[..at].chars().count() as i32)
            .unwrap();
        self.begin_passage_connection(&reference.id);
        self.settled_connections().await;
        checks["connection_controls_do_not_wait_for_background_layout"] =
            json!(has_label(&self.notebook.panel, "Connect notes"));
        buffer.select_range(&buffer.start_iter(), &buffer.iter_at_offset(end));
        self.connect_notebook_selection();
        settle(250).await;
        let section = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .unwrap()
            .sections
            .first()
            .cloned();
        checks["selected_notes_connected"] =
            json!(section.as_ref().is_some_and(|s| s.start == 0
                && s.end == end
                && s.passages == [reference.id.clone()]));
        let Some(section) = section else {
            return checks;
        };
        let expected_source = format!(
            "Open connected passage: {} — {}",
            reference.label,
            reference.locator.label()
        );
        checks["section_source_names_passage_and_location"] =
            json!(click_button(&self.notebook.panel, &expected_source));
        settle(800).await;
        self.snapshot(&directory.join("connected.png")).await;
        buffer.begin_user_action();
        buffer.insert(&mut buffer.start_iter(), "Before. ");
        buffer.end_user_action();
        checks["insertion_before_moves_section"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().sections[0].start == 8);
        self.notebook_undo(false);
        checks["undo_keeps_text_and_range_together"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.text == NOTES && d.sections == [section.clone()])
        );
        buffer.begin_user_action();
        buffer.delete(&mut buffer.start_iter(), &mut buffer.iter_at_offset(end));
        buffer.insert(&mut buffer.start_iter(), "Rewritten thought.");
        buffer.end_user_action();
        checks["replacement_preserves_connection"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.sections.len() == 1
                    && d.sections[0].end == 18
                    && d.sections[0].passages == section.passages)
        );
        self.notebook_undo(false);
        self.notebook_undo(true);
        checks["redo_preserves_replacement_connection"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.text.starts_with("Rewritten thought.") && d.sections.len() == 1)
        );
        self.notebook_undo(false);
        buffer.begin_user_action();
        buffer.delete(&mut buffer.start_iter(), &mut buffer.iter_at_offset(end));
        buffer.end_user_action();
        checks["deleting_linked_text_keeps_saved_passage"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.sections.is_empty() && d.references.len() == 1)
        );
        self.notebook_undo(false);
        checks["undo_restores_deleted_section"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.text == NOTES && d.sections == [section.clone()])
        );
        self.turn(true);
        settle(600).await;
        let before = self.record_for_save().and_then(|r| r.locator);
        self.follow_notebook_section(0);
        settle(1200).await;
        checks["connected_section_navigates_and_highlights"] =
            json!(self.passage_is_highlighted().await && self.window.visible_dialog().is_none());
        self.snapshot(&directory.join("passage-opened.png")).await;
        self.history(false);
        settle(650).await;
        checks["back_restores_prior_reading_position"] =
            json!(self.record_for_save().and_then(|r| r.locator) == before);
        checks["navigation_clears_passage_highlight"] = json!(!self.passage_is_highlighted().await);
        self.follow_saved_passage(&reference.id);
        self.turn(true);
        settle(900).await;
        checks["new_navigation_cancels_pending_passage"] =
            json!(!self.passage_is_highlighted().await);
        if matches!(self.surface.borrow().as_ref(), Some(Surface::Reflow(_))) {
            let mode = self.current.borrow().as_ref().unwrap().settings.mode;
            self.change_settings(|settings| settings.mode = Mode::Pages);
            settle(500).await;
            self.follow_saved_passage(&reference.id);
            settle(800).await;
            self.passage_script(
                r#"JSON.stringify((() => {
                const doc = document.querySelector('foliate-paginator').getContents()[0].doc;
                doc.dispatchEvent(new doc.defaultView.KeyboardEvent('keydown', {key: 'PageDown'}));
                return true;
            })())"#,
            )
            .await;
            settle(400).await;
            checks["source_keyboard_navigation_clears_highlight"] =
                json!(!self.passage_is_highlighted().await);
            self.change_settings(|settings| settings.mode = mode);
            settle(500).await;
        }
        let pdf = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => Some(Rc::clone(pdf)),
            _ => None,
        };
        if let Some(pdf) = pdf {
            let settings = self.current.borrow().as_ref().unwrap().settings.clone();
            self.change_settings(|settings| {
                settings.mode = Mode::Pages;
                settings.pdf_sizing = "page".into();
            });
            self.follow_saved_passage(&reference.id);
            settle(800).await;
            let initial_page = pdf.model.page();
            pdf.next();
            let turned_page = pdf.model.page();
            checks["pdf_immediate_save_keeps_announced_page"] = json!(
                turned_page != initial_page
                    && matches!(self.record_for_save().and_then(|r| r.locator),
                        Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }) if page == turned_page)
            );
            self.bookmark();
            pdf.next();
            let latest_page = pdf.model.page();
            self.change_settings(|settings| settings.pdf_sizing = "width".into());
            checks["pdf_immediate_settings_keeps_latest_page"] = json!(
                latest_page != turned_page
                    && matches!(pdf.stable_location(),
                        Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }) if page == latest_page)
            );
            settle(500).await;
            checks["pdf_deferred_samples_do_not_restore_previous_page"] = json!(
                pdf.model.page() == latest_page
                    && matches!(pdf.stable_location(),
                        Some(Locator { anchor: Anchor::Pdf { page, .. }, .. }) if page == latest_page)
            );
            let document_id = self.current.borrow().as_ref().unwrap().id.clone();
            let bookmarks = self.reading_state.bookmarks(document_id).await.unwrap();
            checks["pdf_immediate_bookmark_persists_announced_page"] = json!(
                bookmarks.iter().any(|bookmark|
                    matches!(bookmark.locator.anchor, Anchor::Pdf { page, .. } if page == turned_page))
            );
            for bookmark in bookmarks {
                self.reading_state
                    .remove_bookmark(bookmark.id)
                    .await
                    .unwrap();
            }
            self.follow_saved_passage(&reference.id);
            settle(800).await;
            pdf.view.select_all();
            settle(100).await;
            let position = (pdf.model.page(), pdf.scroll.vadjustment().value());
            let (serial, available, sensitive, _) = self.notebook.capture_probe();
            checks["pdf_keyboard_fixture_has_selection"] =
                json!(pdf.view.has_selection() && available && sensitive);
            // This is the Papers action used by PageDown/PageUp. It does not
            // emit a wheel event or necessarily change the native selection.
            pdf.view
                .emit_scroll(gtk::ScrollType::PageForward, gtk::Orientation::Vertical);
            settle(400).await;
            checks["pdf_keyboard_action_moves_reader"] =
                json!((pdf.model.page(), pdf.scroll.vadjustment().value()) != position);
            let (new_serial, available, sensitive, _) = self.notebook.capture_probe();
            checks["pdf_keyboard_navigation_invalidates_selection"] =
                json!(new_serial > serial && !available && !sensitive);
            self.save_selected_passage();
            checks["pdf_keyboard_navigation_prevents_stale_capture"] =
                json!(!self.notebook.capture_probe().3);
            settle(200).await;
            self.follow_saved_passage(&reference.id);
            settle(800).await;
            checks["pdf_programmatic_passage_restore_keeps_highlight"] =
                json!(pdf.passage_highlighted());
            pdf.view
                .emit_scroll(gtk::ScrollType::PageForward, gtk::Orientation::Vertical);
            settle(250).await;
            checks["pdf_keyboard_navigation_clears_highlight"] = json!(!pdf.passage_highlighted());
            pdf.view
                .emit_scroll(gtk::ScrollType::PageBackward, gtk::Orientation::Vertical);
            settle(250).await;
            checks["pdf_keyboard_return_does_not_restore_old_highlight"] =
                json!(!pdf.passage_highlighted());
            self.follow_saved_passage(&reference.id);
            pdf.view
                .emit_scroll(gtk::ScrollType::PageForward, gtk::Orientation::Vertical);
            settle(800).await;
            checks["pdf_keyboard_navigation_cancels_pending_passage"] =
                json!(!pdf.passage_highlighted());
            self.change_settings(|current| *current = settings);
            settle(500).await;
        }
        self.attach_passage();
        let second = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .unwrap()
            .references
            .last()
            .unwrap()
            .id
            .clone();
        buffer.select_range(&buffer.start_iter(), &buffer.iter_at_offset(end));
        self.connect_notebook_selection();
        settle(150).await;
        let picker = self
            .window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>();
        checks["notes_first_opens_passage_picker"] = json!(
            picker
                .as_ref()
                .is_some_and(|d| d.heading().as_deref() == Some("Connect a saved passage"))
        );
        if let Some(picker) = picker {
            let scroll = picker
                .extra_child()
                .and_downcast::<gtk::ScrolledWindow>()
                .unwrap();
            // GtkScrolledWindow wraps non-scrollable content in a viewport.
            let mut child = scroll.child().unwrap();
            if let Some(viewport) = child.downcast_ref::<gtk::Viewport>() {
                child = viewport.child().unwrap();
            }
            if let Some(button) = child.last_child().and_downcast::<gtk::Button>() {
                button.emit_clicked();
            }
        }
        settle(200).await;
        checks["one_section_accepts_multiple_passages"] = json!(
            self.notebook.draft.borrow().as_ref().unwrap().sections[0]
                .passages
                .len()
                == 2
        );
        self.follow_notebook_section(0);
        settle(150).await;
        let picker = self
            .window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>();
        checks["multiple_connections_offer_choice"] = json!(
            picker
                .as_ref()
                .is_some_and(|d| d.heading().as_deref() == Some("Open connected passage"))
        );
        if let Some(picker) = picker {
            click_button(&picker, "Cancel");
        }
        settle(200).await;
        self.notebook_metadata(|d| {
            d.sections.clear();
            d.connect(0, 2, &reference.id)?;
            d.connect(3, 5, &second)
        });
        settle(200).await;
        checks["same_line_sections_share_passage_chooser"] = json!(click_button(
            &self.notebook.panel,
            "Choose from 2 connected passages"
        ));
        settle(150).await;
        let picker = self
            .window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>();
        checks["same_line_chooser_opens"] = json!(
            picker
                .as_ref()
                .is_some_and(|d| d.heading().as_deref() == Some("Open connected passage"))
        );
        if let Some(picker) = picker {
            click_button(&picker, "Cancel");
        }
        self.notebook_undo(false);
        settle(200).await;
        self.delete_saved_passage(&second);
        settle(150).await;
        let deletion = self
            .window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>();
        checks["connected_passage_deletion_requires_explicit_choice"] = json!(
            deletion
                .as_ref()
                .is_some_and(|d| d.body().contains("1 notebook"))
        );
        if let Some(deletion) = deletion {
            click_button(&deletion, "Delete and disconnect");
        }
        settle(200).await;
        checks["deleting_passage_preserves_prose_and_other_connections"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.text == NOTES
                    && d.references.len() == 1
                    && d.sections[0].passages == [reference.id.clone()])
        );
        self.notebook_metadata(|d| {
            let passage = d.references[0].passage.as_mut().unwrap();
            passage
                .quote
                .replace_range(..passage.quote.chars().next().unwrap().len_utf8(), "§");
            if let readero::passage::Target::Pdf { fragments } = &mut passage.target {
                let quote = &mut fragments[0].quote;
                quote.replace_range(..quote.chars().next().unwrap().len_utf8(), "§");
            }
            Ok(())
        });
        self.follow_saved_passage(&reference.id);
        settle(800).await;
        let missing = self
            .window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>();
        checks["missing_quote_keeps_connections_and_offers_relink"] = json!(
            missing
                .as_ref()
                .is_some_and(|d| d.heading().as_deref() == Some("Passage needs relinking"))
                && self
                    .notebook
                    .draft
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .sections
                    .len()
                    == 1
        );
        if let Some(missing) = missing {
            click_button(&missing, "Relink to selected text");
        }
        settle(250).await;
        self.select_source_fixture().await;
        settle(200).await;
        self.save_selected_passage();
        settle(400).await;
        let relink = self
            .window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>();
        checks["relink_previews_changed_quotation"] =
            json!(relink.as_ref().is_some_and(|d| d.heading().as_deref()
                == Some("Replace the saved quotation?")
                && d.body().contains("Saved:")
                && d.body().contains("Selected:")));
        if let Some(relink) = relink {
            click_button(&relink, "Replace passage with this selection");
        }
        settle(200).await;
        checks["explicit_relink_preserves_identity_and_connections"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.references[0] == reference
                    && d.sections[0].passages == [reference.id.clone()])
        );
        self.notebook_undo(false);
        checks["relink_is_undoable"] = json!(
            self.notebook.draft.borrow().as_ref().unwrap().references[0]
                .passage
                .as_ref()
                .unwrap()
                .quote
                .starts_with('§')
        );
        self.notebook_undo(true);
        self.window.set_default_size(760, 480);
        settle(650).await;
        checks["connected_notebook_fits_small_window"] =
            json!(self.content().height() >= 180 && self.notebook.editor.height() >= 60);
        self.snapshot(&directory.join("connected-short.png")).await;
        self.window.set_default_size(1080, 820);
        self.notebook_metadata(|d| {
            d.disconnect(&section.id, &reference.id);
            Ok(())
        });
        checks["disconnect_preserves_text_and_passage"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.text == NOTES
                    && d.sections.is_empty()
                    && d.references.len() == 1)
        );
        self.notebook_undo(false);
        checks["undo_restores_connection"] = json!(
            self.notebook
                .draft
                .borrow()
                .as_ref()
                .is_some_and(|d| d.sections == [section])
        );
        if matches!(self.surface.borrow().as_ref(), Some(Surface::Pdf(_))) {
            let settings = self.current.borrow().as_ref().unwrap().settings.clone();
            for rotation in [0, 90, 180, 270] {
                self.change_settings(|s| {
                    s.rotation = rotation;
                    s.pdf_sizing = "page".into();
                });
                settle(400).await;
                self.select_source_fixture().await;
                settle(150).await;
                let captured = match self.surface.borrow().as_ref() {
                    Some(Surface::Pdf(pdf)) => {
                        pdf.capture_selection("rotation check").is_ok_and(|c| {
                            c.passage.quote == expected && pdf.resolve_passage(&c.passage).is_ok()
                        })
                    }
                    _ => false,
                };
                checks[format!("pdf_selection_capability_at_{rotation}_degrees")] =
                    json!(if rotation == 0 {
                        captured
                    } else {
                        !captured && has_label(&self.notebook.panel, "Rotate upright to select")
                    });
                self.follow_saved_passage(&reference.id);
                settle(450).await;
                checks[format!("pdf_highlight_at_{rotation}_degrees")] = json!(
                    self.passage_is_highlighted().await && self.window.visible_dialog().is_none()
                );
                self.snapshot(&directory.join(format!("passage-rotation-{rotation}.png")))
                    .await;
            }
            self.save_selected_passage();
            settle(350).await;
            checks["pdf_rotate_upright_action_enables_selection"] = json!(
                matches!(self.surface.borrow().as_ref(), Some(Surface::Pdf(pdf)) if pdf.model.rotation() == 0)
            );
            self.change_settings(|s| *s = settings);
            settle(300).await;
        }
        self.queue_notebook();
        self.reading_state.barrier(Duration::ZERO).await.unwrap();
        let original = self.current.borrow().clone().unwrap();
        let fresh =
            readero::reading_state::ReadingState::start(directory.join("state/reading.sqlite3"));
        checks["passage_and_section_durable"] =
            json!(fresh.notebook(original).await.is_ok_and(|d| d.text == NOTES
                && d.references == [reference]
                && d.sections.len() == 1));
        checks
    }
}
