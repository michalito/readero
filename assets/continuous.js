import { rangeAt, rangeRect } from "./anchors.js";

const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));

/** A bounded, adjacent chapter window. Publisher CSS stays inside each frame. */
export class Continuous extends EventTarget {
  constructor(book, style, onDocument) {
    super();
    this.book = book;
    this.style = style;
    this.onDocument = onDocument;
    this.element = document.createElement("div");
    this.element.className = "continuous";
    this.element.tabIndex = 0;
    this.frames = new Map();
    this.index = 0;
    this.disposed = false;
    this.busy = false;
    this.maintenance = null;
    this.navigating = false;
    this.pendingLoads = new Set();
    this.suppress = true;
    this.tick = 0;
    this.element.addEventListener(
      "scroll",
      () => {
        if (this.suppress || this.navigating) return;
        // Update immediately: chapter loads can finish before the next frame.
        this.anchor = this.capture();
        if (this.tick) return;
        this.tick = requestAnimationFrame(() => {
          this.tick = 0;
          this.relocate();
          this.maintain();
        });
      },
      { passive: true },
    );
    this.resize = new ResizeObserver(() => this.compensate());
  }
  capture() {
    const top = this.element.getBoundingClientRect().top + 24;
    const mounted = [...this.frames.values()].sort((a, b) => a.index - b.index);
    const item =
      mounted.find((item) => item.frame.getBoundingClientRect().bottom > top) ??
      mounted.at(-1);
    if (!item?.frame.contentDocument?.documentElement) return null;
    const rect = item.frame.getBoundingClientRect();
    const range = rangeAt(
      item.frame.contentDocument,
      Math.min(rect.width / 3, 120),
      Math.max(1, top - rect.top),
    );
    if (!range) return null;
    const y = rangeRect(range)?.top ?? 0;
    return {
      index: item.index,
      range,
      screenY: rect.top + y,
      scrollTop: this.element.scrollTop,
      fraction: y / Math.max(1, rect.height),
    };
  }
  compensate(anchor = this.anchor) {
    if (!anchor || this.disposed) return;
    const item = this.frames.get(anchor.index);
    if (!item || !anchor.range.startContainer.isConnected) return;
    const suppressed = this.suppress;
    this.suppress = true;
    const y =
      item.frame.getBoundingClientRect().top +
      (rangeRect(anchor.range)?.top ?? 0);
    // Scrolling can precede delivery of its DOM event. Account for that motion
    // as well as anchors refreshed by the scroll listener.
    anchor.screenY -= this.element.scrollTop - anchor.scrollTop;
    this.element.scrollTop += y - anchor.screenY;
    anchor.scrollTop = this.element.scrollTop;
    this.suppress = suppressed;
  }
  async mount(index) {
    if (this.frames.has(index) || !this.book.sections[index] || this.disposed)
      return;
    const section = this.book.sections[index];
    const src = await section.load();
    if (this.disposed) {
      section.unload?.();
      return;
    }
    const iframe = document.createElement("iframe");
    iframe.className = "chapter";
    iframe.title = `Section ${index + 1}`;
    // WebKit needs allow-scripts for parent-owned event listeners (218086).
    // Authored scripts are disabled by WebKit settings, sanitization and
    // the chapter's script-src 'none' policy. Native IPC is in a private world.
    iframe.setAttribute("sandbox", "allow-same-origin allow-scripts");
    iframe.setAttribute("scrolling", "no");
    const loaded = new Promise((resolve, reject) => {
      let timer;
      const cleanup = () => {
        clearTimeout(timer);
        this.pendingLoads.delete(cancel);
        iframe.removeEventListener("load", success);
        iframe.removeEventListener("error", failure);
      };
      const cancel = () => {
        cleanup();
        resolve(false);
      };
      const success = () => {
        cleanup();
        resolve(true);
      };
      const failure = () => {
        cleanup();
        reject(new Error("A chapter could not be displayed."));
      };
      timer = setTimeout(failure, 15000);
      this.pendingLoads.add(cancel);
      iframe.addEventListener("load", success, { once: true });
      iframe.addEventListener("error", failure, { once: true });
    });
    iframe.src = src;
    const before = [...this.frames.values()]
      .filter((x) => x.index > index)
      .sort((a, b) => a.index - b.index)[0];
    this.compensate();
    this.element.insertBefore(iframe, before?.frame ?? null);
    const item = { index, frame: iframe, observer: null, css: null };
    this.frames.set(index, item);
    // Insertion itself shifts following chapters, before the iframe loads.
    if (this.anchor) this.anchor.scrollTop = this.element.scrollTop;
    this.compensate();
    if (!(await loaded) || this.disposed) return;
    const doc = iframe.contentDocument;
    const root = doc?.body ?? doc?.documentElement;
    if (!root) throw new Error("This chapter has no readable content.");
    const svg = root.namespaceURI === "http://www.w3.org/2000/svg";
    const css = doc.createElementNS(
      svg ? root.namespaceURI : "http://www.w3.org/1999/xhtml",
      "style",
    );
    css.textContent = this.style;
    (doc.head ?? root).append(css);
    item.css = css;
    doc.documentElement.style.setProperty("overflow", "hidden", "important");
    root.style.setProperty("margin", "0", "important");
    // SVG documents have no body. Fit their intrinsic aspect ratio to the
    // chapter width instead of measuring a viewport-dependent scrollHeight.
    const box = svg ? root.viewBox.baseVal : null;
    const width = box?.width || (svg && root.width.baseVal.value) || 300;
    const height = box?.height || (svg && root.height.baseVal.value) || 150;
    if (svg) {
      root.style.setProperty("width", "100%", "important");
      root.style.setProperty("height", "100%", "important");
    }
    const size = () => {
      if (this.disposed) return;
      this.compensate();
      const measured = Math.ceil(
        svg
          ? (iframe.getBoundingClientRect().width * height) / width
          : Math.max(root.scrollHeight, root.getBoundingClientRect().height),
      );
      if (Math.abs(iframe.getBoundingClientRect().height - measured) > 1)
        iframe.style.height = `${Math.max(1, measured)}px`;
      // Browser clamping caused by our resize is not a reader scroll.
      if (this.anchor) this.anchor.scrollTop = this.element.scrollTop;
      this.compensate();
    };
    item.observer = new ResizeObserver(size);
    item.observer.observe(root);
    await Promise.race([
      doc.fonts.ready,
      new Promise((r) => setTimeout(r, 1500)),
    ]);
    this.onDocument(doc, index);
    size();
    this.resize.observe(iframe);
    await frame();
  }
  async goTo({ index, anchor }) {
    this.navigating = true;
    this.suppress = true;
    try {
      // A scroll-triggered preload must finish before an intentional jump
      // replaces its frame window. Otherwise its late completion moves the jump.
      await this.maintenance;
      if (this.disposed) return;
      this.anchor = null;
      for (const key of [...this.frames.keys()]) this.unmount(key);
      this.index = index;
      await this.mount(index);
      await this.maintain(false);
      if (this.disposed) return;
      const item = this.frames.get(index);
      const target =
        typeof anchor === "function"
          ? anchor(item.frame.contentDocument)
          : anchor;
      const y =
        typeof target === "number"
          ? target * item.frame.offsetHeight
          : (rangeRect(target)?.top ?? 0);
      // Mount following content before alignment so the bottom of a chapter
      // does not prematurely clamp the restored passage to the viewport edge.
      this.element.scrollTop =
        item.frame.offsetTop - this.element.offsetTop + y - 24;
      await frame();
    } finally {
      this.navigating = false;
      this.suppress = false;
    }
    if (!this.disposed) this.relocate();
  }
  maintain(preserve = true) {
    if (this.maintenance) return this.maintenance;
    if (this.disposed || (this.navigating && preserve))
      return Promise.resolve();
    this.busy = true;
    this.maintenance = this.maintainWindow(preserve)
      .catch((error) => {
        if (!this.disposed)
          this.dispatchEvent(
            new CustomEvent("reader-error", { detail: error.message }),
          );
      })
      .finally(() => {
        this.busy = false;
        this.maintenance = null;
      });
    return this.maintenance;
  }
  async maintainWindow(preserve) {
    const center = this.index;
    const start = Math.max(
      0,
      Math.min(center - 2, this.book.sections.length - 5),
    );
    const end = Math.min(this.book.sections.length - 1, start + 4);
    const anchor = preserve ? this.capture() : null;
    this.anchor = anchor;
    // Evict before loading replacements: both frames and decoded resources
    // stay bounded while traversing a long publication.
    for (const index of this.frames.keys())
      if (index < start || index > end) this.unmount(index);
    if (anchor) anchor.scrollTop = this.element.scrollTop;
    this.compensate(anchor);
    for (let index = start; index <= end; index++) {
      if (this.disposed) return;
      await this.mount(index);
      // A reader may have scrolled while mount was awaiting the chapter.
      this.compensate();
    }
    if (preserve && !this.disposed) {
      this.anchor = this.capture();
      this.relocate();
    }
  }
  relocate() {
    const anchor = this.capture();
    if (!anchor) return;
    this.anchor = anchor;
    this.index = anchor.index;
    this.dispatchEvent(new CustomEvent("relocate", { detail: anchor }));
  }
  setStyles(css) {
    const anchor = this.capture();
    this.anchor = anchor;
    this.style = css;
    for (const item of this.frames.values())
      if (item.css) item.css.textContent = css;
    requestAnimationFrame(() => {
      this.compensate(anchor);
      this.relocate();
    });
  }
  next() {
    this.element.scrollBy({
      top: this.element.clientHeight * 0.86,
      behavior: "instant",
    });
  }
  prev() {
    this.element.scrollBy({
      top: -this.element.clientHeight * 0.86,
      behavior: "instant",
    });
  }
  getContents() {
    return [...this.frames.values()].map((item) => ({
      index: item.index,
      doc: item.frame.contentDocument,
    }));
  }
  unmount(index) {
    const item = this.frames.get(index);
    if (!item) return;
    item.observer?.disconnect();
    this.resize.unobserve(item.frame);
    item.frame.remove();
    this.frames.delete(index);
    this.book.sections[index].unload?.();
  }
  destroy() {
    this.disposed = true;
    for (const cancel of this.pendingLoads) cancel();
    cancelAnimationFrame(this.tick);
    this.resize.disconnect();
    for (const index of [...this.frames.keys()]) this.unmount(index);
    this.element.remove();
  }
}
