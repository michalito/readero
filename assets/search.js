const yieldTask = () => new Promise((resolve) => setTimeout(resolve, 0));
const blocks =
  "p,div,li,dt,dd,blockquote,h1,h2,h3,h4,h5,h6,pre,td,th,figcaption";

/** Search a chapter's text stream, retaining UTF-16 offsets into the DOM. */
export async function searchRanges(
  doc,
  query,
  limit = 500,
  cancelled = () => false,
) {
  if (!query.trim() || !doc?.body || limit <= 0) return [];
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
  const entries = [],
    chunks = [];
  let node,
    length = 0,
    visited = 0,
    previousBlock;
  while ((node = walker.nextNode())) {
    if (++visited % 1000 === 0) {
      await yieldTask();
      if (cancelled()) return [];
    }
    if (!node.length || node.parentElement?.closest("script,style")) continue;
    const block = node.parentElement?.closest(blocks);
    // Adjacent blocks must not accidentally become one word. Inline elements
    // within a block remain contiguous, including words split by emphasis.
    if (entries.length && block !== previousBlock) {
      chunks.push("\n");
      length++;
    }
    previousBlock = block;
    entries.push({ node, offset: length });
    chunks.push(node.textContent);
    length += node.length;
  }
  if (cancelled()) return [];
  const text = chunks.join("");
  const matcher = new RegExp(
    query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
    "giu",
  );
  const pointAt = (offset, end = false) => {
    let low = 0,
      high = entries.length;
    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      if (
        entries[mid].offset < offset ||
        (!end && entries[mid].offset === offset)
      )
        low = mid + 1;
      else high = mid;
    }
    const entry = entries[Math.max(0, low - 1)];
    return [entry.node, Math.min(entry.node.length, offset - entry.offset)];
  };
  const results = [];
  let match;
  while (results.length < limit && (match = matcher.exec(text))) {
    const range = doc.createRange();
    range.setStart(...pointAt(match.index));
    range.setEnd(...pointAt(match.index + match[0].length, true));
    results.push({
      range,
      label: text
        .slice(Math.max(0, match.index - 32), matcher.lastIndex + 72)
        .replace(/\s+/g, " ")
        .trim(),
    });
  }
  return results;
}
