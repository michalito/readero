(async () => {
  const { searchRanges, searchBook } = await import("readero://app/search.js");
  const { locatorFor, resolveLocator } =
    await import("readero://app/anchors.js");
  const doc = new DOMParser().parseFromString(
    "<html><body><p>🦉 A quiet <em>reader</em> keeps reading.</p><p>A qui<strong>et</strong> reader returns.</p><p>quiet</p><p>reader</p><script>quiet reader</script><style>quiet reader</style><p>Literal a+b.</p></body></html>",
    "text/html",
  );
  const matches = await searchRanges(doc, "quiet reader");
  const checks = {
    search_crosses_inline_formatting:
      matches.length === 2 &&
      matches.every(({ range }) => range.toString() === "quiet reader"),
    search_maps_utf16_offsets:
      matches[0]?.range.startOffset === 5 &&
      matches[0]?.range.endContainer === doc.querySelector("em").firstChild &&
      matches[0]?.range.endOffset === 6,
    search_matches_split_words: (await searchRanges(doc, "quiet")).length === 3,
    search_ignores_case: (await searchRanges(doc, "QUIET READER")).length === 2,
    search_escapes_literal_query: (await searchRanges(doc, "a+b")).length === 1,
    search_does_not_join_blocks:
      (await searchRanges(doc, "quietreader")).length === 0,
    search_caps_results: (await searchRanges(doc, "reader", 1)).length === 1,
    search_empty_query: (await searchRanges(doc, " ")).length === 0,
    search_cancellation:
      (await searchRanges(doc, "reader", 500, () => true)).length === 0,
  };
  const book = { sections: [{ id: "content.xhtml", cfi: "epubcfi(/6/2)" }] };
  const locator = locatorFor(book, 0, matches[0].range);
  const restored = resolveLocator(book, { ...locator, cfi: "" }).anchor(doc);
  checks.search_locator_returns_to_match =
    restored.startContainer === matches[0].range.startContainer &&
    restored.startOffset === 5;
  const svg = new DOMParser().parseFromString(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 600"><text y="30">quiet reader</text></svg>',
    "image/svg+xml",
  );
  checks.search_svg_text =
    (await searchRanges(svg, "quiet reader")).length === 1;
  for (const [name, sections] of Object.entries({
    final_svg: [doc, svg],
    only_svg: [svg],
    final_empty: [doc, null],
    rejected_section: [doc, new Error("Unreadable section"), doc],
    no_sections: [],
  })) {
    const updates = [],
      errors = [];
    const candidate = {
      sections: sections.map((section, index) => ({
        id: `${index}.xhtml`,
        cfi: `epubcfi(/6/${2 * (index + 1)})`,
        async createDocument() {
          if (section instanceof Error) throw section;
          return section;
        },
      })),
    };
    await searchBook(
      candidate,
      "quiet reader",
      (update) => updates.push(update),
      () => false,
      (error) => errors.push(error),
    );
    checks[`search_completes_${name}`] =
      updates.at(-1)?.complete === true &&
      updates.filter((u) => u.complete).length === 1;
    if (name === "rejected_section")
      checks.search_reports_partial_failure =
        errors.length === 1 && updates.at(-1).items.length === 4;
  }
  let cancel = false,
    completed = false,
    release;
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  const searching = searchBook(
    { sections: [{ createDocument: () => pending }] },
    "quiet",
    () => {
      completed = true;
    },
    () => cancel,
    () => {},
  );
  cancel = true;
  release(doc);
  await searching;
  checks.cancelled_search_does_not_complete = !completed;
  globalThis.readeroSearchChecks = checks;
})().catch((error) => {
  globalThis.readeroSearchChecks = { error: String(error) };
});
void 0;
