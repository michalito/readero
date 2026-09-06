import { EPUB } from "./foliate/epub.js";
import "./foliate/paginator.js";
import { Continuous } from "./continuous.js";
import {
  ImageInspector,
  prepareContent,
  handlesOwnKeys,
} from "./interaction.js";
import { locatorFor, resolveLocator, rangeRect } from "./anchors.js";
import { searchBook } from "./search.js";

const nextFrame = () =>
  new Promise((resolve) => requestAnimationFrame(resolve));
const contentPolicy =
  "default-src 'none'; script-src 'none'; style-src 'unsafe-inline' blob: readero:; img-src blob: data: readero:; font-src blob: data: readero:; media-src 'none'; frame-src 'none'; object-src 'none'; connect-src 'none'; base-uri 'none'; form-action 'none'";

/** Defense in depth: no authored execution, nested browsing, forms or network. */
export function sanitize(text, type, resourceBase) {
  const doc = new DOMParser().parseFromString(text, type);
  if (doc.querySelector("parsererror")) {
    if (type === "image/svg+xml") return "";
    return sanitize(text, "text/html", resourceBase);
  }
  for (const node of doc.querySelectorAll(
    "script,iframe,frame,frameset,object,embed,base,form,foreignObject,meta[http-equiv]",
  ))
    node.remove();
  for (const node of doc.querySelectorAll("*")) {
    for (const attr of [...node.attributes]) {
      const name = attr.localName.toLowerCase();
      if (
        name.startsWith("on") ||
        ["srcdoc", "action", "formaction", "ping"].includes(name)
      )
        node.removeAttributeNode(attr);
      else if (
        ["href", "src", "data", "poster"].includes(name) &&
        /^(?:\s*(?:javascript|vbscript|file):)/i.test(attr.value)
      )
        node.removeAttributeNode(attr);
    }
    // Markdown sidecars have no EPUB manifest. Resolve them through the
    // native directory grant; URL normalization never widens that grant.
    if (resourceBase && node.hasAttribute("src")) {
      const src = node.getAttribute("src");
      if (!/^(?:[a-z]+:|\/\/)/i.test(src))
        node.setAttribute("src", new URL(src, resourceBase).href);
    }
  }
  if (type !== "image/svg+xml") {
    const head = doc.head ?? doc.querySelector("head");
    const csp = doc.createElementNS("http://www.w3.org/1999/xhtml", "meta");
    csp.setAttribute("http-equiv", "Content-Security-Policy");
    csp.setAttribute("content", contentPolicy);
    head?.prepend(csp);
  }
  return new XMLSerializer().serializeToString(doc);
}

function readingStyles(settings) {
  const palettes = {
    light: ["#fffefa", "#292b29", "#eaece5", "#40684b"],
    warm: ["#f5efdf", "#3d372d", "#e5ddcc", "#536944"],
    dark: ["#202520", "#e2e6dc", "#343d33", "#acc6a4"],
  };
  const [paper, ink, border, accent] =
    palettes[settings.palette] ?? palettes.light;
  const family =
    settings.font === "sans"
      ? "system-ui,sans-serif"
      : 'Charter,"Noto Serif",Georgia,serif';
  const override = settings.font !== "publisher";
  return `html { color-scheme: ${settings.palette === "dark" ? "dark" : "light"}; background: ${paper} !important; color: ${ink} !important; }
        body { color:${ink} !important; background:${paper} !important; overflow-wrap:break-word; }
        ${
          override
            ? `body { font-family:${family} !important; font-size:${settings.font_size}px !important; line-height:${settings.line_height} !important; }
        p,li,dd,dt,blockquote { font-family:inherit !important; line-height:${settings.line_height} !important; }
        p { text-align:start; margin-block:0 1em; }`
            : ""
        }
        h1,h2,h3,h4,h5,h6 { color:inherit; line-height:1.25; text-wrap:pretty; break-after:avoid; }
        h1 { font-size:1.85em; } h2 { font-size:1.4em; } h3 { font-size:1.15em; }
        a { color:${accent}; text-underline-offset:.18em; }
        img,svg { max-width:100% !important; height:auto; object-fit:contain; }
        img { cursor:zoom-in; } figure { max-width:100%; margin-inline:0; }
        pre { font: .78em/1.55 "DejaVu Sans Mono",monospace !important; white-space:pre !important; overflow-x:auto !important; max-width:100%; padding:16px; border:1px solid ${border}; border-radius:6px; background:${paper}; color:${ink}; }
        ${settings.mode === "pages" ? "pre,table { max-height:75vh; overflow-y:auto; break-inside:avoid; }" : ""}
        pre code { white-space:inherit !important; } code { font-family:"DejaVu Sans Mono",monospace; font-size:.85em; }
        table { display:block; max-width:100%; overflow-x:auto; border-collapse:collapse; font-size:.9em; }
        th,td { border:1px solid ${border}; padding:8px 12px; } blockquote { border-inline-start:3px solid ${border}; padding-inline-start:20px; margin-inline:0; }
        ::selection { background:${accent}44; } :focus-visible { outline:2px solid ${accent}; outline-offset:3px; }
        * { animation:none !important; transition:none !important; }`;
}

export async function start(config, nativeSend) {
  const send = (message) =>
    nativeSend(JSON.stringify({ ...message, generation: config.generation }));
  let settings = config.settings,
    current = config.locator,
    renderer = null,
    stable = false,
    searchGeneration = 0;
  let disposed = false,
    layoutSerial = Promise.resolve(),
    settingsPending = false,
    layoutLocator = null;
  const root = document.getElementById("reader");
  const inspector = new ImageInspector(root);
  const base = `readero://${config.host}/`;
  const sizes = new Map(
    config.entries.map((entry) => [entry.name, entry.size]),
  );
  const response = async (name) => {
    const url = base + name.split("/").map(encodeURIComponent).join("/");
    const result = await fetch(url);
    return result.ok ? result : null;
  };
  const loadText = async (name) => {
    const result = await response(name);
    if (!result) return null;
    let text = await result.text();
    const type = /\.svg$/i.test(name)
      ? "image/svg+xml"
      : /\.(xhtml|html|htm)$/i.test(name)
        ? "application/xhtml+xml"
        : null;
    if (type)
      text = sanitize(text, type, config.format === "markdown" ? base : null);
    return text;
  };
  const book = await new EPUB({
    loadText,
    loadBlob: async (name) => (await response(name))?.blob(),
    getSize: (name) => sizes.get(name) ?? 0,
  }).init();
  if (!book.sections.length)
    throw new Error("This publication has no readable sections.");
  if (book.rendition?.layout === "pre-paginated")
    throw new Error("Fixed-layout EPUBs are not supported in this version.");
  book.transformTarget.addEventListener("load", (event) => {
    if (event.detail.isScript) event.detail.allow = false;
  });

  const toc = [];
  const pending = (book.toc ?? [])
    .map((item) => ({ item, depth: 0 }))
    .reverse();
  while (pending.length && toc.length < 3000) {
    const { item, depth } = pending.pop();
    toc.push({
      label: String(item.label ?? "Untitled").slice(0, 300),
      href: item.href ?? "",
      depth,
    });
    if (depth < 20)
      for (const child of [...(item.subitems ?? [])].reverse())
        pending.push({ item: child, depth: depth + 1 });
  }
  const onLocation = ({ detail }) => {
    const locator = locatorFor(
      book,
      detail.index,
      detail.range,
      detail.fraction,
    );
    if (!locator) return;
    if (!stable) {
      layoutLocator = locator;
      return;
    }
    if (detail.reason === "anchor" && current) return;
    current = locator;
    send({
      type: "location",
      locator,
      section: detail.index + 1,
      total: book.sections.length,
    });
  };
  const onDocument = (doc, index) => {
    doc.addEventListener("click", (event) => {
      const link = event.target.closest?.("a[href]");
      if (link) {
        event.preventDefault();
        if (!event.isTrusted) return;
        const href = link.getAttribute("href");
        if (/^(https?:|mailto:)/i.test(href)) {
          send({ type: "external", href });
          return;
        }
        if (/^[a-z]+:/i.test(href)) return;
        const target = book.sections[index].resolveHref(href);
        const resolved = book.resolveHref(target);
        if (resolved?.index >= 0) {
          send({ type: "jump", locator: current });
          queue(() => navigate(resolved));
        } else if (
          config.format === "markdown" &&
          /\.(md|markdown)(?:#|$)/i.test(href)
        )
          send({ type: "related", href });
        return;
      }
      const image = event.target.closest?.("img");
      if (image && event.isTrusted) inspector.open(image);
    });
    prepareContent(doc, (image) => inspector.open(image));
    doc.addEventListener("keydown", (event) => {
      if (
        event.ctrlKey ||
        event.altKey ||
        event.metaKey ||
        handlesOwnKeys(event.target)
      )
        return;
      if (
        settings.mode === "pages" &&
        ["ArrowRight", "PageDown", "ArrowLeft", "PageUp", " "].includes(
          event.key,
        )
      ) {
        event.preventDefault();
        queue(() =>
          ["ArrowLeft", "PageUp"].includes(event.key) || event.shiftKey
            ? renderer.prev()
            : renderer.next(),
        );
      }
    });
  };
  function queue(action) {
    layoutSerial = layoutSerial
      .then(() => (disposed ? null : action()))
      .catch((error) => send({ type: "error", message: error.message }));
    return layoutSerial;
  }
  function reportRecovery(target) {
    if (target.recovery === "approximate")
      send({
        type: "notice",
        message: "The document changed. Restored a nearby passage.",
      });
  }
  async function navigate(target, locator = null) {
    stable = false;
    layoutLocator = null;
    await renderer.goTo(target);
    await nextFrame();
    await nextFrame();
    current =
      target.recovery === "approximate"
        ? (layoutLocator ?? current)
        : (locator ?? layoutLocator ?? current);
    reportRecovery(target);
    stable = true;
    if (current)
      send({
        type: "location",
        locator: current,
        section: current.section + 1,
        total: book.sections.length,
      });
  }
  async function layout(locator = current, changed = false) {
    stable = false;
    layoutLocator = null;
    renderer?.destroy();
    inspector.close();
    root.replaceChildren();
    document.documentElement.dataset.palette = settings.palette;
    root.style.setProperty("--reading-width", `${settings.width}px`);
    root.dataset.mode = settings.mode;
    if (settings.mode === "scroll") {
      renderer = new Continuous(book, readingStyles(settings), onDocument);
      root.append(renderer.element);
      renderer.addEventListener("reader-error", (event) =>
        send({ type: "error", message: event.detail }),
      );
    } else {
      renderer = document.createElement("foliate-paginator");
      renderer.setAttribute("max-column-count", "1");
      renderer.setAttribute("max-inline-size", `${settings.width}px`);
      renderer.setAttribute("margin", "30px");
      renderer.addEventListener("load", ({ detail }) =>
        onDocument(detail.doc, detail.index),
      );
      root.append(renderer);
      renderer.open(book);
      renderer.setStyles(readingStyles(settings));
    }
    renderer.addEventListener("relocate", onLocation);
    const target = resolveLocator(book, locator, changed);
    await renderer.goTo(target);
    await nextFrame();
    await nextFrame();
    current =
      changed || target.recovery === "approximate"
        ? (layoutLocator ?? locator)
        : (locator ?? layoutLocator);
    reportRecovery(target);
    stable = true;
    if (current)
      send({
        type: "location",
        locator: current,
        section: current.section + 1,
        total: book.sections.length,
      });
  }
  async function search(query) {
    const generation = ++searchGeneration;
    return searchBook(
      book,
      query,
      send,
      () => generation !== searchGeneration || disposed,
      () =>
        send({
          type: "notice",
          message: "Some sections could not be searched.",
        }),
    );
  }
  const command = async (message) => {
    switch (message.type) {
      case "next":
        return queue(() => renderer.next());
      case "previous":
        return queue(() => renderer.prev());
      case "settings":
        settings = message.settings;
        if (settingsPending) return;
        settingsPending = true;
        return queue(async () => {
          settingsPending = false;
          await layout();
        });
      case "goto":
        return queue(() =>
          navigate(resolveLocator(book, message.locator), message.locator),
        );
      case "href": {
        const target = book.resolveHref(message.href);
        if (target?.index >= 0) return queue(() => navigate(target));
        break;
      }
      case "search":
        return search(message.query);
      case "snapshot":
        if (current)
          send({
            type: "location",
            locator: current,
            section: current.section + 1,
            total: book.sections.length,
          });
        break;
      case "escape":
        send({ type: "escape", handled: inspector.close() });
        break;
      case "dispose":
        disposed = true;
        stable = false;
        ++searchGeneration;
        inspector.close();
        renderer.destroy();
        book.destroy();
        break;
    }
  };
  // This global lives only in the named WebKit script world.
  globalThis.readeroCommand = command;
  globalThis.readeroCheckpoint = async () => {
    await layoutSerial;
    await renderer.maintenance;
    await nextFrame();
    await nextFrame();
    if (disposed || !stable)
      throw new Error("The reading position is still changing.");
    return current;
  };
  function anchorVisible(locator) {
    try {
      const target = resolveLocator(book, locator);
      const content = renderer
        .getContents()
        .find((item) => item.index === target.index);
      if (!content) return false;
      const anchor = target.anchor(content.doc);
      const bounds = content.doc.documentElement.getBoundingClientRect();
      const rect = typeof anchor === "number"
        ? new DOMRect(bounds.left, bounds.top + anchor * bounds.height, bounds.width, 1)
        : rangeRect(anchor);
      const frame =
        content.doc.defaultView.frameElement.getBoundingClientRect();
      const viewport = root.getBoundingClientRect();
      return (
        !!rect &&
        rect.left + frame.left < viewport.right &&
        rect.right + frame.left >= viewport.left &&
        rect.top + frame.top < viewport.bottom &&
        rect.bottom + frame.top >= viewport.top
      );
    } catch {
      return false;
    }
  }
  globalThis.readeroDiagnostics = () => ({
    mode: settings.mode,
    locator: current,
    mounted: renderer.getContents().length,
    stable: stable && !renderer.busy && !renderer.tick,
    sections: book.sections.length,
    anchorVisible: anchorVisible(current),
  });
  await layout(config.locator, config.changed);
  const title = (
    typeof book.metadata.title === "string" ? book.metadata.title : config.title
  ).slice(0, 600);
  send({
    type: "ready",
    title,
    toc,
    locator: current,
    total: book.sections.length,
  });
}
