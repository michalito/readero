// Executed by the native smoke driver in WebKit's private world.
(async () => {
  const { Passages, UnresolvedPassage } =
    await import("readero://app/passages.js");
  const CFI = await import("readero://app/foliate/epubcfi.js");
  const book = {
    sections: [{ id: "content.xhtml", cfi: "epubcfi(/6/2)" }],
    resolveCFI: (cfi) => {
      const parts = CFI.parse(cfi);
      (parts.parent ?? parts).shift();
      return { index: 0, anchor: (doc) => CFI.toRange(doc, parts) };
    },
  };
  const documentFor = (body) =>
    new DOMParser().parseFromString(
      `<html><body>${body}</body></html>`,
      "text/html",
    );
  const capture = (doc, range) => {
    const passages = new Passages(book, "test revision", () => {});
    passages.selection = { doc, index: 0, range };
    return passages.capture(0);
  };
  const checks = {};
  // Exercise availability through the same selectionchange adapter as the
  // reader, with real DOM ranges but without taking over the active selection.
  const availabilityDoc = documentFor(
    "<p>words   words</p><p><b> \n\u00a0 </b><i>\t </i></p><p><b> </b><i>🦉</i></p>",
  );
  let selected = availabilityDoc.createRange();
  const listeners = {};
  const availability = [];
  let focusedChapter = 0;
  const source = {
    body: availabilityDoc.body,
    hasFocus: () => focusedChapter === 0,
    addEventListener: (event, callback) => (listeners[event] = callback),
    getSelection: () => ({
      rangeCount: 1,
      isCollapsed: selected.collapsed,
      getRangeAt: () => selected,
    }),
  };
  const selectionState = new Passages(book, "availability", (event) =>
    availability.push(event.available),
  );
  selectionState.attach(source, 0);
  const words = availabilityDoc.querySelector("p").firstChild;
  selected.setStart(words, 0);
  selected.setEnd(words, 5);
  listeners.selectionchange();
  selected.setStart(words, 5);
  selected.setEnd(words, 8);
  listeners.selectionchange();
  checks.whitespace_replaces_available_selection =
    availability[0] === true && availability[1] === false;
  checks.whitespace_has_no_capturable_snapshot =
    selectionState.selection === null;
  selected.selectNodeContents(availabilityDoc.querySelectorAll("p")[1]);
  listeners.selectionchange();
  checks.multinode_unicode_whitespace_disables_selection =
    availability.at(-1) === false;
  selected.selectNodeContents(availabilityDoc.querySelectorAll("p")[2]);
  listeners.selectionchange();
  checks.multinode_nonwhitespace_enables_selection =
    availability.at(-1) === true;
  selected.collapse(true);
  listeners.selectionchange();
  checks.collapsed_selection_disables_action = availability.at(-1) === false;
  const otherDoc = documentFor("<p>other   </p>");
  const otherRange = otherDoc.createRange();
  const otherListeners = {};
  const otherSource = {
    body: otherDoc.body,
    hasFocus: () => focusedChapter === 1,
    addEventListener: (event, callback) => (otherListeners[event] = callback),
    getSelection: () => ({
      rangeCount: 1,
      isCollapsed: otherRange.collapsed,
      getRangeAt: () => otherRange,
    }),
  };
  selectionState.attach(otherSource, 1);
  const selectFirstChapter = () => {
    focusedChapter = 0;
    selected.setStart(words, 0);
    selected.setEnd(words, 5);
    listeners.selectionchange();
  };
  selectFirstChapter();
  otherRange.setStart(otherDoc.querySelector("p").firstChild, 5);
  otherRange.setEnd(otherDoc.querySelector("p").firstChild, 8);
  focusedChapter = 1;
  otherListeners.selectionchange();
  checks.other_chapter_whitespace_invalidates_previous_capture =
    availability.at(-1) === false && selectionState.selection === null;
  selectFirstChapter();
  otherRange.collapse(true);
  focusedChapter = 1;
  otherListeners.selectionchange();
  checks.other_chapter_click_invalidates_previous_capture =
    availability.at(-1) === false && selectionState.selection === null;
  otherRange.setStart(otherDoc.querySelector("p").firstChild, 0);
  otherRange.setEnd(otherDoc.querySelector("p").firstChild, 5);
  otherListeners.selectionchange();
  const currentSerial = selectionState.serial;
  selected.collapse(true);
  listeners.selectionchange();
  checks.background_deselection_keeps_newer_chapter_selection =
    selectionState.selection?.doc === otherSource &&
    selectionState.serial === currentSerial;
  selectionState.selection = null;
  const backgroundHighlights = new Map([["readero-passage", {}]]);
  selectionState.marked.add({
    defaultView: { CSS: { highlights: backgroundHighlights } },
  });
  listeners.selectionchange();
  checks.background_deselection_keeps_passage_highlight =
    backgroundHighlights.has("readero-passage");
  selectFirstChapter();
  const highlights = new Map([["readero-passage", {}]]);
  selectionState.marked.add({ defaultView: { CSS: { highlights } } });
  selected.collapse(true);
  listeners.selectionchange();
  checks.user_deselection_clears_temporary_highlight =
    !highlights.has("readero-passage") && selectionState.marked.size === 0;
  selectFirstChapter();
  const fallback = { doc: source, range: selected.cloneRange() };
  selectionState.fallback = fallback;
  const fallbackSerial = selectionState.serial;
  listeners.selectionchange();
  checks.fallback_mark_selection_event_is_ignored =
    selectionState.fallback === fallback &&
    selectionState.serial === fallbackSerial;
  selectionState.fallback = null;
  const doc = documentFor(
    "<p>Before.</p><p>A café <em>👩‍🔬 é</em> מחשבה.</p><p>After.</p>",
  );
  const range = doc.createRange();
  range.selectNodeContents(doc.querySelectorAll("p")[1]);
  const saved = capture(doc, range);
  checks.selection_element_endpoints_keep_full_unicode_quote =
    saved.passage.quote === range.toString();
  checks.selection_offsets_count_utf16 =
    saved.passage.end - saved.passage.start === saved.passage.quote.length;
  const across = doc.createRange();
  across.setStart(doc.querySelector("p").firstChild, 2);
  across.setEnd(doc.querySelectorAll("p")[2].firstChild, 3);
  checks.selection_spanning_inline_and_paragraph_nodes =
    capture(doc, across).passage.quote === across.toString();
  const partial = doc.createRange();
  partial.setStart(doc.querySelector("em").firstChild, 0);
  partial.setEnd(doc.querySelector("em").firstChild, 5);
  checks.selection_emoji_keeps_surrogate_pairs =
    capture(doc, partial).passage.quote === "👩‍🔬";
  const passages = new Passages(book, "new revision", () => {});
  const resolved = await passages.resolve(saved.passage, [{ index: 0, doc }]);
  checks.unchanged_range_verifies_exactly =
    !resolved.recovered && resolved.passage.quote === saved.passage.quote;
  const changed = documentFor(
    "<aside>Inserted before.</aside><p>Before.</p><section><p>A café <strong>👩‍🔬 é</strong> מחשבה.</p></section><p>After.</p>",
  );
  // A long prefix makes unrelated insertion outside the local context immaterial.
  const recoverable = { ...saved.passage, cfi: "", prefix: "Before." };
  const recovered = await passages.resolve(recoverable, [
    { index: 0, doc: changed },
  ]);
  checks.unique_context_recovers_reformatted_range =
    recovered.recovered && recovered.passage.quote === saved.passage.quote;
  async function rejects(passage, source = doc) {
    try {
      await passages.resolve(passage, [{ index: 0, doc: source }]);
      return false;
    } catch {
      return true;
    }
  }
  checks.changed_quote_needs_relink = await rejects({
    ...saved.passage,
    quote: "Words no longer present.",
  });
  checks.missing_original_resource_needs_relink = await rejects({
    ...saved.passage,
    href: "missing.xhtml",
  });
  checks.ambiguous_quote_is_never_guessed = await rejects(
    { ...saved.passage, cfi: "", quote: "same", prefix: "", suffix: "" },
    documentFor("<p>same same</p>"),
  );
  checks.overlapping_quote_candidates_are_ambiguous = await rejects(
    { ...saved.passage, cfi: "", quote: "aaa", prefix: "", suffix: "" },
    documentFor("<p>aaaa</p>"),
  );
  const only = documentFor("<p>before same after / other same end</p>");
  const distinct = await passages.resolve(
    {
      ...saved.passage,
      cfi: "",
      quote: "same",
      prefix: "other ",
      suffix: " end",
    },
    [{ index: 0, doc: only }],
  );
  checks.context_disambiguates_repeated_words = distinct.passage.start === 26;
  checks.changed_context_is_not_claimed_verified = await rejects({
    ...saved.passage,
    prefix: "different prefix",
  });
  const manyMatches = documentFor(`<p>${"a ".repeat(50000)}X a Y</p>`);
  const slowPassage = {
    ...saved.passage,
    cfi: "",
    quote: "a",
    prefix: "X ",
    suffix: " Y",
  };
  const cancelled = (promise) =>
    promise.then(
      () => false,
      (error) => error.message === "Passage navigation was cancelled.",
    );
  const recovery = new Passages(book, "cancellation", () => {});
  const afterNavigation = cancelled(
    recovery.resolve(slowPassage, [{ index: 0, doc: manyMatches }]),
  );
  recovery.clear();
  checks.navigation_aborts_yielded_recovery_scan = await afterNavigation;
  const superseded = cancelled(
    recovery.resolve(slowPassage, [{ index: 0, doc: manyMatches }]),
  );
  const newest = await recovery.resolve(saved.passage, [{ index: 0, doc }]);
  checks.newer_request_aborts_yielded_recovery_scan = await superseded;
  checks.newer_request_still_resolves_its_passage =
    newest.passage.quote === saved.passage.quote;
  let finishLoading;
  const loading = new Passages(
    {
      ...book,
      sections: [
        {
          ...book.sections[0],
          createDocument: () =>
            new Promise((resolve) => (finishLoading = resolve)),
        },
      ],
    },
    "cancellation",
    () => {},
  );
  const afterLoad = cancelled(loading.resolve(saved.passage, []));
  loading.clear();
  finishLoading(doc);
  checks.navigation_aborts_recovery_after_document_load = await afterLoad;
  const blank = documentFor("<p>  \n\t </p>");
  const empty = blank.createRange();
  empty.selectNodeContents(blank.body);
  try {
    capture(blank, empty);
    checks.blank_capture_rejected = false;
  } catch {
    checks.blank_capture_rejected = true;
  }
  const huge = documentFor(`<p>${"x".repeat(65537)}</p>`);
  const big = huge.createRange();
  big.selectNodeContents(huge.body);
  try {
    capture(huge, big);
    checks.oversized_capture_rejected_without_truncation = false;
  } catch {
    checks.oversized_capture_rejected_without_truncation = true;
  }
  passages.selection = { doc, index: 0, range };
  passages.clear();
  try {
    passages.capture(0);
    checks.invalidated_selection_cannot_be_saved = false;
  } catch {
    checks.invalidated_selection_cannot_be_saved = true;
  }
  const operational = new Passages(
    {
      ...book,
      sections: [
        {
          ...book.sections[0],
          createDocument: async () => {
            throw new Error("temporary chapter load");
          },
        },
      ],
    },
    "test",
    () => {},
  );
  try {
    await operational.resolve(saved.passage, []);
    checks.chapter_load_error_is_retryable = false;
  } catch (error) {
    checks.chapter_load_error_is_retryable =
      !(error instanceof UnresolvedPassage) &&
      error.message === "temporary chapter load";
  }
  try {
    await operational.resolve(
      { ...saved.passage, href: "missing-resource.xhtml" },
      [],
    );
    checks.missing_resource_is_explicit_matching_failure = false;
  } catch (error) {
    checks.missing_resource_is_explicit_matching_failure =
      error instanceof UnresolvedPassage;
  }
  passages.dispose();
  checks.disposed_source_cannot_resolve = await rejects(saved.passage);
  globalThis.readeroPassageChecks = checks;
})().catch((error) => {
  globalThis.readeroPassageChecks = { error: String(error) };
});
void 0;
