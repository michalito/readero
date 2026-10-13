import * as CFI from "./foliate/epubcfi.js";
import { locatorFor, textIndex } from "./anchors.js";

const MAX_BYTES = 65536;
const fail = () => {
  throw new Error(
    "This passage needs relinking. Its words could not be identified confidently.",
  );
};
const textRoot = (doc) => doc.body ?? doc.documentElement;
const sameRange = (a, b) =>
  a &&
  b &&
  a.startContainer === b.startContainer &&
  a.startOffset === b.startOffset &&
  a.endContainer === b.endContainer &&
  a.endOffset === b.endOffset;

// Availability needs only one selected non-whitespace character. Do not build
// the full quotation while the user is dragging a potentially large range.
function hasSelectedText(range) {
  const root = range.commonAncestorContainer;
  const hasText = (node) => {
    if (node.parentElement?.closest("script,style")) return false;
    const start = node === range.startContainer ? range.startOffset : 0;
    const end = node === range.endContainer ? range.endOffset : node.length;
    return /\S/u.test(node.data.slice(start, end));
  };
  if (root.nodeType === Node.TEXT_NODE) return hasText(root);
  const walker = root.ownerDocument.createTreeWalker(
    root,
    NodeFilter.SHOW_TEXT,
  );
  let first = range.startContainer;
  if (first.nodeType !== Node.TEXT_NODE) {
    const child = first.childNodes[range.startOffset];
    if (child) first = child;
    else while (first.lastChild) first = first.lastChild;
  }
  walker.currentNode = first;
  let node = first.nodeType === Node.TEXT_NODE ? first : walker.nextNode();
  for (; node; node = walker.nextNode()) {
    if (range.comparePoint(node, 0) > 0) break;
    if (range.comparePoint(node, node.length) >= 0 && hasText(node))
      return true;
  }
  return false;
}

function stream(doc) {
  const index = textIndex(textRoot(doc));
  if (index.text === undefined)
    index.text = index.entries.map((e) => e.node.textContent).join("");
  return index;
}
function offsetAt(doc, node, offset) {
  const index = stream(doc);
  if (node.nodeType === Node.TEXT_NODE && index.offsets.has(node))
    return index.offsets.get(node) + offset;
  const point = doc.createRange();
  point.setStart(node, offset);
  point.collapse(true);
  for (const entry of index.entries) {
    const end = point.comparePoint(entry.node, entry.node.length);
    if (end === 0) return entry.offset + entry.node.length;
    if (end > 0) return entry.offset;
  }
  return index.length;
}
function rangeAtOffsets(doc, start, end) {
  const index = stream(doc);
  if (start < 0 || end <= start || end > index.length) return null;
  const point = (offset) => {
    let low = 0,
      high = index.entries.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (index.entries[mid].offset + index.entries[mid].node.length < offset)
        low = mid + 1;
      else high = mid;
    }
    const entry = index.entries[low];
    return entry ? [entry.node, offset - entry.offset] : null;
  };
  const a = point(start),
    b = point(end);
  if (!a || !b) return null;
  const range = doc.createRange();
  range.setStart(...a);
  range.setEnd(...b);
  return range;
}
function capture(book, doc, index, range, revision) {
  if (!range || range.collapsed)
    throw new Error("Select text in the document first.");
  const start = offsetAt(doc, range.startContainer, range.startOffset);
  const end = offsetAt(doc, range.endContainer, range.endOffset);
  const text = stream(doc).text;
  const quote = text.slice(start, end);
  if (!quote.trim())
    throw new Error("Select nonempty text in the document first.");
  if (new TextEncoder().encode(quote).length > MAX_BYTES)
    throw new Error("Select a shorter passage (at most 64 KiB of text).");
  if (quote !== range.toString())
    throw new Error(
      "This selection includes unsupported content. Select one text passage.",
    );
  // CFI offsets address text nodes. Element offsets count child nodes and do
  // not round-trip through the CFI parser as character endpoints.
  const canonical = rangeAtOffsets(doc, start, end);
  if (!canonical || canonical.toString() !== quote)
    throw new Error("This selection cannot be represented as a text range.");
  return {
    locator: locatorFor(book, index, canonical),
    passage: {
      kind: "reflow",
      quote,
      revision,
      href: book.sections[index].id,
      cfi: CFI.joinIndir(book.sections[index].cfi, CFI.fromRange(canonical)),
      start,
      end,
      prefix: Array.from(text.slice(Math.max(0, start - 256), start))
        .slice(-128)
        .join(""),
      suffix: Array.from(text.slice(end, end + 256))
        .slice(0, 128)
        .join(""),
    },
  };
}

export class Passages {
  constructor(book, revision, send) {
    this.book = book;
    this.revision = revision;
    this.send = send;
    this.serial = 0;
    this.selection = null;
    this.disposed = false;
    this.marked = new Set();
    this.fallback = null;
  }
  attach(doc, index) {
    doc.addEventListener("selectionchange", () => {
      if (this.disposed) return;
      const selection = doc.getSelection();
      const range =
        selection?.rangeCount === 1 && !selection.isCollapsed
          ? selection.getRangeAt(0).cloneRange()
          : null;
      if (this.fallback?.doc === doc && sameRange(range, this.fallback.range))
        return;
      // Only text belonging to this live source document can be captured.
      if (
        range &&
        textRoot(doc).contains(range.commonAncestorContainer) &&
        hasSelectedText(range)
      ) {
        this.selection = { doc, index, range };
      } else if (range || this.selection?.doc === doc || doc.hasFocus())
        this.selection = null;
      // A background chapter's delayed deselection must not clear a newer
      // selection in the focused chapter. User interaction there does clear it.
      else return;
      this.clearMark();
      this.send({
        type: "selection",
        serial: ++this.serial,
        available: !!this.selection,
      });
    });
    doc.addEventListener("wheel", () => this.clear(), { passive: true });
  }
  clearMark() {
    for (const doc of this.marked)
      doc.defaultView?.CSS?.highlights?.delete("readero-passage");
    this.marked.clear();
    if (this.fallback) {
      const { doc, range } = this.fallback;
      this.fallback = null;
      const selection = doc.getSelection();
      if (
        selection?.rangeCount === 1 &&
        sameRange(selection.getRangeAt(0), range)
      )
        selection.removeAllRanges();
    }
  }
  clear() {
    this.clearMark();
    this.selection = null;
    this.send({ type: "selection", serial: ++this.serial, available: false });
  }
  capture(serial) {
    if (serial !== this.serial || !this.selection || this.disposed)
      throw new Error("The selection changed. Select the passage again.");
    const { doc, index, range } = this.selection;
    return capture(this.book, doc, index, range, this.revision);
  }
  async resolve(passage, live) {
    if (passage.kind !== "reflow" || !passage.quote?.trim()) fail();
    const index = this.book.sections.findIndex((s) => s.id === passage.href);
    if (index < 0) fail();
    const doc =
      live.find((c) => c.index === index)?.doc ??
      (await this.book.sections[index].createDocument());
    if (this.disposed) fail();
    const text = stream(doc).text;
    const matches = (range) => {
      if (!range || range.collapsed || range.toString() !== passage.quote)
        return false;
      const start = offsetAt(doc, range.startContainer, range.startOffset);
      const end = offsetAt(doc, range.endContainer, range.endOffset);
      return (
        text.slice(Math.max(0, start - passage.prefix.length), start) ===
          passage.prefix &&
        text.slice(end, end + passage.suffix.length) === passage.suffix
      );
    };
    let range = null;
    try {
      const resolved = this.book.resolveCFI(passage.cfi);
      if (resolved?.index === index) range = resolved.anchor(doc);
    } catch {
      /* Contextual recovery below. */
    }
    let recovered = !matches(range);
    if (recovered) {
      let found = -1,
        offset = 0,
        visited = 0;
      while (offset <= text.length) {
        const start = text.indexOf(passage.quote, offset);
        if (start < 0) break;
        const end = start + passage.quote.length;
        if (
          text.slice(Math.max(0, start - passage.prefix.length), start) ===
            passage.prefix &&
          text.slice(end, end + passage.suffix.length) === passage.suffix
        ) {
          if (found >= 0) fail();
          found = start;
        }
        offset = start + 1;
        if (++visited % 100 === 0) {
          await new Promise((resolve) => setTimeout(resolve, 0));
          if (this.disposed) fail();
        }
      }
      if (found < 0) fail();
      range = rangeAtOffsets(doc, found, found + passage.quote.length);
      if (!matches(range)) fail();
    }
    return {
      ...capture(this.book, doc, index, range, this.revision),
      recovered,
    };
  }
  mark(doc, passage) {
    this.clearMark();
    const resolved = this.book.resolveCFI(passage.cfi);
    const range = resolved.anchor(doc);
    if (range.toString() !== passage.quote) fail();
    const win = doc.defaultView;
    if (win.CSS?.highlights && win.Highlight) {
      if (!doc.getElementById("readero-passage-style")) {
        const style = doc.createElement("style");
        style.id = "readero-passage-style";
        style.textContent =
          "::highlight(readero-passage) { background-color:rgba(53,132,228,.32); }";
        (doc.head ?? doc.documentElement).append(style);
      }
      win.CSS.highlights.set("readero-passage", new win.Highlight(range));
      this.marked.add(doc);
    } else {
      this.fallback = { doc, range: range.cloneRange() };
      const selection = doc.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
  }
  dispose() {
    this.disposed = true;
    this.selection = null;
    this.clearMark();
  }
}
