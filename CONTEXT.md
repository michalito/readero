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
