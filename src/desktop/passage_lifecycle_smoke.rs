//! Pending target removal and retryable renderer failures in the native driver.
use super::*;

impl Shell {
    pub(super) async fn target_verifies_directly(
        &self,
        reference: &readero::notebook::Reference,
    ) -> bool {
        let passage = reference.passage.as_ref().unwrap();
        let pdf = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => Some(
                pdf.resolve_passage(passage, "verification probe")
                    .is_ok_and(|r| !r.recovered),
            ),
            _ => None,
        };
        if let Some(verified) = pdf {
            return verified;
        }
        self.passage_script(&format!("JSON.stringify((() => {{globalThis.smokeDirectVerification=null; globalThis.readeroResolvePassage({}).then(r => globalThis.smokeDirectVerification = r.status === 'resolved' && !r.recovered, () => globalThis.smokeDirectVerification=false); return true;}})())", serde_json::to_string(passage).unwrap())).await;
        for _ in 0..100 {
            let result = self
                .passage_script("JSON.stringify(globalThis.smokeDirectVerification)")
                .await;
            if let Some(verified) = result.as_bool() {
                return verified;
            }
            settle(30).await;
        }
        false
    }

    pub(super) async fn check_recovered_passage(
        self: &Rc<Self>,
        directory: &std::path::Path,
    ) -> Value {
        let mut checks = json!({});
        let baseline = self.notebook.draft.borrow().clone().unwrap();
        let original = baseline.references[0].clone();
        let mut stale = original.clone();
        match &mut stale.locator.anchor {
            Anchor::Pdf { x, y, .. } => {
                *x = 500_000.0;
                *y = 500_000.0;
            }
            Anchor::Reflow { cfi, fraction, .. } => {
                *cfi = "epubcfi(/999999)".into();
                *fraction = 0.999;
            }
        }
        let passage = stale.passage.as_mut().unwrap();
        passage.revision = "obsolete recovery fixture".into();
        match &mut passage.target {
            readero::passage::Target::Pdf { fragments } => {
                for f in fragments {
                    f.points = [1_000_000.0, 1_000_000.0, 1_000_001.0, 1_000_001.0];
                }
            }
            readero::passage::Target::Reflow {
                cfi, start, end, ..
            } => {
                *cfi = "epubcfi(/999999)".into();
                *start += 1;
                *end += 1;
            }
        }
        self.notebook_metadata(|d| {
            d.references[0] = stale.clone();
            Ok(())
        });
        checks["recovery_fixture_is_not_structurally_verified"] =
            json!(!self.target_verifies_directly(&stale).await);
        let settings = self.current.borrow().as_ref().unwrap().settings.clone();
        if matches!(self.surface.borrow().as_ref(), Some(Surface::Pdf(_))) {
            self.change_settings(|s| s.rotation = 270);
            settle(400).await;
        }
        self.follow_saved_passage(&stale.id);
        settle(750).await;
        let recovered = self.notebook.draft.borrow().as_ref().unwrap().references[0].clone();
        checks["recovered_target_updates_metadata_and_revision"] = json!(
            recovered.locator != stale.locator
                && recovered.passage != stale.passage
                && recovered.passage.as_ref().unwrap().revision
                    == self.current.borrow().as_ref().unwrap().revision
        );
        checks["recovery_preserves_exact_quote_identity_label_and_connections"] = json!(
            recovered.id == original.id
                && recovered.label == original.label
                && recovered.passage.as_ref().unwrap().quote
                    == original.passage.as_ref().unwrap().quote
                && self.notebook.draft.borrow().as_ref().unwrap().sections == baseline.sections
        );
        checks["recovered_target_reopens_without_contextual_scan"] =
            json!(self.target_verifies_directly(&recovered).await);
        if let readero::passage::Target::Pdf { fragments } =
            &recovered.passage.as_ref().unwrap().target
        {
            let old_fragments = match &original.passage.as_ref().unwrap().target {
                readero::passage::Target::Pdf { fragments } => fragments,
                _ => unreachable!(),
            };
            checks["pdf_recovery_preserves_ordered_fragment_quotes"] = json!(
                fragments
                    .iter()
                    .map(|f| &f.quote)
                    .eq(old_fragments.iter().map(|f| &f.quote))
            );
            checks["pdf_recovery_fallback_uses_unrotated_selection_endpoint"] = json!(
                matches!(recovered.locator.anchor, Anchor::Pdf { page, x, y, .. }
                if page == fragments[0].page && x == fragments[0].points[0] && y == fragments[0].points[1])
            );
            self.change_settings(|s| *s = settings);
            settle(350).await;
            checks["pdf_recovered_target_remains_verified_upright"] =
                json!(self.target_verifies_directly(&recovered).await);
        }
        self.notebook_undo(false);
        checks["recovery_is_one_shared_history_action"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().references[0] == stale);
        self.notebook_undo(true);
        checks["recovery_redo_restores_updated_target"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().references[0] == recovered);
        self.queue_notebook();
        self.reading_state.barrier(Duration::ZERO).await.unwrap();
        let fresh =
            readero::reading_state::ReadingState::start(directory.join("state/reading.sqlite3"));
        let current = self.current.borrow().clone().unwrap();
        checks["recovered_target_is_persisted_for_fresh_worker"] = json!(
            fresh
                .notebook(current)
                .await
                .is_ok_and(|d| d.references[0] == recovered && d.sections == baseline.sections)
        );
        std::fs::write(
            directory.join("recovered-reference.json"),
            serde_json::to_vec(&recovered).unwrap(),
        )
        .unwrap();
        checks
    }

    pub(super) async fn check_passage_lifecycle(
        self: &Rc<Self>,
        original: &readero::notebook::Reference,
    ) -> Value {
        let mut checks = json!({});
        let baseline = self.notebook.draft.borrow().clone().unwrap();
        for mode in ["connect", "relink"] {
            for removal in ["unconnected_delete", "connected_delete", "undo"] {
                let mut target = original.clone();
                target.id = format!("pending-{mode}-{removal}");
                if mode == "relink" {
                    let passage = target.passage.as_mut().unwrap();
                    passage
                        .quote
                        .replace_range(..passage.quote.chars().next().unwrap().len_utf8(), "§");
                    if let readero::passage::Target::Pdf { fragments } = &mut passage.target {
                        let quote = &mut fragments[0].quote;
                        quote.replace_range(..quote.chars().next().unwrap().len_utf8(), "§");
                    }
                }
                self.notebook_metadata(|draft| {
                    draft.references.push(target.clone());
                    if removal == "connected_delete" {
                        draft.connect(0, 5, &target.id)?;
                    }
                    Ok(())
                });
                self.render_references();
                if mode == "connect" {
                    self.begin_passage_connection(&target.id);
                } else {
                    self.follow_saved_passage(&target.id);
                    settle(600).await;
                    if let Some(dialog) = self
                        .window
                        .visible_dialog()
                        .and_downcast::<adw::AlertDialog>()
                    {
                        click_button(&dialog, "Relink to selected text");
                    }
                    settle(150).await;
                }
                let (connect, relink, _) = self.notebook.passage_mode_probe(&target.id);
                checks[format!("{mode}_{removal}_fixture_has_pending_mode")] =
                    json!(if mode == "connect" { connect } else { relink });
                let delayed = mode == "relink"
                    && removal == "unconnected_delete"
                    && matches!(self.surface.borrow().as_ref(), Some(Surface::Reflow(_)));
                if delayed {
                    self.select_source_fixture().await;
                    settle(150).await;
                    self.delay_capture().await;
                    self.save_selected_passage();
                    settle(150).await;
                    checks["deleted_relink_fixture_has_pending_capture"] =
                        json!(self.notebook.capture_probe().3);
                }
                if removal == "undo" {
                    self.notebook_undo(false);
                } else {
                    self.delete_saved_passage(&target.id);
                    if removal == "connected_delete" {
                        settle(100).await;
                        if let Some(dialog) = self
                            .window
                            .visible_dialog()
                            .and_downcast::<adw::AlertDialog>()
                        {
                            click_button(&dialog, "Cancel");
                        }
                        let (connect, relink, _) = self.notebook.passage_mode_probe(&target.id);
                        checks[format!("{mode}_cancelled_deletion_keeps_pending_mode")] =
                            json!(connect || relink);
                        self.delete_saved_passage(&target.id);
                        settle(100).await;
                        if let Some(dialog) = self
                            .window
                            .visible_dialog()
                            .and_downcast::<adw::AlertDialog>()
                        {
                            click_button(&dialog, "Delete and disconnect");
                        }
                    }
                }
                let (connect, relink, _) = self.notebook.passage_mode_probe(&target.id);
                checks[format!("{mode}_{removal}_clears_removed_target")] = json!(
                    !connect
                        && !relink
                        && !self.notebook.capture_probe().3
                        && self.notebook.draft.borrow().as_ref().unwrap().references
                            == baseline.references
                        && self.notebook.draft.borrow().as_ref().unwrap().sections
                            == baseline.sections
                );
                if delayed {
                    self.release_capture().await;
                    settle(200).await;
                    checks["deleted_relink_late_reply_cannot_restore_mode_or_reference"] = json!(
                        self.window.visible_dialog().is_none()
                            && self.notebook.draft.borrow().as_ref().unwrap().references
                                == baseline.references
                    );
                }
                // Restore the reference and remove it again via the opposite
                // history direction; ephemeral modes must never be resurrected.
                self.notebook_undo(removal == "undo");
                let (connect, relink, _) = self.notebook.passage_mode_probe(&target.id);
                checks[format!("{mode}_{removal}_history_restores_reference_without_mode")] = json!(
                    !connect
                        && !relink
                        && self
                            .notebook
                            .draft
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .references
                            .iter()
                            .any(|r| r.id == target.id)
                );
                if mode == "connect" {
                    self.begin_passage_connection(&target.id);
                } else {
                    self.follow_saved_passage(&target.id);
                    settle(600).await;
                    if let Some(dialog) = self
                        .window
                        .visible_dialog()
                        .and_downcast::<adw::AlertDialog>()
                    {
                        click_button(&dialog, "Relink to selected text");
                    }
                    settle(150).await;
                }
                self.notebook_metadata(|_| Ok(()));
                let (connect, relink, _) = self.notebook.passage_mode_probe(&target.id);
                checks[format!("{mode}_{removal}_reconcile_preserves_existing_target")] =
                    json!(if mode == "connect" { connect } else { relink });
                self.notebook_undo(removal != "undo");
                let (connect, relink, _) = self.notebook.passage_mode_probe(&target.id);
                checks[format!("{mode}_{removal}_history_removal_clears_reactivated_target")] = json!(
                    !connect
                        && !relink
                        && self.notebook.draft.borrow().as_ref().unwrap().references
                            == baseline.references
                );
            }
        }
        self.begin_passage_connection("removed-fixture-id");
        checks["stale_connection_action_cannot_start_missing_target"] =
            json!(!self.notebook.passage_mode_probe("removed-fixture-id").0);
        self.select_source_fixture().await;
        settle(150).await;
        self.save_selected_passage();
        settle(350).await;
        checks["capture_after_target_removal_is_normal"] = json!(
            self.window.visible_dialog().is_none()
                && !self.notebook.relink_pending()
                && self.notebook.draft.borrow().as_ref().unwrap().references == baseline.references
        );
        if matches!(self.surface.borrow().as_ref(), Some(Surface::Reflow(_))) {
            self.passage_script("JSON.stringify((globalThis.smokeOriginalResolve=globalThis.readeroResolvePassage,true))").await;
            for failure in ["timeout", "transport", "payload", "invalid_capture"] {
                let script = match failure {
                    "timeout" => "globalThis.readeroResolvePassage = () => new Promise(() => {});",
                    "transport" => {
                        "globalThis.readeroResolvePassage = () => { throw new Error('temporary bridge failure fixture'); };"
                    }
                    "payload" => {
                        "globalThis.readeroResolvePassage = () => ({status:'not-a-protocol-result'});"
                    }
                    _ => {
                        "globalThis.readeroResolvePassage = async (...args) => { const result = await globalThis.smokeOriginalResolve(...args); result.passage.quote=''; return result; };"
                    }
                };
                self.passage_script(&format!(
                    "JSON.stringify((() => {{{script} return true;}})())"
                ))
                .await;
                settle(5500).await; // Previous retry toast must expire before this assertion.
                self.follow_saved_passage(&original.id);
                let mut retried = false;
                for _ in 0..130 {
                    settle(50).await;
                    if has_label(
                        &self.window,
                        "Could not open this passage. Please try again.",
                    ) {
                        retried = true;
                        break;
                    }
                }
                checks[format!("resolver_{failure}_reports_retry_without_relink")] = json!(
                    retried
                        && self.window.visible_dialog().is_none()
                        && !self.notebook.passage_mode_probe(&original.id).2
                        && self.notebook.draft.borrow().as_ref().unwrap().references
                            == baseline.references
                        && self.notebook.draft.borrow().as_ref().unwrap().sections
                            == baseline.sections
                );
            }
            self.passage_script("JSON.stringify((globalThis.readeroResolvePassage=globalThis.smokeOriginalResolve,true))").await;
            self.follow_saved_passage(&original.id);
            settle(700).await;
            checks["resolver_retry_succeeds_without_replacing_passage"] = json!(
                self.passage_is_highlighted().await
                    && self.notebook.draft.borrow().as_ref().unwrap().references
                        == baseline.references
            );
            self.passage_script("JSON.stringify((globalThis.readeroResolvePassage=()=>({status:'unresolved'}),true))").await;
            self.follow_saved_passage(&original.id);
            settle(300).await;
            if let Some(dialog) = self
                .window
                .visible_dialog()
                .and_downcast::<adw::AlertDialog>()
            {
                click_button(&dialog, "Keep passage");
            }
            settle(150).await;
            checks["verification_fixture_remembers_failed_exact_target"] =
                json!(self.notebook.passage_mode_probe(&original.id).2);
            self.notebook_metadata(|d| {
                d.references[0].label.push_str(" verified after rename");
                Ok(())
            });
            self.passage_script("JSON.stringify((globalThis.readeroResolvePassage=globalThis.smokeOriginalResolve,true))").await;
            self.follow_saved_passage(&original.id);
            settle(600).await;
            checks["newer_success_retires_old_exact_target_failure"] = json!(
                !self.notebook.passage_mode_probe(&original.id).2
                    && self.passage_is_highlighted().await
            );
            self.notebook_undo(false);
            checks["unrelated_undo_preserves_newer_successful_verification"] =
                json!(!self.notebook.passage_mode_probe(&original.id).2);
            self.notebook_undo(true);
            self.notebook_undo(false);
            checks["redo_then_undo_cannot_resurrect_superseded_failure"] = json!(
                !self.notebook.passage_mode_probe(&original.id).2
                    && self.notebook.draft.borrow().as_ref().unwrap().references
                        == baseline.references
            );
        }
        checks
    }
}
