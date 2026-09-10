# Readero MVP product requirements

Version 1.0 · 4 September 2026 · Personal Ubuntu reader

Status: finalized implementation baseline from the discussion and stack research. This is a product/build specification, not a claim that the application or its release checks are complete. Implementation must satisfy the gates below before the MVP is called done.

## 1. Purpose

Make it easy to start reading, stay comfortable, and resume without losing the passage or train of thought.

The primary user reads technical books, PDFs, and Markdown on their own Ubuntu laptop. They switch between scrolling and page-by-page reading depending on the document and their mood. They value responsiveness, quiet presentation, reliable saved position, and a future place to collect page-linked notes and worked examples.

The product succeeds when the owner voluntarily returns to it for everyday reading and trusts it to preserve their place. This is a personal product; growth, monetization, social features, and engagement metrics are not goals.

## 2. Target and scope

Initial supported platform: Ubuntu 26.04 amd64, primarily Wayland, on the owner's current Core Ultra 7 255U laptop with approximately 32 GB installed RAM. X11 is a fallback compatibility check, not a reason to compromise Wayland behavior. First distribution artifact: an installable `.deb` plus reproducible source/build instructions.

MVP formats:

- PDF: born-digital and scanned documents, including password-protected files when the user supplies the password. Preserve page layout. Scans display normally; OCR is outside this release.
- EPUB: DRM-free, reflowable EPUB 2 and 3. Preserve ordinary publisher CSS, local images, fonts, SVG, MathML, footnotes, and navigation. Fixed-layout EPUB, scripted interactivity, media overlays, and DRM systems are outside the release's guaranteed support.
- Markdown: UTF-8 `.md`/`.markdown`, CommonMark plus tables, task lists, strikethrough, and footnotes. Preserve code blocks and resolve local relative resources. Raw HTML is escaped. This is a defined dialect, not complete GitHub or Obsidian compatibility.

Both Scroll and Pages are required in all three supported reading paths. They are independent of focus mode and fullscreen.

## 3. Product principles

1. Preserve the passage through navigation, layout changes, document switches, and reopening.
2. Make normal reading calm and focus mode explicit and reversible.
3. Show useful content quickly and keep work bounded as the document grows.
4. Respect document structure: PDF pages remain fixed; EPUB/Markdown text reflows.
5. Open files where they already live. Reading does not require importing a library.
6. Preserve originals. Progress and bookmarks belong to application data.
7. Make degraded behavior clear: unavailable text, unsupported content, missing files, and approximate restoration should be understandable.

## 4. Main flows

### Continue reading

Launch without a file → a compact start view presents the most recently read document prominently → Continue opens the saved passage with its reading settings. A short recent-documents list provides other choices. With no history, the start view presents Open and a drag target. Avoid preloading document renderers or extracting covers simply to show this screen.

### Open directly

Open With, drag-and-drop, or Open → validate the file → show readable content → resume if this document has a saved location. Opening a different document saves the current state before replacing the active surface. The MVP has one active document in its primary window and no tab/session workspace.

### Change how it reads

Use a visible Scroll/Pages control → retain the current passage → save the choice for this document. Initial defaults are Scroll for PDF/Markdown and Pages for EPUB. Changes to type, width, zoom, or window geometry follow the same preserve-and-restore behavior.

### Follow a reference

Activate an internal link, contents entry, bookmark, or search result → navigate → Back returns to the prior passage and viewing state. Continuous scrolling does not fill navigation history with every movement.

### Focus

Activate Focus from a labeled control or shortcut → header/sidebar/progress chrome withdraws while the passage remains visible → Escape or the same shortcut restores the previous layout. Fullscreen can be enabled independently. Opening a menu temporarily must not unexpectedly exit focus or move the document.

### Resume after interruption

Position saves quietly during use. A clean close flushes the latest stable location. Relaunch → Continue → return to that location. After an abrupt process exit, restore the most recent committed position within the specified save interval. After an external content change, preserve location where it can be resolved and identify an approximate fallback.

## 5. Functional requirements

### FR01 — File opening and recovery

- Support the file picker, drag-and-drop, desktop Open With, and a command-line file argument; handle spaces, non-ASCII names, and case-insensitive extensions.
- Show a responsive opening state and allow opening another file/canceling slow work. An older load completion cannot replace the newer document.
- For a protected PDF, request its password, distinguish an incorrect password from corruption, and allow cancellation. Keep passwords in memory for the session; do not save them in progress data or logs.
- If the source is missing, keep its history/bookmarks and provide Locate and Remove from recents. Removing from recents does not delete the source or its durable bookmark data.
- Report unsupported/corrupt/empty files without crashing or overwriting previous state.

Acceptance: open one sample per supported path; cancel a protected PDF; rapidly open A then B; relink a moved document; verify A's delayed completion cannot replace B and original bytes remain unchanged.

### FR02 — Recent documents

- Show title or filename, format, and saved position when available; keep the recent list compact, initially capped at 30 entries.
- Display only documents explicitly opened in the app. Do not scan Downloads or build a library in the shipped product.
- Keep a single most recent document with a prominent Continue action. Recency changes on successful reading, not a failed open attempt.

Acceptance: restart with several prior documents and return to the intended one without browsing folders.

### FR03 — PDF reading

- Scroll uses continuous vertical page layout. Pages shows one original page at a time.
- Provide fit width, fit page, numeric/free zoom, rotation, next/previous page, and page entry supporting available page labels.
- A zoomed page may be panned; page mode must not make offscreen content unreachable.
- Render to an appropriate scale for the display. Zoom must settle to sharp content rather than remain a stretched low-resolution preview.
- Preserve PDF colors by default. A dark application theme may surround a light document page; color inversion is not required for MVP.
- Respect scanned-image limits: display the page and clearly indicate unavailable searchable/selectable text when relevant.

Acceptance: use text, image-only, mixed-size, rotated, labeled-page, and protected fixtures; verify navigation and content reachability in both modes.

### FR04 — EPUB reading

- Pages reflows content into a single reading column with keyboard/click navigation. A two-page spread is a future option.
- Scroll presents adjacent spine sections in a continuous vertical flow. Reaching a chapter boundary must not discard unread trailing lines or require a separate chapter-switch action.
- Keep heading structure, illustrations, code, tables, and native MathML readable. Publisher style and a comfortable application reading style should be available through a restrained typography policy.
- Offer font family, font size, line spacing, reading width, and margins. Use useful bounds/defaults instead of exposing all CSS values.
- Support ordinary footnote/endnote links with return navigation; a dedicated footnote popup is optional polish after the core path works.
- Oversized blocks remain accessible; an image can be inspected at a larger size and closed without changing reading position.

Acceptance: test the three chosen technical EPUBs in both modes, including a visible chapter boundary, delayed image loading, code/table-heavy passages, changes in font/window width, and backward movement across boundaries. Add an EPUB 2 fixture because the personal sample contains only EPUB 3.

### FR05 — Markdown reading

- Render the defined dialect with semantic headings, lists, links, tables, footnotes, and fenced code.
- Offer the same typography and reading-mode controls as reflowable EPUB.
- Preserve code whitespace and make wide blocks/tables horizontally accessible. Syntax coloring is outside the initial acceptance criteria.
- Resolve links and assets relative to the source file, subject to the resource-access boundary. Opening a related Markdown document participates in Back navigation.
- Watch the active source for completed external edits using a debounce. Reload without moving away from an identifiable passage. On an ambiguous edit, restore a nearby section and indicate the approximation.

Acceptance: use a synthetic document with long code, a wide table, duplicate headings, footnotes, relative images/links, Unicode, and edits before/inside the active passage. Check every part remains reachable in Pages and Scroll.

### FR06 — Locations and navigation history

- Store a format-aware, versioned location independent of its presentation mode.
- Preserve it through Scroll/Pages switches, typography, rotation/zoom where applicable, focus, resizing, and reopening.
- PDF references use original document coordinates and physical page identity. EPUB references use CFI/resource identity. Markdown uses block/text/source-revision information.
- Persist document-specific mode and reading settings. Maintain a Back/Forward history of intentional jumps within the session.
- Do not persist temporary top-of-document locations emitted during opening or reflow over a good saved position.

Acceptance: for unchanged content, the referenced passage remains in the viewport after every transformation. In the same window geometry/mode, restore a PDF point within 8 logical pixels or a reflowed passage within one text line, excluding unavoidable document-edge clamping. With different geometry, require the same passage visible rather than the same screen-page number. Use at least 20 locations per format in the release tests.

### FR07 — Search, contents, and selection

- Provide document-wide text search, progressive results, next/previous result, cancellation, and a clear result count/completion state. Scan whole EPUB/Markdown content, not only mounted sections.
- Work incrementally on large documents and allow reading during search. Do not start library-wide indexing in the background.
- Provide the document's table of contents where present and a generated heading outline for Markdown.
- Support text selection and copying where the source has text. Preserve ordinary code text on copy.
- Back returns from a search/contents jump to the pre-jump passage. Application shortcuts must not consume text-entry keys while focus is in a search field or other input.

Acceptance: find terms outside the current chapter/page, cancel during a scan, switch documents during a scan, navigate back, and copy a technical code passage. Clearly handle an image-only PDF.

### FR08 — Bookmarks and saving

- Add/remove a bookmark at the current location. List and revisit bookmarks for the document.
- Save reading location after approximately 750 ms of inactivity, with a maximum two-second unsaved interval during continuous movement. Coalesce redundant events.
- Flush stable state on document switch, focus loss, meaningful discrete navigation, and clean shutdown. Writes must not block the rendering/UI thread.
- If storage fails, keep the current session usable and show a clear persistent indication that progress is not being saved. Do not silently recreate an empty database over a damaged one.

Acceptance: bookmark a passage, change layout, restart, and return to it. Terminate during an uncommitted write and recover the preceding committed state. Simulate an unwritable/full state directory and verify the user is informed. Power-loss behavior needs the filesystem/runtime qualification described in the validation plan.

### FR09 — Focus, theme, and accessibility

- Keep a restrained toolbar and optional navigation sidebar. Use labeled/tooltip-equipped controls and a shortcut reference.
- Focus hides chrome and restores the previous layout. Fullscreen is separate. Default to an ordinary visible-control window after a fresh launch.
- Provide light, warm, and dark reading palettes for reflowable content; follow the system application theme by default. Preserve figures/code contrast and avoid applying a universal image inversion.
- Make all core actions keyboard reachable with visible focus. Basic text reading/navigation and controls must be checked with Orca; scanned PDFs remain subject to their lack of text.
- Respect system text scaling/reduced-motion behavior where applicable. Do not inhibit system suspend merely because a document is open.

Acceptance: complete open → search → bookmark → mode switch → focus → close using a keyboard. Exit focus using Escape. Verify the same passage remains visible and controls return in the expected order. Test representative semantic text with Orca and check 100%, 125%, 150%, and 200% display scaling.

## 6. Performance budgets and measurement

These are initial release targets, not results measured during research. Use a release build on the target laptop, native Wayland, a fixed 1050 × 800 logical-pixel window and 60 Hz display setting where available. Record actual display scale, power mode, filesystem cache condition, engine versions, and all child processes. Do not alter the user's system configuration automatically to run a benchmark.

- **First readable content:** p95 ≤ 2 seconds for process-cold launches with warm OS file cache on the agreed ordinary corpus; p95 ≤ 1 second for reopening in an already running app. Measure from the open request to readable content with working input at the intended location. Measure cold OS-cache startup separately; no cold-cache guarantee is yet established.
- **Simple actions:** visible feedback within 100 ms for ordinary controls and cached navigation. Uncached work remains cancellable and keeps the UI responsive.
- **Scrolling/page transitions:** on the measured 60 Hz path, at least 95% of active frames within 16.7 ms and 99% within 33.3 ms, excluding explicitly identified initial document loading. No recurring visible stalls at EPUB chapter boundaries.
- **Idle:** after loading/search settles, total app-process-tree CPU averages below 0.5% of one logical CPU over 60 seconds, with no recurring indexing/animation activity.
- **Memory:** initial ordinary-document budget of ≤ 350 MiB process-tree PSS for a PDF and ≤ 500 MiB for EPUB/Markdown, measured after steady reading. Record GPU memory separately where observable. These are investigation thresholds, not reasons to corrupt or truncate a legitimate document.
- **Bounded growth:** after 20 forward/backward traversals and 20 mode switches, memory returns near the settled baseline: within the greater of 15% or 50 MiB after cache settling. Count mounted chapter frames and cache entries to explain growth.

Run at least 10 independent process launches and 30 warm-open/action observations per representative file for the initial report; record distributions and individual outliers. Long-document stress fixtures test responsiveness, cancellation, and bounded memory separately from the ordinary-corpus timing targets. Compare against Papers/Foliate under the same conditions for context, without assuming Readero must win every metric.

If a budget proves inappropriate, change it explicitly with measured evidence and its reading-experience impact. A smoke-harness timing cannot be used as an application result.

## 7. Data and document boundaries

Keep originals unchanged. Store durable state/bookmarks under the app's XDG data directory with schema migrations and recoverable backups. Keep rebuildable caches separate. Avoid logging passages, passwords, or full document contents.

Serve opened publication assets through an allowlisted local resource service. Disable publication-authored scripts and automatic remote resource loads. External browsing requires an intentional link activation through desktop APIs. Constrain Markdown relative-resource access to the granted directory tree, resolving traversal/symlinks before reading. Verify malformed archives and decompression limits without hanging the UI.

Application scripts and document content need isolated contexts and a narrow message bridge. The research harness is not the production security implementation. Include explicit malicious-content fixtures in the renderer milestone.

## 8. Future notebook and annotations

The [future-feature roadmap](ROADMAP.md) consolidates these ideas, the smaller optional reading improvements, open decisions, and a proposed delivery order. This PRD remains the MVP acceptance baseline.

Preserve the architectural ability to add these after MVP:

- A continuous notebook belonging to each document, suitable for examples, questions, and longer thoughts.
- An unobtrusive current-page/passage indicator and an action to attach that location to a note.
- Clickable location references that restore the relevant passage across reading modes.
- A stable notebook view while pages change; optional filtering to notes near the current passage later.
- Passage highlights, comments, and eventual export.

MVP needs versioned locators, durable document identity, navigation/selection boundaries, and a layout that can accommodate a future right sidebar. It does not need an empty notebook panel, a note editor, annotation UI, or speculative annotation tables. PDF annotation export and EPUB/Markdown anchor repair require their own later acceptance criteria.

## 9. Explicitly outside MVP

Cloud accounts/sync; managed libraries and catalog import; OCR; editing source documents; PDF forms/signing/redaction; DRM integration; fixed-layout EPUB guarantees; TTS/media overlays; AI summaries/chat; reading streaks; multi-document tab workspaces; PDF reflow; syntax-coloring engines; Mermaid/LaTeX Markdown extensions; mobile/Windows/macOS distribution.

These exclusions protect the core reading loop. Existing annotations in a PDF should display as supported by its renderer, while creation/modification is deferred.

## 10. Delivery sequence and release gates

### M0 — Compile and qualify the adapters

Compile/link the pinned Rust/native graph. Wire Papers contexts correctly. Create the production resource boundary and typed messages. Qualify the continuous EPUB controller against delayed images, font changes, width changes, reverse traversal, unloading, and CFI restoration. Establish Markdown page/scroll behavior with the defined dialect. Check that document scripts cannot invoke native actions.

Exit: all three formats can be rendered/navigated in both modes on the target machine, and no unresolved architectural blocker remains. If continuous EPUB flow fails, revisit the stack decision before expanding the application; do not downgrade the behavior silently.

### M1 — Trustworthy reading and resume

Implement file opening, password/error flows, locators, transactional saving, per-document preferences, navigation history, and resize/mode restoration. Complete the killed-writer and changed/moved-file cases.

Exit: the owner can repeatedly open/read/close/resume without losing the intended passage on unchanged documents.

### M2 — Comfortable everyday use

Implement recents/Continue, contents/search, bookmarks, typography/themes, focus/fullscreen, image inspection, and keyboard/accessibility behavior. Check technical figures, tables, and code at different widths/scales.

Exit: all functional acceptance criteria pass on the ordinary corpus and supplemental fixtures.

### M3 — Measure, package, and use daily

Profile a release build on native Wayland; meet or explicitly revise the performance budgets. Verify `.deb` installation, launcher/Open With behavior, dependency availability, and data preservation across an application upgrade. Document known format limitations.

The owner completes at least five reading sessions across the formats they use, including both modes and a restart. Collect qualitative friction in ordinary notes, without adding telemetry or reading-pressure features to the application.

Exit: no unresolved loss of saved location/bookmarks, unreadable/clipped supported content, uncontrolled resource growth, or core keyboard failure. Ship only after the package and acceptance results are recorded.

## 11. Definition of done

Readero opens the supported document types directly; reads comfortably in Scroll or Pages; searches/navigates/copies where text exists; preserves position and document preferences; resumes reliably; offers bookmarks and reversible focus mode; protects source files; and has an installable package plus reproducible build/test instructions for this Ubuntu machine.

The [stack decisions](STACK_DECISIONS.md) explain the implementation choices. The [validation record](research/VALIDATION.md) states what has already been demonstrated and what still requires the application.
