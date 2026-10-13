//! Pending target removal and retryable renderer failures in the native driver.
use super::*;

impl Shell {
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
        }
        checks
    }
}
