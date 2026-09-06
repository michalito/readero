// Run against real WebKit iframe layout; no mocked geometry or renderer.
(async () => {
  const { Continuous } = await import("readero://app/continuous.js");
  const { rangeRect, locatorFor, resolveLocator } = await import("readero://app/anchors.js");
  const { ImageInspector, prepareContent, handlesOwnKeys } =
    await import("readero://app/interaction.js");
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const checks = {};
  const host = document.createElement("div");
  host.style.cssText =
    "position:fixed;left:-1000px;top:0;width:800px;height:600px";
  document.body.append(host);
  let slow = false;
  let scrollOnLoad = null;
  let slowIndex = 5;
  const urls = new Map();
  const book = {
    sections: Array.from({ length: 16 }, (_, index) => ({
      async load() {
        if (slow && index === slowIndex) {
          await wait(180);
          scrollOnLoad?.();
        }
        const url = URL.createObjectURL(
          new Blob(
            [
              `<html><head></head><body><img alt="A reading illustration"><pre>${"a long code sample ".repeat(200)}</pre>${Array.from({ length: 35 }, (_, n) => `<p>Section ${index}, passage ${n}: ${"Comfortable reading returns to the same passage. ".repeat(5)}</p>`).join("")}</body></html>`,
            ],
            { type: "text/html" },
          ),
        );
        urls.set(index, url);
        return url;
      },
      unload() {
        URL.revokeObjectURL(urls.get(index));
        urls.delete(index);
      },
    })),
  };
  const reader = new Continuous(
    book,
    "body{font:20px/1.6 serif} pre{width:100%;overflow:auto;max-height:200px} p{margin:0 0 20px}",
    () => {},
  );
  host.append(reader.element);
  let peak = 0;
  const observe = new MutationObserver(() => {
    peak = Math.max(peak, reader.frames.size);
  });
  observe.observe(reader.element, { childList: true });
  const passage = (doc) => doc.querySelectorAll("p")[15];
  try {
    await reader.goTo({ index: 0, anchor: passage });
    slow = true;
    await reader.goTo({ index: 3, anchor: passage });
    reader.unmount(5);
    const scrollingLoad = reader.maintain();
    await wait(60);
    const beforeScroll = reader.element.scrollTop;
    reader.element.scrollTop += 400;
    await wait(60);
    const scrolled = reader.capture();
    checks.scrolled_during_pending_preload =
      reader.busy && reader.element.scrollTop - beforeScroll > 390;
    await scrollingLoad;
    await wait(60);
    checks.preload_keeps_latest_scroll =
      Math.abs(
        reader.frames.get(scrolled.index).frame.getBoundingClientRect().top +
          rangeRect(scrolled.range).top -
          scrolled.screenY,
      ) < 3;
    // Repeat with a chapter inserted above the reader, and move in the same
    // task as load completion, before WebKit delivers its scroll event.
    slowIndex = 1;
    reader.unmount(1);
    let latest;
    scrollOnLoad = () => {
      reader.element.scrollTop += 400;
      latest = reader.capture();
    };
    reader.index = 3;
    await reader.maintain();
    await wait(60);
    checks.prepend_keeps_scroll_before_event =
      !!latest &&
      Math.abs(
        reader.frames.get(latest.index).frame.getBoundingClientRect().top +
          rangeRect(latest.range).top -
          latest.screenY,
      ) < 3;
    scrollOnLoad = null;
    slowIndex = 5;
    await reader.goTo({ index: 0, anchor: passage });
    reader.index = 3;
    const pending = reader.maintain();
    await reader.goTo({ index: 10, anchor: passage });
    await pending;
    checks.preload_cannot_overwrite_jump =
      reader.capture()?.index === 10 &&
      reader.frames.has(10) &&
      !reader.frames.has(3);
    const original = reader.capture();
    const doc = reader.frames.get(10).frame.contentDocument;
    const image = doc.querySelector("img");
    image.src =
      "data:image/svg+xml," +
      encodeURIComponent(
        '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="360"><rect width="400" height="360" fill="gray"/></svg>',
      );
    await wait(180);
    const screenY = () =>
      reader.frames.get(10).frame.getBoundingClientRect().top +
      rangeRect(original.range).top;
    checks.delayed_image_keeps_passage =
      Math.abs(screenY() - original.screenY) < 3;
    host.style.width = "650px";
    await wait(180);
    checks.resize_keeps_passage = Math.abs(screenY() - original.screenY) < 3;
    prepareContent(doc, () => {});
    checks.long_block_owns_keys =
      doc.querySelector("pre").tabIndex === 0 &&
      handlesOwnKeys(doc.querySelector("pre"));
    checks.image_keyboard_affordance =
      image.tabIndex === 0 && image.getAttribute("role") === "button";
    const inspector = new ImageInspector(host);
    inspector.open(image);
    checks.image_dialog_is_modal =
      inspector.dialog.getAttribute("aria-modal") === "true" &&
      reader.element.inert;
    checks.image_close_restores_focus =
      inspector.close() && !reader.element.inert && doc.activeElement === image;
    for (const index of [
      11, 12, 13, 14, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    ]) {
      reader.index = index;
      await reader.maintain(false);
    }
    checks.transient_chapter_frames_bounded = peak <= 5;
    slow = true;
    reader.index = 3;
    const loading = reader.maintain();
    reader.destroy();
    await loading;
    checks.destroy_releases_chapters =
      reader.frames.size === 0 && urls.size === 0;
  } finally {
    observe.disconnect();
    reader.destroy();
    host.remove();
  }
  const svgHost = document.createElement("div");
  svgHost.style.cssText =
    "position:fixed;left:-1000px;top:0;width:600px;height:500px";
  document.body.append(svgHost);
  const svgUrl = URL.createObjectURL(
    new Blob(
      [
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 800">\n<defs><linearGradient id="shade"><stop stop-color="gray"/></linearGradient></defs>\n<rect width="400" height="800" fill="url(#shade)"/><text x="20" y="40">Caption above the artwork</text>\n</svg>',
      ],
      { type: "image/svg+xml" },
    ),
  );
  const CFI = await import("readero://app/foliate/epubcfi.js");
  const svgBook = {
    sections: [{ id: "image.svg", cfi: "epubcfi(/6/2)", load: async () => svgUrl }],
    resolveCFI: (cfi) => ({
      index: 0,
      anchor: (doc) => CFI.toRange(doc, CFI.parse(cfi).slice(1)),
    }),
  };
  const svgReader = new Continuous(
    svgBook,
    "svg{max-width:100%;height:auto}",
    () => {},
  );
  svgHost.append(svgReader.element);
  try {
    await svgReader.goTo({ index: 0, anchor: () => 0 });
    await wait(100);
    const frame = svgReader.frames.get(0).frame;
    const capture = svgReader.capture();
    const locator = locatorFor(svgBook, capture.index, capture.range);
    const restored = resolveLocator(svgBook, JSON.parse(JSON.stringify(locator)));
    checks.svg_chapter_has_locator =
      locator.href === "image.svg" && !locator.cfi &&
      typeof restored.anchor(frame.contentDocument) === "number";
    const empty = new DOMParser().parseFromString(
      '<svg xmlns="http://www.w3.org/2000/svg"/>', "image/svg+xml",
    );
    const { rangeAt } = await import("readero://app/anchors.js");
    const emptyLocator = locatorFor(svgBook, 0, rangeAt(empty), 0.4);
    checks.empty_svg_has_persistent_locator =
      emptyLocator.href === "image.svg" &&
      resolveLocator(svgBook, JSON.parse(JSON.stringify(emptyLocator))).anchor(empty) === 0.4;
    checks.svg_chapter_fits_aspect_ratio =
      Math.abs(
        frame.getBoundingClientRect().height -
          frame.getBoundingClientRect().width * 2,
      ) < 2;
    svgReader.element.scrollTop = 350;
    await wait(100);
    const middle = svgReader.capture();
    const bookmark = JSON.parse(JSON.stringify(locatorFor(svgBook, 0, middle.range, middle.fraction)));
    checks.svg_positions_are_distinct = bookmark.fraction > locator.fraction + 0.2;
    await svgReader.goTo({ index: 0, anchor: () => 0 });
    await svgReader.goTo(resolveLocator(svgBook, bookmark));
    checks.svg_bookmark_restores_position = Math.abs(svgReader.element.scrollTop - 350) < 2;
    // A newly constructed renderer exercises reopening from serialized state.
    const reopened = new Continuous(svgBook, "", () => {});
    svgHost.append(reopened.element);
    try {
      await reopened.goTo(resolveLocator(svgBook, bookmark));
      checks.svg_reopen_restores_position = Math.abs(reopened.element.scrollTop - 350) < 2;
    } finally { reopened.destroy(); }
    svgHost.style.width = "400px";
    await wait(180);
    checks.svg_chapter_resizes =
      Math.abs(
        frame.getBoundingClientRect().height -
          frame.getBoundingClientRect().width * 2,
      ) < 2;
    svgReader.setStyles("svg{height:auto}");
    await wait(100);
    checks.svg_chapter_keeps_anchor =
      Math.abs(svgReader.capture().fraction - bookmark.fraction) < 0.005;
  } finally {
    svgReader.destroy();
    URL.revokeObjectURL(svgUrl);
    svgHost.remove();
  }
  // Exercise the default paginator with real SVG and HTML spine documents.
  await import("readero://app/foliate/paginator.js");
  const pagesHost = document.createElement("div");
  pagesHost.style.cssText = "position:fixed;left:-1000px;top:0;width:600px;height:500px";
  document.body.append(pagesHost);
  const pages = document.createElement("foliate-paginator");
  pages.style.cssText = "display:block;width:100%;height:100%";
  pages.setAttribute("max-column-count", "1");
  pagesHost.append(pages);
  const cover = URL.createObjectURL(new Blob([
    '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="800"><rect width="400" height="800" fill="gray"/></svg>',
  ], { type: "image/svg+xml" }));
  const chapter = URL.createObjectURL(new Blob(['<html><body><p>After the cover.</p></body></html>'], { type: "text/html" }));
  const pagedBook = { sections: [
    { id: "cover.svg", cfi: "epubcfi(/6/2)", load: async () => cover },
    { id: "chapter.xhtml", cfi: "epubcfi(/6/4)", load: async () => chapter },
  ] };
  let location;
  pages.addEventListener("relocate", ({ detail }) => { location = detail; });
  try {
    pages.open(pagedBook);
    await Promise.race([
      pages.goTo({ index: 0, anchor: () => 0 }),
      wait(3000).then(() => { throw new Error("SVG paginator readiness timed out"); }),
    ]);
    checks.pages_svg_ready = location?.index === 0 && !!locatorFor(pagedBook, 0, location.range, location.fraction);
    checks.pages_svg_is_one_page = pages.pages === 3;
    const coverDoc = pages.getContents()[0].doc;
    const graphic = coverDoc.querySelector("rect").getBoundingClientRect();
    checks.pages_svg_fits_viewport = graphic.height <= 500 && graphic.width <= 600;
    pagesHost.style.width = "450px";
    await wait(100);
    checks.pages_svg_resize_keeps_page = pages.pages === 3 && location?.index === 0;
    await pages.next();
    checks.pages_svg_advances_to_html = location?.index === 1;
    await pages.prev();
    checks.pages_html_returns_to_svg = location?.index === 0;
  } finally {
    pages.destroy(); pagesHost.remove();
    URL.revokeObjectURL(cover); URL.revokeObjectURL(chapter);
  }
  globalThis.readeroContinuousChecks = checks;
})().catch((error) => {
  globalThis.readeroContinuousChecks = { error: String(error) };
});
void 0;
