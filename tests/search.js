(async () => {
  const { searchRanges } = await import("readero://app/search.js");
  const { locatorFor, resolveLocator } = await import(
    "readero://app/anchors.js"
  );
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
  globalThis.readeroSearchChecks = checks;
})().catch((error) => {
  globalThis.readeroSearchChecks = { error: String(error) };
});
void 0;
