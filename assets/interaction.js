/** Reader-owned affordances. Publication content cannot call the native bridge. */
export class ImageInspector {
  constructor(root) {
    this.root = root;
    this.dialog = null;
    this.source = null;
  }
  open(source) {
    this.close();
    this.source = source;
    const dialog = document.createElement("div");
    dialog.className = "image-inspector";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", source.alt || "Image");
    const copy = document.createElement("img");
    copy.src = source.src;
    copy.alt = source.alt;
    const close = document.createElement("button");
    close.textContent = "Close image · Esc";
    close.onclick = () => this.close();
    dialog.addEventListener("keydown", (event) => {
      if (event.key === "Tab") {
        event.preventDefault();
        close.focus();
      }
    });
    dialog.append(copy, close);
    for (const child of this.root.children) child.inert = true;
    this.root.append(dialog);
    this.dialog = dialog;
    close.focus();
  }
  close() {
    if (!this.dialog) return false;
    this.dialog.remove();
    this.dialog = null;
    for (const child of this.root.children) child.inert = false;
    if (this.source?.isConnected) this.source.focus({ preventScroll: true });
    this.source = null;
    return true;
  }
}

export function handlesOwnKeys(target) {
  return !!target.closest?.(
    "input,textarea,select,button,a[href],[contenteditable],pre,table",
  );
}

export function prepareContent(doc, inspect) {
  for (const block of doc.querySelectorAll("pre,table")) {
    block.tabIndex = 0;
    if (block.tagName.toLowerCase() === "pre") {
      block.setAttribute("role", "region");
      block.setAttribute("aria-label", "Code block");
    }
  }
  for (const image of doc.querySelectorAll("img")) {
    // Linked images keep the link's own keyboard behavior.
    if (image.closest("a[href]")) continue;
    image.tabIndex = 0;
    image.setAttribute("role", "button");
    image.setAttribute(
      "aria-label",
      image.alt ? `Enlarge image: ${image.alt}` : "Enlarge image",
    );
    image.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        event.stopPropagation();
        if (event.isTrusted) inspect(image);
      }
    });
  }
}
