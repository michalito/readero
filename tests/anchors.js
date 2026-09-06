// Run in the smoke build's private WebKit world, using the real DOM and CFI code.
(async () => {
  const { rangeAt, rangeRect, locatorFor, resolveLocator } = await import(
    "readero://app/anchors.js"
  );
  const CFI = await import("readero://app/foliate/epubcfi.js");
  const book = {
    sections: [{ id: "content.xhtml", cfi: "epubcfi(/6/2)" }],
    resolveCFI: (cfi) => ({
      index: 0,
      anchor: (doc) => CFI.toRange(doc, CFI.parse(cfi).slice(1)),
    }),
  };
  const documentFor = (body) =>
    new DOMParser().parseFromString(
      `<html><body>${body}</body></html>`,
      "text/html",
    );
  const paragraph = (id, text) =>
    `<div id="${id}" data-reader-block="${id}"><p>${text}</p></div>`;
  const before = paragraph("before", "An earlier passage.");
  const target = paragraph("target", "The bookmarked passage continues here.");
  const after = paragraph("after", "A later passage.");
  const original = documentFor(before + target + after);
  const range = original.createRange();
  range.setStart(original.querySelector("#target p").firstChild, 15);
  range.collapse(true);
  const locator = locatorFor(book, 0, range);
  const checks = {};
  const blockId = (anchor) => {
    const node = anchor.startContainer ?? anchor;
    const element =
      node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    return element.closest("[data-reader-block]")?.id;
  };
  const unchanged = resolveLocator(book, locator).anchor(original);
  checks.unchanged_bookmark_keeps_character =
    unchanged.startContainer === range.startContainer &&
    unchanged.startOffset === 15;

  const inserted = documentFor(
    paragraph("inserted", "New material.") + before + target + after,
  );
  checks.bookmark_survives_insertion =
    blockId(resolveLocator(book, locator).anchor(inserted)) === "target";
  const deleted = documentFor(target + after);
  checks.bookmark_survives_deletion_before =
    blockId(resolveLocator(book, locator).anchor(deleted)) === "target";
  checks.changed_revision_finds_block =
    blockId(resolveLocator(book, locator, true).anchor(inserted)) === "target";
  checks.changed_revision_keeps_character =
    resolveLocator(book, locator, true).anchor(inserted).startOffset === 15;

  const edited = documentFor(
    before +
      paragraph(
        "edited",
        "The bookmarked passage continues here. Extra words.",
      ) +
      after,
  );
  checks.edited_block_uses_quote =
    blockId(resolveLocator(book, locator).anchor(edited)) === "edited" &&
    resolveLocator(book, locator, true).anchor(edited).startOffset === 15;
  const missing = documentFor(before + after);
  const approximate = resolveLocator(book, { ...locator, fraction: 0.4 });
  const nearby = approximate.anchor(missing);
  checks.deleted_passage_reports_approximation =
    approximate.recovery === "approximate" && nearby.startContainer.isConnected;
  const duplicated = documentFor(
    target + target.replaceAll('"target"', '"copy"'),
  );
  const ambiguous = resolveLocator(
    book,
    { ...locator, cfi: "", block: "" },
    true,
  );
  ambiguous.anchor(duplicated);
  checks.duplicate_quote_is_not_guessed = ambiguous.recovery === "approximate";
  const reordered = {
    ...book,
    sections: [
      { id: "new.xhtml", cfi: "epubcfi(/6/2)" },
      { id: "content.xhtml", cfi: "epubcfi(/6/4)" },
    ],
  };
  const repaired = resolveLocator(reordered, locator);
  checks.chapter_identity_survives_reorder =
    repaired.index === 1 &&
    repaired.anchor(original).startOffset === 15 &&
    repaired.recovery === "block";
  const unicode = documentFor(
    paragraph("unicode", "Reading 🐢 is calm. ".repeat(20)),
  );
  const unicodeRange = unicode.createRange();
  unicodeRange.setStart(unicode.querySelector("p").firstChild, 41);
  unicodeRange.collapse(true);
  const unicodeLocator = locatorFor(book, 0, unicodeRange);
  checks.unicode_quote_is_well_formed =
    unicodeLocator.quote.isWellFormed() &&
    unicodeLocator.quote_offset <= unicodeLocator.quote.length;
  const noBlock = resolveLocator(book, { ...locator, block: "" }).anchor(
    original,
  );
  checks.epub_cfi_keeps_character = noBlock.startOffset === 15;
  // Ordinary EPUBs have no Markdown block IDs; a valid CFI can point at
  // completely different text after an insertion in the same chapter.
  const epubDoc = documentFor(
    "<p>An earlier passage.</p><p>The bookmarked passage continues here.</p><p>A later passage.</p>",
  );
  const epubRange = epubDoc.createRange();
  epubRange.setStart(epubDoc.querySelectorAll("p")[1].firstChild, 15);
  const epubLocator = locatorFor(book, 0, epubRange);
  const epubInserted = documentFor(
    "<p>A newly inserted passage.</p>" + epubDoc.body.innerHTML,
  );
  const epubRecovered = resolveLocator(book, epubLocator);
  const recoveredRange = epubRecovered.anchor(epubInserted);
  checks.epub_bookmark_validates_quote_after_insertion =
    epubRecovered.recovery === "quote" &&
    recoveredRange.startOffset === 15 &&
    recoveredRange.startContainer.textContent ===
      epubRange.startContainer.textContent;
  const epubMissing = documentFor(
    "<p>An earlier passage.</p><p>A different passage has replaced the bookmark.</p><p>A later passage.</p>",
  );
  const epubApproximate = resolveLocator(book, epubLocator);
  epubApproximate.anchor(epubMissing);
  checks.epub_replaced_passage_reports_approximation =
    epubApproximate.recovery === "approximate";
  const epubFormatted = documentFor(
    "<p>An earlier passage.</p><p>The <em>bookmarked</em> passage continues here.</p><p>A later passage.</p>",
  );
  const epubReformatted = resolveLocator(book, epubLocator);
  const reformattedRange = epubReformatted.anchor(epubFormatted);
  checks.epub_quote_survives_inline_formatting =
    epubReformatted.recovery === "quote" &&
    reformattedRange.startContainer.textContent ===
      " passage continues here." &&
    reformattedRange.startOffset === 1;
  // A standalone illustration can contain a caption far from the viewport.
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const svgUrl = URL.createObjectURL(new Blob([
    '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="4000"><rect width="400" height="4000" fill="gray"/><text x="20" y="80">Caption</text></svg>',
  ], { type: "image/svg+xml" }));
  try {
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = svgUrl;
    await loaded;
    const svgDoc = frame.contentDocument;
    const rootRect = svgDoc.documentElement.getBoundingClientRect();
    const positions = [0.25, 0.75].map(fraction => {
      const range = rangeAt(svgDoc, 24, rootRect.top + fraction * rootRect.height);
      const locator = locatorFor(book, 0, range);
      return { range, locator, fraction };
    });
    checks.svg_caption_does_not_collapse_artwork_positions = positions.every(
      ({ range, locator, fraction }) =>
        !locator.cfi && !locator.quote &&
        Math.abs(locator.fraction - fraction) < 0.001 &&
        Math.abs(rangeRect(range).top - (rootRect.top + fraction * rootRect.height)) < 1 &&
        resolveLocator(book, locator).anchor(svgDoc) === fraction,
    );
  } finally { frame.remove(); URL.revokeObjectURL(svgUrl); }
  globalThis.readeroAnchorChecks = checks;
})().catch((error) => {
  globalThis.readeroAnchorChecks = { error: String(error) };
});
void 0;
