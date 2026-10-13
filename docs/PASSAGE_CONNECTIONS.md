# Selected passages and notebook connections

Approved design · 19 September 2026 · implemented and validated on 20 September 2026.

The owner selected two additions: save text selected in a document, and connect
saved passages to particular sections of the notebook. This extends F02 before
the broader highlighting feature. Persistent document highlights, rich text,
export, and a separate notes workspace are not prerequisites.

## Behavior before this change

Attach passage calls the current reading-position path, then stores a reference
with an ID, editable name, and point locator. It does not inspect the document's
selection. A locator's short context excerpt is not a quotation selected by the
reader. Those references belonged to the notebook as a whole. The implementation
below preserves them as explicit location-only references.

## Product decisions

Keep one continuous plain-text notebook per document. A **notebook section** is
an explicitly connected span of its text: a sentence, a paragraph, or several
paragraphs. Headings are optional text, not an imposed document structure. The owner approved this span-based section model.

A **saved passage** preserves a source destination and, for a text selection,
the selected quotation. A **connection** associates a notebook section with a
saved passage. Editing your explanation does not edit the quotation. Editing a
passage's name does not change its destination.

One section can connect to several passages; one passage can support several
sections. Keep each passage once and refer to its ID. Saving a passage does not
require writing notes, and writing notes does not require a passage.

## Reader interaction

1. Select text by mouse or keyboard in the document.
2. Invoke **Save selected passage** from the selection context menu or the
   notebook's reader-source controls. The control names the selection explicitly.
3. Save the selected text and its complete range together. Show its quotation,
   source location, editable name, and save status in **Saved passages**.
4. Continue reading, or choose **Connect to notes** on the saved passage.

Saving opens the notebook if needed but does not overwrite, append to, or move
the cursor in its text. It presents the saved passage in the notebook. The default name is a
short excerpt; the full quotation remains available in an expandable detail.
The source location is always displayed separately from the editable name.

Rename today's Attach passage action to **Save current location**. Keep it as a
secondary action for illustrations, scanned pages, and occasions when a page is
enough. It creates a location-only passage, visibly without a quotation. Never
silently turn a failed selection capture into a location-only save.

Selection is captured before a source action changes focus. A lightweight
selection token includes the active document generation and selection revision;
the capture response must match both. A newly selected range replaces the
previous token. Explicit deselection, navigation, source reload, or document
switch invalidates it. Moving focus to the action's own menu/button may consume
the matching snapshot, but later commands cannot resurrect a stale selection.
Notebook selection and document selection are independent.

Capture text, endpoints, context, and navigation fallback from the same snapshot.
An empty or whitespace-only selection disables the action. Failure reports a
specific reason and preserves both notes and any existing passages. Reading
rights enforced by the reader remain enforced. The clipboard is never used as
an internal transport.

Repeated activation for the same in-flight selection is idempotent. Saving the
same range in the same source revision again selects its existing passage.
Identical words at different locations are separate passages. A later changed
quotation is not allowed to overwrite an earlier saved quotation.

## Connecting notebook text

Two equivalent entry points use the same operation:

- **Notes first:** select notebook text, choose **Connect passage**, then pick
  from this document's saved passages with quotation previews.
- **Passage first:** choose **Connect to notes** on a saved passage, then select
  the intended notebook text and press **Connect**. Escape cancels without
  changing text or connections. A pending connection is visible until resolved.

With only a caret, **Connect passage** proposes the current nonempty paragraph
and visibly selects it before confirmation. A paragraph is text between blank
lines; a wrapped display line is not a paragraph. Empty paragraphs require an
explicit selection or new text. There is no implicit "last paragraph edited"
target and no insertion at a forgotten caret position.

Connected text has a restrained underline and a small source control anchored
beside its first visible line. One connection shows its name/location; several
show **3 passages**, opening a chooser. The source control is a real accessible
button, drawn outside the text buffer, not special hidden characters or pasted
markup. A compact **Passages for this section** group provides the same actions
when the cursor is inside the section, including on narrow screens.

**One click on the section's source control opens the passage.** Ordinary clicks
in prose place the caret and dragging selects text. Ctrl+click on connected prose
also follows a single connection, or opens the chooser for several. Keyboard
users can Tab to the source control or use **Open connected passage** from the
notebook context menu. Do not add a global read/edit mode merely to make links
clickable. If the owner wants plain single-click prose navigation, that is an
interaction change to resolve explicitly, not an accidental consequence.

Following a connection uses existing Back/Forward history. It preserves the
notebook text and editing position, navigates to the range start, and marks the
verified selected words in the reading view. This temporary indication clears
on manual document navigation or a new source selection. It is not a persistent
highlight annotation. Back returns to the prior reading passage. Mouse source
buttons leave the notebook ready for continued typing; keyboard activation
retains visible focus on its accessible source control.

Use **Remove connection** to disconnect a source while preserving notes and the
saved passage. Use **Delete saved passage** for the saved item itself. If it has
connections, a small dialog shows their count and offers Cancel or **Delete and
disconnect**; it never deletes notebook text. These changes are undoable within
the live notebook session.

## Editing rules

Persist section IDs and Unicode character ranges in the same snapshot as text.
Live ranges follow edits; saved raw offsets alone are insufficient.

- Inserting before a section moves it. Inserting strictly inside extends it.
  Inserting exactly at either outer edge stays outside, so a newly adjacent
  paragraph does not accidentally acquire the connection.
- Partial deletion shrinks the section. A replacement of the entire section
  with nonempty text in one user action keeps its connections on the replacement.
- Deleting the whole section without replacement removes its connections and
  section marker, but keeps every saved passage. Undo restores the text and
  connections together; redo removes them together.
- Exact selection of an existing section adds another connection to it.
  Duplicate section/passage pairs are ignored. Partial overlap with another
  section is rejected with **Select the existing linked section or separate
  text**. Do not invent overlapping or nested editing semantics in this release.
- Editing text must not merge two section identities. If deletion removes the
  gap between them, they remain adjacent sections. A replacement crossing
  multiple sections disconnects affected sections as one undoable action.
- Copy/cut/paste uses plain text. It does not secretly move or duplicate
  connections through the system clipboard. Cut counts as deletion, so Undo can
  restore the connection. Explicit connection creation handles the pasted text.
- Resizing, source navigation, focus changes, and autosave never mutate section
  boundaries. A link cannot drift to an unrelated paragraph after restart.

The existing 100-action session undo limit remains. Text, connection metadata,
selection, and passage mutations must form coherent user actions. Do not keep
independent text and connection stacks that can fall out of step.

## Source identity and recovery

Keep point locators for ordinary reading, history, and legacy passages. A selected
passage introduces a separate **range target**, containing a quotation snapshot,
format-specific endpoints, local prefix/suffix context, source revision hint,
and a point locator for explicit fallback navigation.

The quotation is the exact Unicode string extracted by the reader at capture,
including its line breaks. It is not guaranteed to be byte-for-byte source text:
PDF extraction and rendered HTML have their own text order and whitespace. Never
trim, truncate, or normalize the stored quotation silently. Derive normalization
only for matching and keep a mapping back to the original text endpoints. No
case folding, punctuation removal, or arbitrary fuzzy match establishes an exact
connection. The W3C quote-selector model's exact/prefix/suffix distinction is a
useful precedent, not a requirement to implement a general annotation platform.

For **PDF**, use Papers' selected text plus per-page selection descriptors. Copy
page IDs, selection style, and endpoints in unrotated document coordinates.
Derive line regions through the existing selection implementation when showing
the passage. Do not store screen pixels or a single bounding box around several
lines. Multi-page selections preserve their ordered page fragments. Page, zoom,
rotation, fractional scale, and extracted text order need native validation.
Papers 50.2 disables new selections when the document is rotated. Readero offers
**Rotate upright to select** explicitly in that state. Saved passages still
resolve and display at all four rotations; the app does not invent selected text
or implement a competing PDF selection engine.

For **EPUB and Markdown**, capture a noncollapsed DOM range in the actual content
document. Use the existing CFI implementation, which already serializes ranges,
plus resource identity, both text endpoints, quotation, and context. Keep UTF-16
DOM offsets distinct from notebook Unicode character offsets. Reuse the existing
text index and resource loading instead of building a second search engine.

The current EPUB reader uses separate content documents for spine sections.
Selections spanning paragraphs in one section are supported; a native selection
cannot normally span separate iframe documents. Cross-chapter material should be
saved as separate passages and connected to the same note section. No silent
truncation of a range that the renderer cannot represent.

Resolve a selected passage to one of three explicit outcomes:

1. **Verified:** range and extracted text/context agree. Reveal its exact words.
2. **Recovered:** structural position changed, but a unique quote/context match
   in the original resource identifies the range. Reveal it and indicate that
   its position was updated. Preserve the original quotation.
3. **Needs relinking:** ambiguous, missing, unsupported, or changed beyond safe
   recovery. Show the saved quotation; offer **Open last known location** and
   **Relink to selected text**. Never highlight a merely nearby match.

For initial recovery, search the original EPUB/Markdown resource or original PDF
pages only. A missing resource, repaginated PDF, or match elsewhere requires
manual relinking. Do not scan an entire 1,200-page PDF or every EPUB chapter at
startup. The current size/mtime revision string is only a hint; always validate
the candidate range against text and context, even if that hint is unchanged.
Duplicate quote candidates without sufficient structural/context evidence remain
ambiguous. Location-only passages continue to use existing approximate-position
feedback without pretending to have a verified quotation.

Relinking previews old and new selections and requires an explicit **Relink**.
An identical quotation can replace the target while retaining identity. If the
quotation differs, **Replace passage with this selection** makes that replacement
explicit, updates all its connections together, and is undoable. Renaming alone
can never do this.

Image-only PDFs cannot supply a text quotation. Keep **Save current location**
available and explain **This page has no selectable text** when appropriate.
OCR and persistent highlight colors remain outside this addition.

## Implementation shape

Extend the existing notebook model and worker rather than adding a passage
database or a second persistence service:

- **SavedPassage:** stable ID, editable label, and a target which is either the
  existing point locator or the new quotation/range target.
- **NotebookSection:** stable ID, nonempty half-open character range, and a
  deduplicated list of passage IDs. The notebook owns sections and passages.
- **Reader selection interface:** capture the active selection and resolve/show
  a range. PDF and reflow are the two real adapters. The shell never inspects
  PDF geometry or DOM endpoints. Request generation guards stale replies.
- **Notebook editor module:** owns range transforms, editing transactions, undo,
  and projection of connected sections onto the native TextView. Its small
  interface edits text, connects/disconnects passages, follows a section, and
  produces a validated snapshot. Widget handlers do not implement those rules.

GTK marks are useful for live presentation but not sufficient for durable undo.
A local GTK experiment deleted the range [7,19), then undid the deletion: text
returned, but opposing mark gravities produced endpoints [19,7). The application
must restore semantic metadata together with the text transaction.

Plan one bounded notebook transaction history recording text splice deltas,
affected section/passage changes, and before/after selection. Keep native GTK
selection, input methods, layout, and clipboard behavior. Replace its text-only
undo ownership inside this editor instead of layering a competing metadata
stack. Route keyboard, menu, and accessible Undo/Redo through the same history.
GTK user-action boundaries group paste, replacement, and composition; normal
typing can coalesce only while adjacent and uninterrupted by selection movement,
connection actions, or a pause. Do not snapshot the whole notebook per keystroke.

This history integration is the first implementation gate: prove typing, input
method composition, cut/paste, full-range replacement, Undo/Redo, and native menu
behavior in a small editor experiment before building the rest of the feature.
Keep the experiment out of production once the module covers its cases. A native
extension that demonstrably provides one shared history is preferable if found;
an unproven parallel-stack synchronization scheme is not an acceptable shortcut.

Do not insert child-widget anchors or invisible ID characters into the text.
Source buttons live in a layout-managed gutter/overlay; only visible section
controls and underlines are materialized. Reposition on layout/scroll events, with no frame poll.
Selections sharing a displayed line share one source button; multiple distinct
passages open a chooser, so controls never obscure each other on that line.
The cursor's connection group provides an accessible, compact fallback when
gutter space is limited. Styling must not change the user's plain text.

## Persistence and migration

Notebook payload v2 adds passages and sections. Migrate each legacy reference to
a location-only saved passage with the same ID, name, and locator. Do not infer a
quotation from the existing locator excerpt or guess a note section. Preserve
every notebook character, editing position, pane dimension, and visibility flag.

Bump the database compatibility version to 3 in a transaction so older builds
refuse to open it. Accept legacy notebook payload v1 and upgrade it on load;
write v2 on its next normal save. Avoid rewriting every notebook at startup.
Newer or malformed payloads remain protected from replacement. A failed upgrade
or failed save must leave the durable old snapshot intact.

Save text, ranges, passages, and connections atomically in the existing notebook
row using the existing single worker, debounce, retained drafts, Retry, and close
protection. No independent saves that can persist new text with old ranges.
Validate IDs, referential integrity, disjoint section bounds, Unicode offsets,
finite in-bounds PDF geometry, resource identity, and capture sizes on load and
save. Removing recents and Locate retain the same document-owned information.

## Responsiveness and bounded work

Use events to report selection availability. Only materialize the full quotation
when a save action requests it; coalesce selection changes while dragging. Clear
ephemeral snapshots after consumption or invalidation. Captured source text is
rendered as text, never interpreted as markup or commands.

Initial explicit capture limits: 64 KiB of UTF-8 quotation, 32 PDF page fragments,
and one reflow content document. Context is capped at 128 Unicode characters on
each side. Oversized capture gives **Select a shorter passage** before any save;
do not silently shorten it. Validate the serialized response against the existing
2 MiB bridge limit. These are implementation bounds, not targets for UI clutter.

Resolve only the passage being opened. Cache by active source generation, not
forever by passage ID. Selection capture and recovery must be cancelable on a
document switch. Chunk reflow recovery to yield to input; PDF extraction must
respect Papers' threading rules, use only requested pages, and avoid moving live
GTK/Papers objects to arbitrary workers.

Use sorted section ranges with binary lookup and updates to affected ranges.
Start with vectors; introduce a more elaborate index only if measurements require
it. Reuse the current text snapshot for cursor-only saves. Keep quotation previews
short without shortening storage. Benchmark 124 KB and 1 MB notes with 100 and
1,000 connections, plus the existing large PDF corpus. Record p95 typing/link
latency, idle CPU, and memory before/after; any persistent idle loop or full-book
work during note editing is a release failure.

## Acceptance and implementation order

1. Prove native editor history and PDF selection capture/redisplay at 0/90/180/270
   degrees and supported display scales. Verify accessible source controls.
2. Implement the pure passage/section model, transforms, validation, transaction
   history, and lossless v1 upgrade. Test repeated text, emoji, combining marks,
   RTL, overlaps, boundary insertions, whole-section replacement, and deletion
   followed by Undo/Redo. Verify all states after a fresh process restart.
3. Add PDF and reflow selection capture with generation/revision guards. Test
   keyboard/reverse selections, multi-paragraph/multi-node ranges, multi-page
   PDF selections, unsupported cross-resource capture, blank/scanned documents,
   duplicate saves, oversized ranges, and focus changes during capture.
4. Ship Save selected passage, explicit current-location saving, both connection
   entry points, one-click source buttons, Ctrl+click, chooser, disconnect/delete,
   and exact reveal/Back as one complete user flow across all three formats.
5. Test source edits: unique recovery, duplicate quotes, removed text, moved
   Markdown blocks, EPUB spine insertions, PDF page changes, and explicit relink.
   No ambiguous case may be reported as an exact match.
6. Test autosave failure, close protection, reload, Locate, recents removal,
   document switches with delayed replies, narrow/wide layout, Focus, fractional
   scaling, and source/editor focus. Run core/native checks, strict lint, and the
   existing reading regressions; measure performance and perform human Orca and
   ordinary reading checks. Record evidence separately from design promises.

The first shippable result includes selection saving AND notebook connections.
Internal sequencing does not turn either user requirement into an optional
follow-up. See the implementation record for completed checks and runtime limitations.

## Sources checked

- Current implementation: src/notebook.rs; src/desktop/shell/notebook.rs;
  src/desktop/reflow.rs; src/desktop/pdf.rs; assets/anchors.js;
  assets/foliate/epubcfi.js; vendor/papers-view/src/auto/view.rs;
  vendor/papers-view/src/view_selection.rs; src/state.rs; src/resource.rs.
- [GTK text marks](https://docs.gtk.org/gtk4/class.TextMark.html) explain mutation
  tracking and gravity; the local experiment above checks actual undo behavior.
- [GTK text buffers](https://docs.gtk.org/gtk4/class.TextBuffer.html) document
  native editing and grouped user actions. No newer GTK requirement is assumed.
- [W3C text quote selectors](https://www.w3.org/TR/annotation-model/#text-quote-selector)
  distinguish captured quotation and surrounding context from positional hints.
