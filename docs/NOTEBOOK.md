# Document notebook · F01–F02

Selected 18 September 2026 from the first undeveloped roadmap item.

This document records the original F01–F02 location-only notebook design.
Quotation capture and connections to notebook sections are now delivered as
F02a, specified in [Selected passages and notebook connections](PASSAGE_CONNECTIONS.md).
The original decisions below are historical where that extension adds or replaces
behavior: quotations, connected ranges, shared undo, and schema v3.

## Reading and writing

Each document has one continuous plain-text notebook. GTK TextView supplies
selection, clipboard, input methods, accessible text, and bounded undo history.
Markdown punctuation may be typed but is not rendered. No rich-text engine,
workspace, note-card model, or source-file editing is introduced.

The Notebook toolbar button and Ctrl+Shift+N show/hide it. Wide windows place a
resizable notebook on the right; narrow windows stack it below the document.
While stacked and visible, the window has a 650-pixel minimum height so both
reading and writing remain usable; hiding the notebook restores the usual minimum.
The editor is independently scrollable. Turning pages, searching, changing
appearance, or reloading the source never replaces its buffer. Hide/show keeps
the live buffer and undo history. Focus hides the notebook temporarily.

A current-location indicator follows the reader. “Attach passage” snapshots the
current settled locator and adds a named reference to the notebook's reference
list. The editable name can identify an example in the text. Multiple references
are allowed; they belong to the whole notebook, without fragile inline offsets.
Following one uses existing navigation/history, so Back returns to the previous
passage. Rename and removal are explicit. Moving the reader never changes a saved
reference. Selected quotation capture, highlights, and inline reference editing
remain separate features.

## Durable work

SQLite schema v2 adds one notebook row per document UUID, with a versioned JSON
payload containing exact UTF-8 text, named references and locators, and editing
context (Unicode character cursor/selection and first-visible-character offsets, panel
visibility and dimensions). The additive migration runs in one transaction;
failure rolls back and leaves existing documents/bookmarks intact. Newer schemas
and invalid/unknown notebook payloads are rejected without replacement.

The existing single state worker owns database access and retained notebook
snapshots. Notebook loading checks retained drafts before disk. Saves serialize
with document identity reconciliation and Locate. Removing a recent document
preserves its notebook, including retained drafts. Reading-position updates do
not carry or rewrite notebook text.

Only the active document’s notebook is loaded. A failed load shows Retry and disables editing until the
existing notebook can be read. Edits debounce at 750 ms with a two-second maximum,
and flush when hiding, leaving the document, losing window activation, or closing.
Failed writes retain the latest snapshot in memory, expose a persistent error,
and retry through the existing storage flow. Close waits for queued work and
offers Keep open or explicit Close anyway if saving fails; the latter can lose
unsaved drafts. A process kill may lose edits since the last completed autosave.

Save status belongs to the state worker; late replies never mark a newer draft
saved or replace live editor text. Load completions are checked against the active
notebook so switching documents cannot put another document's text in the editor.
Showing or hiding a notebook while it loads is explicit user intent: it overrides
the saved visibility when loading finishes and is then persisted.

## Validation

Core tests cover v1 migration and rollback, text/context/reference round trips,
document isolation, Locate and recents removal, invalid/newer payload protection,
failed saves and latest-draft retry, and delayed reply ordering. Native checks
exercise typing and selection, notebook focus during page turns, search and
mode changes, references and Back, hide/show, narrow layout, source reload,
document switching, and persistence through a fresh notebook load. Use all three
document formats and a long notebook; record measured checks and limits in the
implementation record. No new performance percentile or Orca usability claim is
implied by these checks.

## Implementation decisions confirmed by native checks

The initial native panel established that TextView can preserve focus while all
three readers turn pages. Native wide/narrow screenshots exposed an allocation
problem; an Adwaita breakpoint now owns the orientation without polling frames
while idle. A second-process restart test exposed GTK collapsing selection during
unmap; the notebook captures context before hiding and restores it on map.

The breakpoint temporarily unmaps the reader and restores its focus. PDF passage
preservation captures the locator synchronously in the breakpoint notification,
before Adwaita's deferred focus traversal, and uses the existing restore path.
Hiding a focused sidebar or notebook returns focus to its toolbar toggle; entering
Focus mode explicitly focuses the current reading surface. Native checks keep a
non-first PDF page visible through these transitions and later resizing.

The active notebook is loaded in full. The validation corpus includes a 124 KB
native editor draft (12,000 lines) and a 1.06 MB storage round trip. These are
functional volume checks, not a claim of bounded memory for arbitrary notebooks.
Undo history is bounded to 100 user actions and stays within the live document
session; it is not persisted across document switches or restarts. Text,
references and editing context are persisted.

GTK text marks defer scroll restoration until the saved display line is laid out.
The saved offset accounts for soft wrapping within a single long paragraph. This
keeps the editing passage stable across panel widths and avoids clamping a saved
pixel offset against a long buffer’s temporarily incomplete scroll bounds.
At fractional display scales, GTK's subsequent layout updates can stop the scroll
animation before it reaches the mark. A single idle callback completes the restore
at the laid-out mark without animation. Hiding the pane cancels that callback;
new text or cursor input cancels restoration so saved context cannot override it.

Scroll-position lookup uses GTK hit-testing, with a binary search within the
current paragraph when the hit falls on blank space. It handles right-to-left
text without walking every preceding wrapped line. Native regressions cover
ordinary, indented, and right-to-left paragraphs, including short final wraps.

The pane stops scroll animations before resetting or hiding the editor. This
prevents GTK 4.22 from finalizing the same animation twice while unmapping a view
whose buffer has just been cleared. Both a minimal reproducer and the real
notebook reset flow were verified with native criticals treated as fatal.

Editing context is captured separately from text. Hiding the pane does not copy
its text, and cursor/scroll-only saves reuse the current text snapshot. Document
matching and pane-size constraints have shared definitions. The performance probe
covers multiline notebooks through 1.08 MB and single paragraphs through 278 KB.
Large individual paragraphs still carry GTK shaping and layout costs; these
observations are not an arbitrary-size latency guarantee.
