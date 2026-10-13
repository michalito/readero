# Readéro

Readéro is a local document reader that preserves the reader's place and saved passages.

## Language

**Document**:
A PDF, DRM-free reflowable EPUB, or Markdown work opened by the reader. Moving its source file does not make it a different document.

**Reading state**:
The remembered reading position and appearance of a document, together with its bookmarks and presence in recents.

**Reading position**:
The passage currently being read, independent of whether the reader uses Scroll or Pages.
_Avoid_: Screen page (as passage identity)

**Locator**:
A saved reference to a position in a document. It may recover a nearby passage when the source changes, in which case restoration is approximate.

**Bookmark**:
A named reference to a passage that the reader explicitly saves. Removing a document from recents preserves its bookmarks.

**Locate**:
The action that reconnects a known document to its moved source file while preserving its reading state.

**Notebook**:
The continuous text the reader writes for one document, together with its saved passages and connections.
_Avoid_: Workspace, note-card collection

**Saved passage**:
A source destination explicitly kept for a document. It may identify a reading location alone or preserve a quotation selected from the source.
_Avoid_: Highlight (unless an actual highlight annotation is meant)

**Quotation**:
The source text captured when the reader saves a selection. It is distinct from the reader's own notebook text and from a passage's editable name.

**Notebook section**:
A span of notebook text the reader identifies for connections to saved passages. Its identity does not depend on its wording or on a heading.

**Connection**:
An association between a notebook section and a saved passage. Removing the association does not remove either the reader's text or the saved passage.

**Relink**:
The explicit action that reconnects a saved quotation to its source when its previous destination can no longer be identified confidently.
