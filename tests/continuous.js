// Run against real WebKit iframe layout; no mocked geometry or renderer.
(async () => {
  const { Continuous } = await import("readero://app/continuous.js");
  const { rangeRect } = await import("readero://app/anchors.js");
  const { ImageInspector, prepareContent, handlesOwnKeys } = await import(
    "readero://app/interaction.js"
  );
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
  globalThis.readeroContinuousChecks = checks;
})().catch((error) => {
  globalThis.readeroContinuousChecks = { error: String(error) };
});
void 0;
