import * as CFI from "./foliate/epubcfi.js";

export function rangeAt(doc, x = 24, y = 24) {
  const range = doc.caretRangeFromPoint?.(x, Math.max(1, y));
  if (
    range?.startContainer.nodeType === Node.TEXT_NODE &&
    range.startContainer.textContent.trim()
  ) {
    const rect = rangeRect(range);
    if (rect?.height && rect.bottom >= y - 2 && rect.top <= y + 60)
      return range;
  }
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) =>
      node.textContent.trim() && !node.parentElement.closest("script,style")
        ? NodeFilter.FILTER_ACCEPT
        : NodeFilter.FILTER_REJECT,
  });
  let node,
    last = null;
  while ((node = walker.nextNode())) {
    const block = doc.createRange();
    block.selectNodeContents(node);
    const rect = block.getBoundingClientRect();
    if (!rect.height) continue;
    last = node;
    if (rect.bottom < y) continue;
    // Find the first character on the visible line without treating source
    // whitespace or an illustration as the start of the whole chapter.
    let low = 0,
      high = node.length;
    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      const point = doc.createRange();
      point.setStart(node, mid);
      point.setEnd(node, Math.min(mid + 1, node.length));
      if (point.getBoundingClientRect().bottom < y) low = mid + 1;
      else high = mid;
    }
    const point = doc.createRange();
    point.setStart(node, Math.min(low, node.length));
    point.collapse(true);
    return point;
  }
  if (!last) return null;
  const fallback = doc.createRange();
  fallback.setStart(last, 0);
  fallback.collapse(true);
  return fallback;
}

// DOM Range offsets are UTF-16 code units. Cache text positions by document;
// typography and image layout do not change this index.
const textIndexes = new WeakMap();
function textIndex(root) {
  let index = textIndexes.get(root);
  if (index) return index;
  const doc = root.ownerDocument ?? root;
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) =>
      node.parentElement?.closest("script,style")
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  const entries = [],
    offsets = new WeakMap();
  let length = 0,
    node;
  while ((node = walker.nextNode())) {
    offsets.set(node, length);
    entries.push({ node, offset: length });
    length += node.length;
  }
  index = { entries, offsets, length };
  textIndexes.set(root, index);
  return index;
}
function textPoint(range) {
  if (range.startContainer.nodeType === Node.TEXT_NODE)
    return { node: range.startContainer, offset: range.startOffset };
  const root = range.startContainer;
  const child = root.childNodes[range.startOffset];
  const entries = textIndex(child ?? root).entries;
  const entry = child ? entries[0] : entries.at(-1);
  if (child?.nodeType === Node.TEXT_NODE) return { node: child, offset: 0 };
  return entry
    ? { node: entry.node, offset: child ? 0 : entry.node.length }
    : null;
}
function rangeInBlock(block, offset) {
  const index = textIndex(block);
  const entry =
    index.entries.find((e) => e.offset + e.node.length > offset) ??
    index.entries.at(-1);
  if (!entry) return block;
  const range = block.ownerDocument.createRange();
  range.setStart(
    entry.node,
    Math.max(0, Math.min(entry.node.length, offset - entry.offset)),
  );
  range.collapse(true);
  return range;
}

function matchesQuote(range, locator, doc) {
  if (!locator.quote) return true; // Legacy locators may have no quote.
  const point = textPoint(range);
  if (!point) return false;
  const index = textIndex(doc.body);
  const offset = index.offsets.get(point.node);
  if (offset === undefined) return false;
  const start = offset + point.offset - (locator.quote_offset ?? 0);
  const text = index.entries.map(({ node }) => node.textContent).join("");
  return (
    start >= 0 &&
    text.slice(start, start + locator.quote.length) === locator.quote
  );
}

export function locatorFor(book, index, range, fraction = 0) {
  if (!range || !book.sections[index]) return null;
  const anchor = range.cloneRange();
  anchor.collapse(true);
  const point = textPoint(anchor);
  const element =
    anchor.startContainer.nodeType === Node.ELEMENT_NODE
      ? anchor.startContainer
      : anchor.startContainer.parentElement;
  const block = element?.closest("[data-reader-block]");
  let quote = "",
    quoteOffset = 0,
    blockOffset = null;
  if (point) {
    let begin = Math.max(0, point.offset - 32);
    let end = Math.min(point.node.length, point.offset + 128);
    if (/[\uDC00-\uDFFF]/.test(point.node.textContent[begin] ?? "")) begin--;
    if (/[\uDC00-\uDFFF]/.test(point.node.textContent[end] ?? "")) end++;
    quote = point.node.textContent.slice(begin, end);
    quoteOffset = point.offset - begin;
    const documentIndex = textIndex(point.node.ownerDocument.body);
    const position = documentIndex.offsets.get(point.node);
    if (position !== undefined && documentIndex.length)
      fraction = (position + point.offset) / documentIndex.length;
    if (block) {
      const position = textIndex(block).offsets.get(point.node);
      if (position !== undefined) blockOffset = position + point.offset;
    }
  }
  return {
    version: 1,
    kind: "reflow",
    href: book.sections[index].id,
    cfi: CFI.joinIndir(book.sections[index].cfi, CFI.fromRange(anchor)),
    section: index,
    fraction: Math.max(0, Math.min(1, fraction || 0)),
    quote,
    quote_offset: quoteOffset,
    block: block?.id ?? "",
    block_offset: blockOffset,
  };
}

export function resolveLocator(book, locator, changed = false) {
  if (!locator)
    return {
      index: Math.max(
        0,
        book.sections.findIndex((s) => s.linear !== "no"),
      ),
      anchor: () => 0,
      recovery: "start",
    };
  const resourceIndex = book.sections.findIndex((s) => s.id === locator.href);
  let resolved;
  try {
    resolved = locator.cfi ? book.resolveCFI(locator.cfi) : null;
  } catch {
    /* Repair below. */
  }
  // Resource identity wins over a stale spine index after chapter insertion.
  const index =
    resourceIndex >= 0
      ? resourceIndex
      : Math.min(Math.max(0, locator.section ?? 0), book.sections.length - 1);
  const result = {
    index,
    recovery: "cfi",
    anchor: (doc) => {
      const block = locator.block ? doc.getElementById(locator.block) : null;
      if (!changed && resolved?.index === index) {
        try {
          const range = resolved.anchor(doc);
          const element =
            range?.startContainer.nodeType === Node.ELEMENT_NODE
              ? range.startContainer
              : range?.startContainer.parentElement;
          if (
            range &&
            matchesQuote(range, locator, doc) &&
            (!locator.block ||
              element?.closest("[data-reader-block]")?.id === locator.block)
          )
            return range;
        } catch {
          /* Repair below. */
        }
      }
      if (block) {
        result.recovery = "block";
        return Number.isInteger(locator.block_offset)
          ? rangeInBlock(block, locator.block_offset)
          : block;
      }
      if (locator.quote) {
        const text = textIndex(doc.body)
          .entries.map(({ node }) => node.textContent)
          .join("");
        const offset = text.indexOf(locator.quote);
        if (offset >= 0 && text.indexOf(locator.quote, offset + 1) < 0) {
          result.recovery = "quote";
          return rangeInBlock(doc.body, offset + (locator.quote_offset ?? 0));
        }
      }
      result.recovery = "approximate";
      // A text fraction provides a nearby passage after a destructive edit.
      const text = textIndex(doc.body);
      if (text.length)
        return rangeInBlock(
          doc.body,
          Math.floor(text.length * (locator.fraction || 0)),
        );
      return Math.max(0, Math.min(1, locator.fraction || 0));
    },
  };
  return result;
}

export function rangeRect(anchor) {
  if (typeof anchor?.cloneRange === "function") {
    const rect = anchor.getBoundingClientRect();
    if (rect.height) return rect;
    const range = anchor.cloneRange();
    if (
      range.startContainer.nodeType === Node.TEXT_NODE &&
      range.startOffset < range.startContainer.length
    ) {
      range.setEnd(range.startContainer, range.startOffset + 1);
      return range.getBoundingClientRect();
    }
    return anchor.startContainer.parentElement?.getBoundingClientRect() ?? rect;
  }
  return anchor?.getBoundingClientRect?.();
}
