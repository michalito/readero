# Implementation and quality record

Updated 13 October 2026 · Readero 0.1.1 personal preview

## Review follow-up · 13 October 2026

Saved passages now display the source location separately from their editable
name, including an explicit no-quotation indication for location-only references.
Single-source gutter controls expose the passage name and location in their
tooltip and accessible name. Capturing the same source range again selects and
reveals the existing saved passage without changing the notebook editing state
at capture time. Reflow whitespace-only selections keep Save disabled without
materializing the full quotation on each selection event.

The independent review also found that native PDF keyboard scrolling retained
the previous selection and quotation indication. Papers scroll actions now
invalidate capture and pending navigation before their normal handler runs.
Eight focused native checks cover keyboard movement, stale capture rejection,
highlight clearing, and preserved programmatic passage restoration.

Hosted review follow-up clears stale capture/highlight state when the user
deselects or selects whitespace in another reflow chapter, while preserving a
newer selection when a background chapter reports delayed deselection. Saved PDF
quotations are validated against their ordered page fragments, permitting only
whitespace between them. The original notebook design now labels its location-only
description as historical and identifies F02a as delivered. Recovery scans now abort
when navigation or a newer request supersedes them, including during document
loading. Paged PDF navigation preserves the announced page immediately for saves,
bookmarks and appearance changes while deferring coordinate sampling.

The [fresh review evidence](qa/review-validation.json) records 53 core and 53
desktop Rust tests, 10 installer tests, strict Clippy, 358 native assertions across
Markdown/EPUB/PDF passage flows, the notebook flow and mixed-size PDF navigation,
and 25 AT-SPI assertions.
All pass in an isolated Ubuntu 26.04 container with synthetic data, private Xvfb,
and private D-Bus sessions. The host disallows WebKit's user-namespace sandbox,
so it was disabled only for these container runs; this is not sandbox
qualification. No installation or user desktop/database was touched. Historical
September measurements below remain historical. The human, ordinary-reading,
and complete performance qualification gates remain open.

## Responsiveness and opening recovery · 21 September 2026

Follow-up diagnosis reproduced the large connected-notebook delay and isolated
it to GLib scheduling. GTK's incremental offscreen text validation runs before
default-idle callbacks. A debugger caught the main thread in Pango/HarfBuzz
layout while Readero's already queued connection update waited behind it.
Connection updates now run after visible layout/paint and before background text
validation, using GTK's published priority constant. The existing coalesced,
one-shot update remains; there is no frame poll or new rendering abstraction.
A native regression fails with the old priority and passes with the new one.
The final optimized X11/Cairo run records an 8.01 ms maximum for insertion plus
connection updates in a 1.24 MB notebook with 1,000 connections (20 samples,
including the first update). Cursor relocation is measured separately and stays
within 1.12 ms in that case. Disconnect/undo plus decoration peaks at 22.73 ms.
After 40 reading turns, 20 mode changes and notebook edits, the eight-process app
tree averaged 0.49% of one CPU core over 119.8 idle seconds. Proportional memory
fell from 184.6 to 183.7 MiB, within the existing growth budget. These measurements
ran without another Readero test or build workload.

The benchmark now separates cursor relocation from insertion, and records the
first sample as well as steady-state work. A minimal GTK TextView independently
reproduces expensive layout for enormous unbroken paragraphs. That native limit
remains; operation measurements do not establish physical input/frame latency.

The two previously recorded isolated startup stalls did not recur in 100 trials:
30 X11 launches, 40 ordinary file-open launches inspected through AT-SPI, and
30 Wayland 200% Home/open/snapshot/close sequences. Their original cause remains
unproven. Inspection found an independent failure mode: a silent WebKit renderer
could leave an opening candidate pending forever. A 30-second renderer deadline
now uses the existing recovery path to preserve the previous reader and notes,
restore controls, and explain how to retry. Parsing and PDF password entry are
outside that deadline. Successful, cancelled, superseded, and closing candidates
dispose their timers. This does not claim to recover a blocked native main thread
or a graphics-driver hang.

The native startup regression deliberately delays WebKit beyond the deadline,
both from Home and during replacement. The old build fails eight assertions;
the fixed build passes all 20 checks, including preserved notes, successful retry,
and cancelled deadlines remaining harmless after their expiry time. Repeatable
`--startup-only` and `--startup-recovery` modes keep the fault injection and
synthetic data out of the ordinary application.

The [follow-up evidence](qa/responsiveness-validation.json) records the red/green
checks, final cross-format runs, optimized measurements, and installed build.
All 404 native assertions pass, along with 52 Rust tests both with and without
desktop features, 10 installer tests, strict Clippy, and package formatting.
The ordinary release was rebuilt and installed under `~/.local`; its checksum
matches the build and it excludes the smoke driver. All 39 keyboard/AT-SPI/Orca
checks also pass against that installed executable, using private test data and
silent synthesis. No user reading database was opened by validation.

## Selected quotations and notebook connections · 20 September 2026

F02a implements the [approved design](PASSAGE_CONNECTIONS.md). PDF, EPUB and
Markdown selections become immutable saved quotations with precise range targets.
Save current location remains an explicit separate action. Selected notebook text
can connect to one or several passages; passages can support several sections.
The adjacent arrow opens a source (or a chooser), while ordinary clicks edit and
Ctrl+click follows. Sections sharing a displayed line share one chooser button.
Missing or ambiguous text offers explicit relinking and a preview before replacing
a quotation. Deleting a connected passage asks before disconnecting it.

The pure passage and notebook modules own validation, Unicode offset rules,
range transforms, and a single bounded undo history for text and connections.
GTK user actions group paste, replacement, and composition; ordinary typing uses
GTK's action boundaries, without a speculative word-coalescing timer. Text history
stores splices and affected sections; semantic history retains changed records.
The existing state worker saves the complete notebook atomically. Database schema
3 fences older executables; legacy payloads upgrade losslessly on load without
inventing quotations or connections or rewriting every notebook at startup.

Renderer adapters capture and verify source ranges. Reflow uses DOM UTF-16 ranges,
CFIs, exact quotations and bounded context. PDF uses native Papers selections and
selection regions in unrotated coordinates. Recovery searches only the original
resource/pages and rejects ambiguity. New selections in rotated PDFs are disabled
by Papers 50.2; the UI offers Rotate upright to select. Saved passages still open
and highlight correctly at 0°, 90°, 180°, and 270°. This adds temporary navigation
indicators, not persistent PDF annotations or a parallel rendering engine.

Only visible notebook underlines and source controls are materialized. A queued
idle coalesces updates; controls are reused, quotations open lazily, and no frame
poll runs while idle. This removes the reproduced multi-second decoration work on
every edit with 1,000 connections. GTK still has expensive initial layout and
pathological single-paragraph cases; measurements retain those outliers rather
than presenting operation timings as physical input latency.

The [safe evidence record](qa/passage-validation.json) contains fresh synthetic
native checks, core and desktop tests, keyboard/AT-SPI/Orca checks, 125% and 200%
Wayland coverage, and optimized operation/idle measurements. Functional checks
cover exact capture, deduplication, note edits and undo, shared connections,
missing-text recovery, confirmed relinking/deletion, Back, stale completions,
small windows and real process restart. Accessibility testing also fixed source
menus whose announced names changed to their popover contents; names are now
explicit and stable. Automated Orca announcements use silent synthesis and do
not replace human listening or reading-usability assessment.

The completed runs record 629 native assertions, 308 assertions across six
format/scale cases, 39 accessibility/keyboard checks, 52 Rust tests both with
and without desktop features, and 10 installer tests. All pass, with clean
formatting and strict Clippy. One private Wayland startup/restart stalled before
opening the source; a fresh compositor completed the affected cases. An initial
optimized accessibility launch also stalled; its completed rerun passed. These
incomplete attempts remain recorded, without claiming an established root cause.

In the final optimized X11/Cairo probe, a 1.24 MB notebook with 1,000 connections
had an 8.68 ms edit-and-decoration p95 and a 15.72 ms disconnect/undo p95 (20 samples
each). The first edit after wholesale replacement and cursor relocation cost
879.57 ms; that maximum is retained. A single 278 KB paragraph had a 149.64 ms
native-edit p95. After 40 turns, 20 mode changes and editing with 1,000 connections,
the eight-process app tree used 0.192% of one CPU core over 119.7 idle seconds.
PSS fell from 211.8 to 207.6 MiB against a 199.2 MiB settled baseline, within the
existing growth budget. No concurrent Readero test/build workload ran during
these measurements; GPU memory and physical frame/input latency are unmeasured.

The ordinary optimized desktop executable was installed under `~/.local`, with
its checksum verified against the build and smoke-driver exclusion checked.
The installed executable also passes all 39 accessibility/keyboard checks using
private test data. A verified backup of the user's schema-2 database was created
before installation; validation never opened it for writing. The normal first
launch performs the tested transactional schema-3 upgrade.

## Document notebook and passage references · 18 September 2026

F01–F02 now provide one plain-text notebook per document with native undo/redo,
selection and clipboard support, named passage references, autosave, and restored
editing context. The [selected design](NOTEBOOK.md) records the editing format,
reference structure, persistence rules, layout decisions, and deliberate limits.
A resizable panel sits beside the reader or below it at narrow widths; the stacked
layout enforces a usable minimum window height. Focus hides and restores it.

The live TextView survives page turns, search, appearance changes and Markdown
reloads. GTK can collapse selection when unmapping and validates long text lazily;
the notebook captures context before hiding, restores selection on map, and uses
a text mark for deferred scrolling to the saved character. Its native behavior
was checked visually at wide, narrow and short window sizes.

Schema v2 adds a versioned notebook payload keyed by document UUID. Migration is
transactional and preserves existing documents and bookmarks. The existing state
worker retains failed drafts, reconciles provisional identities, and isolates
Locate from later reuse of an old path. Notebook saves do not overwrite reading
progress or resurrect removed recents. Failed loads remain read-only; failed
closes explain draft loss and offer Keep open. No new production dependencies or
rendering engine were needed.

The [validation record](qa/notebook-validation.json) records 40 passing Rust tests
with and without desktop features, 643 native assertions across the three formats
including encrypted PDF and real process restarts, 11 AT-SPI checks, 10 installer tests, and clean formatting
and strict Clippy. The ordinary release executable was rebuilt without smoke hooks.
Tests use synthetic documents and isolated databases;
manual Orca usability and the broader performance/release gates remain open.

A follow-up review reproduced and fixed two missed notebook cases: restoring a
passage within a soft-wrapped paragraph and hiding the panel before its asynchronous
load completes. Debug-build validation also exposed the existing Papers signal
binding rejecting a null passage origin; the callback now receives the notification
without decoding the unused arguments. Focused native checks and process restarts
pass across all three formats (125 assertions), including these regressions.
Strict Clippy and formatting pass, and the ordinary release was rebuilt.

The intermittent GTK signal-disconnect/frame-clock critical was subsequently
reproduced in a minimal reset sequence. Clearing text during an animated scroll
could make GTK finalize the animation twice during unmap. The pane now stops
scrolling before hiding and resetting; 100 minimal repetitions and the native
application regression verify the fix. Full native runs treat GTK criticals as fatal.

Performance review also removed a quadratic wrapped-line scan: saving a position
deep in a 278 KB paragraph previously spent 6–8 seconds finding its visible line.
The pane now uses native hit-testing and a bounded search over blank space, with
right-to-left and short-wrap regressions. Context capture no longer reads/copies
the complete text, and cursor-only changes reuse the text snapshot. Document
matching and notebook dimensions have one definition each. The repeatable
`notebook` and `notebook-idle` probes record large-note operation costs and idle
resources without introducing production instrumentation or new dependencies.
On the first optimized X11 run, the 278 KB paragraph's visible-line lookup had
a 6.2 ms median (7.2 ms maximum, ten samples). A 1.08 MB multiline notebook had
a 0.46 ms median native edit and 14.0 ms median save completion. A single paragraph
of 278 KB still costs GTK roughly 100 ms to edit/layout; the benchmark records that
limit explicitly. These are direct native operation measurements, not physical
typing or frame-time percentiles.
After 40 page turns, 20 mode changes, and notebook edits, the eight-process app
tree averaged 0.44% of one CPU core over 119.6 idle seconds. Proportional memory
fell from 207.9 to 206.9 MiB, within the existing growth budget. This observation
used X11/Cairo; GPU memory and Wayland behavior were not measured in this run.

Additional qualification uses a private GNOME Wayland compositor with the Vulkan
renderer and actual 100%, 125%, 150%, and 200% surface scales. It covers Markdown,
EPUB 2/NCX, and a 1,200-page PDF, including notebook layouts, selection, references,
save failures, and process restart. An 80-page, 21 MB image-only PDF also exercises
the full native regression flow. The [follow-up evidence](qa/notebook-qualification.json)
records the final binary, checks, and measurements without personal document data.

This caught a fractional-scale restoration defect: GTK could stop its scroll
animation partway while validating text. A single post-layout idle now completes
restoration at the saved text mark without animation. Hiding the notebook or
editing before the callback cancels it; no timer or frame polling remains active
while idle. The regression verifies saved selection and new input taking precedence.
The headless test supplies virtual keyboard input because Wayland requires valid
input serials for primary-selection ownership. It never changes the host display.

Visual review additionally found that a PDF search jump could leave the notebook's
current-passage label describing the preceding page. Papers emits its page-change
signal before the view's coordinates are updated; an ordinary idle can also run
before that allocation. The PDF adapter now refreshes position after a completed
layout frame. Native assertions compare saved references and current position
with the actual visible PDF page, rather than only comparing saved locators.
A separate scanned-PDF regression exposed focus traversal into Papers when a
focused sidebar closed. That traversal could show page 1 while the model retained
another page. Closing the focused sidebar or notebook now returns focus to its
toolbar button; entering Focus mode explicitly focuses the current reading page.
Adwaita's breakpoint transition also temporarily unmaps the reading area and
restores focus. The breakpoint notification captures the locator synchronously, before that
deferred focus restoration, and reuses the existing PDF restore path. Search release itself is unchanged. Regression checks start away
from page 1 and compare the actual visible page through closing, focus and resize.

The ordinary release also passes real Orca announcement generation and keyboard
typing, undo/redo, notebook toggling, and reference rename/follow/removal. The
isolated Speech Dispatcher uses a silent output module. This verifies integration;
it does not substitute for human listening or a reading-usability assessment.

The completed follow-up records 617 full-flow native assertions, 628 assertions
across 12 format/scale cases, and 26 Orca/keyboard checks. All pass, alongside
40 Rust tests with and without desktop features, strict Clippy, formatting and
syntax checks. The record distinguishes the shared-focus test artifact from the
final PDF-only synchronization fix, which has its own full PDF flows and scale
matrix. The ordinary release is rebuilt without the smoke feature.

The follow-up readiness measurements use an observed 1050 × 800 logical window,
125% Wayland scale, and Vulkan. Ten launches and 30 reopens per format gave
launch/reopen p95s of 2.028/0.649 seconds for Markdown, 1.576/0.455 for EPUB 2,
and 1.166/0.070 for PDF. The Markdown launch outlier exceeds the two-second
target and remains in the record. Launches include private D-Bus setup with the
OS cache retained; PDF readiness is a native-ready notification plus 32 ms,
not verified pixel delivery. These observations do not establish input or frame
latency. A repeat X11 notebook probe measured a 0.79 ms median edit for 1.08 MB
of multiline text, but roughly 300 ms for a single 278 KB paragraph. The latter
varied from roughly 100–300 ms across runs and remains a GTK layout limitation;
visible-line lookup is bounded and no longer takes seconds.

With the final optimized build and no concurrent test/build workloads, the
1050 × 800, 125% Wayland EPUB/notebook run averaged 0.134% of one CPU core over
119.6 idle seconds after 40 turns, 20 mode changes and editing. The eight-process
tree's PSS fell from 228.8 to 225.2 MiB, against a 220.5 MiB settled baseline.
CPU, EPUB memory and post-activity growth meet the stated investigation budgets
for this observation. GPU memory and physical frame delivery remain unmeasured.

## Document opening and reload ownership · 10 September 2026

Document opening now owns complete requests, candidate readers and password dialogs, readiness, cancellation, history commitment, directory monitoring, and deferred source changes in `desktop/shell/opening.rs`. Markdown reload retains the working reader until its replacement finishes native layout; startup failure restores that reader and reports the failure immediately. A newer edit during preparation is retained, and reopening the same document captures the latest position and appearance. Home and Close reject late completions; a failed close resumes deferred source changes only after Keep open is chosen.

The [validation record](qa/document-opening-validation.json) records a native red run reproducing loss of the working reader and available reading state after an injected WebKit startup failure, followed by 308 passing native assertions across Markdown source edits, EPUB with rapid replacement, encrypted PDF, and unavailable storage. The expanded checks cover edits during startup, stale readiness and password responses, navigation superseding an opening, and source changes during failed close. All 28 Rust tests pass with and without desktop features, formatting and strict Clippy pass, and the ordinary release executable was rebuilt without smoke hooks. The removed standalone SaveGate test is replaced by native opening-interface coverage. A final review also reproduced and fixed document actions bypassing the mapped-candidate guard and a failed close retaining an older in-memory position than its successful renderer checkpoint. These are functional checks using synthetic fixtures and private X11 displays, not new performance or Wayland qualification.

## Reading-state ownership · 9 September 2026

The reading-state module now owns retained saves, identity reconciliation, Locate, recents removal, and bookmark-save ordering on one worker. The shell supplies stable positions and presents the latest save status; delayed replies do not restore obsolete errors or discard newer progress. The database schema, locator format, debounce policy, and shutdown checkpoint behavior are preserved.

The [validation record](qa/reading-state-validation.json) records 29 passing Rust tests with and without desktop features, including 11 new reading-state scenarios using real temporary SQLite databases. Formatting, strict Clippy, 10 installer tests, and 284 native assertions passed across Markdown with source edits, EPUB, PDF, and unavailable-storage flows. These are functional regression checks, not a new performance or release qualification.

A follow-up review reproduced delayed bookmark and recents-removal failures whose feedback disappeared after a later successful save. Explicit action failures now produce their own high-priority toast while the save banner reflects current status. Native checks hold GTK callbacks until both operations finish, then verify visible failure feedback and unchanged stored outcomes.

## Local installation validation · 6 September 2026

The Make workflow adds dependency preflight, release builds, user-local installation, safe upstream updates, Debian packaging, and uninstall without removing reading data. The current [validation record](qa/local-install-validation.json) includes 19 desktop Rust tests, 18 core-only tests (a subset run without desktop features), 10 installer tests, formatting/Clippy/syntax checks, and 655 native assertions across nine synthetic-document flows. It also records AT-SPI, Wayland crash/database recovery, readiness timing, and idle-resource observations.

The package was rebuilt from ordinary desktop features, compared with the release executable, checked for resolved runtime libraries and a valid launcher, and accepted by an apt installation simulation. Staged installation, uninstall, and isolated build cleanup were exercised. Actual system package installation and dependency/toolchain setup were not applied to the host during this validation.

The accessibility test exposed a traversal race when WebKit replaced a loading tree. The test now skips null AT-SPI children, and the complete accessibility check passed after that correction. The broader manual and performance qualification gates below remain open.

## Delivered behavior

A working Rust/GTK4/libadwaita application opens PDF, EPUB, and Markdown directly. It has a quiet recent-documents home, per-document Scroll/Pages and reading appearance, versioned passage locators, transactional progress/bookmarks, contents/search, Back/Forward, focus, fullscreen, PDF zoom/rotation/page entry, image inspection, and source watching for Markdown. The optimized binary and Ubuntu package are built locally; no system packages or default document associations were changed.

The [PRD](MVP_PRD.md) remains the acceptance baseline. This preview is ready for owner evaluation, not a declaration that every release gate has passed. The linked notebook is delivered; highlights remain deferred.

## Module ownership

- `document.rs` defines durable identity, settings, locators, and validation.
- `notebook.rs` and `notebook/history.rs` define the versioned notebook, selected sections, references, validation, and shared text/connection undo. `passage.rs` defines precise quotation targets and exact contextual matching. `desktop/shell/notebook.rs` owns the live editor, responsive layout, asynchronous loading and save presentation; its editing and passages modules own native interaction. The existing reading-state worker retains and persists drafts.
- `desktop/shell/opening.rs` owns candidate preparation, native readiness and cancellation, history/link requests, source watching, deferred reloads, and Home/Close transitions. Active reading state remains saveable until a replacement is ready; candidate failure retains the working reader. Its private state replaces the standalone save gate and caller-managed pending fields.
- `reading_state.rs` owns save retries, provisional identity reconciliation, Locate, recents removal, and bookmark-save ordering on one worker. Operations enter its queue before their replies are awaited, and retained snapshots are independent of reply delivery. The shell supplies stable positions, debounce timers, and presentation; it reads the latest save status instead of replaying status from delayed replies.
- `state.rs` owns schema v3 and the SQLite implementation: transactional record updates, notebooks, bookmarks, recents, and relinking. Reading state uses this implementation directly, including real temporary SQLite databases in its tests.
- `resource.rs` validates archive/resource limits and serves only capability-scoped resources. Markdown sidecars resolve within the source directory after canonicalization; EPUBs are not extracted to disk.
- `markdown.rs` renders the defined dialect and creates stable content-block IDs with source offsets.
- `desktop/pdf.rs` adapts Papers models, native page coordinates, async loading, search, and settled restoration.
- `desktop/reflow.rs` owns WebKit setup, local resource delivery, lifecycle cancellation, and typed messages. The native bridge exists only in a named script world.
- `desktop/shell.rs` coordinates a single active document; controls, home, navigation, and persistence live in separate modules.
- `assets/reader.js`, `continuous.js`, and `anchors.js` own reflow presentation and position restoration; `passages.js` owns selected quotation capture, verification and indication. Continuous reading keeps at most five neighboring chapter frames, including during replacement loads. The pinned foliate-js paginator has a local patch for standalone SVG spine documents: body-free DOM access and a single fitted page. Papers' manual bindings add safe rectangle construction and selection-style access; generated bindings and the engine remain unchanged.

Native calls remain on the GTK thread except work dispatched through Papers' scheduler. ZIP/Markdown resource work uses background tasks. PDFs have a 64 MiB page cache; reflow resources are tied to the active document. Search is incremental and capped at 500 displayed matches. Save events coalesce at 750 ms with a two-second maximum requested interval; closing awaits a settled renderer checkpoint and the final save, with a visible failure and a choice to keep the window open. The checkpoint has a three-second deadline.

The resource boundary combines a per-document random host, canonical path checks, archive size/expansion limits, an ephemeral WebKit session, navigation policy, content sanitization, document CSP, disabled authored JavaScript markup, and a private native-message world. These are tested boundaries, not a claim that a general-purpose rendering engine has no security defects.

## Review passes and corrections

1. Compiled and linked the actual native engine graph using checksummed development packages extracted into `/tmp`. Initialized all Papers model/context dependencies.
2. Exercised all three native reading surfaces, reviewed screenshots, refined reading width, margins, toolbar placement, sidebar collapse, and focus behavior.
3. Checked location retention, intentional jumps, search/Back, bookmarks, reopening, generation cancellation, and continuous chapter unloading. Fixed passage capture over illustration/whitespace areas and cross-frame Range detection. Distinguished deliberate jumps from layout-only relocation.
4. Tested recovery and renderer boundaries. Fixed encrypted-PDF retry to reuse the original Papers load job. A native debugger traced a warning to an outline job on PDFs without an outline; the adapter now checks that precondition.
5. Built an optimized executable, checked native runtime dependencies, and generated package dependencies from the actual binary with `dpkg-shlibdeps`.
6. Reviewed the implementation again for everyday reliability. Save requests now enter the worker queue before a subsequent open can overtake them. Markdown locators retain UTF-16 character offsets; repairs use resource identity, stable blocks, or a unique context quote, and clearly identify an approximate fallback. Source reloads preserve focus and history, including atomic file replacement. Native restart checks cover a killed app, an interrupted database writer, and closing during layout.
7. Serialized continuous-chapter maintenance with intentional jumps, bounded frames during loading, and cancelled pending frame listeners on disposal. Real WebKit geometry checks cover delayed images, width changes, reverse traversal, and frame release. Added keyboard image inspection, a modal image view with focus return, independently scrollable code/table regions, useful Markdown progress, ordinary heading fragments, and related-file fragment navigation.
8. Retained the borrowed PDF undo context through widget disposal. PDF saving now retains the intended anchor when a layout change clamps its alignment at a page edge, and an entirely blank gutter keeps the last readable passage. Mixed-size PDF tests distinguish readable horizontal panning from empty space beside a narrower page.

One integration detail matters for maintenance: do not register `readero:` as a WebKit *local/file-like* scheme. A reduced native probe showed that classification prevents blob chapter frames from loading. It is registered as secure and CORS-enabled instead. Parent-owned frame handlers also require the sandbox script capability on this WebKit version; publication execution remains blocked by the settings/sanitization/CSP layers. See the documented upstream [WebKit event-handler issue](https://bugs.webkit.org/show_bug.cgi?id=218086).

## Executed checks

The current safe results are recorded in [hardening checks](qa/hardening.json), [build evidence](qa/build.json), and [performance observations](qa/performance-observations.json). The [original native checks](qa/native-checks.json), [original network check](qa/network-check.json), and [first Wayland idle observation](qa/wayland-idle.json) retain the 0.1.0 baseline. Full local screenshots/logs/locators stay under `/tmp`; private document contents and paths are excluded from deliverable evidence.

- **16 passing Rust unit/integration tests** cover state persistence and relinking, corrupt/newer databases, failed-save rollback, cancelled-open recents, versioned location validation, stale generations, Markdown escaping and block identities, directory/symlink escapes, malformed archives, excessive expansion, save-before-open ordering, original-locator compatibility, and unique Markdown heading fragments.
- Strict Clippy covers authored targets and the opt-in native smoke driver. Formatting checks cover authored Rust and renderer JavaScript/CSS; upstream code is kept intact.
- Native smoke flows exercise opening, Scroll, Pages, warm appearance, contents jumps and Back where an outline exists, search, bookmarks, focus, and reopening. Reflow assertions check that the referenced passage is actually in the viewport, in addition to comparing stored locators.
- The synthetic hostile EPUB includes scripts, event handlers, an external image, a file iframe, and a form. The native bridge is absent from its main world and authored execution stays disabled. A loopback test listener received **zero automatic requests** during the full hostile-publication reading flow.
- All three representative personal technical EPUBs passed 20 mode/font changes with visible passage anchors and at most five mounted chapter frames after settling.
- Actual Wayland checks passed Markdown reading/navigation/resume, watched source edits and atomic replacement, related-file headings and Back/Forward, and the restart recovery probes. Xvfb/Cairo is used for isolated reproducible runs; those screenshots are not evidence of GPU performance.
- The mixed portrait/landscape PDF fixture and encrypted fixture exercise mode changes and horizontal restoration at all four rotations in both modes. An empty gutter in a mixed-size document is not treated as a readable panning target.
- Actual WebKit checks preserve the exact Markdown character offset after insertion/deletion before a passage, repair an edited block by unique quote, avoid guessing duplicate quotes, and keep Unicode excerpts well formed. Delayed image loading and width changes keep the captured passage within three pixels in the dedicated geometry fixture.
- Forced process termination preserves committed position/settings/bookmarks. Killing a writer after dirty-page spill leaves a rollback journal; reopening recovers the preceding commit and passes SQLite integrity checking. Closing immediately after a layout request preserves the intended position and new settings.
- AT-SPI exposes the native controls, named reading modes/appearance action, semantic headings, and document text. This is a tree inspection, not an Orca usability session.
- An unavailable data directory leaves the document readable and shows a persistent saving-error banner. Password tests use an original encrypted fixture and never store a password in reading data.

## Performance observations

The new readiness probe uses the optimized native app on Wayland, with no smoke-navigation delays. For the original Markdown tour, **10 independent launches** had a readiness p95 of **1.493 s**, and **30 reopens** had a p95 of **0.482 s**. For representative technical **epub-02**, the corresponding values were **1.385 s** and **0.460 s**. Individual observations and probe binary hashes are retained in the safe JSON report. A launch includes private D-Bus setup; the OS file cache was not cleared. Reflow readiness requires a stable renderer and a visible passage, but input latency and frame delivery are not measured.

On **epub-02**, after **40 turns and 20 mode changes**, a valid **119.38-second** idle observation measured **0.302% of one logical CPU** across the **eight actual app processes**. Settled process-tree PSS was **194.56 MiB before activity**, **210.61 MiB at the start of idle**, and **216.29 MiB at its end**; the observed idle peak was **221.90 MiB**. Growth stayed below the stated investigation threshold. GPU memory was not measured. The first attempt incorrectly included private desktop helpers whose process set changed; it was rejected and replaced by actual descendant-tree sampling.

These are encouraging diagnostic observations, not full PRD performance qualification. The window was the application's default **1080 × 820 logical pixels**, rather than the PRD's proposed fixed test size, and refresh/scale/power settings were inherited without normalization. The observation does not establish battery behavior, frame-time percentiles, very large-document behavior, or comparative performance against Papers/Foliate. The earlier 0.1.0 one-minute Markdown observation remains available separately.

## Qualification still required

The complete PRD performance protocol remains open: controlled frame-time/input distributions, battery behavior, GPU memory, long-session profiling, the wider ordinary corpus, and comparison runs need further work. The new observations are explicitly scoped above; passing selected native flows does not establish all release gates.

Human Orca usability, the wider real-document corpus, and the complete move/upgrade
matrix still need broader qualification. Notebook checks now cover the 100–200%
Wayland scale matrix, synthetic large/scanned PDFs, and EPUB 2/NCX; that does not
establish compatibility with every publication. The owner should complete at least
five ordinary reading sessions across their actual formats and preferred modes
before declaring the MVP finished. This pass does not replace that daily-use assessment.

The package has been inspected and its dependency resolution checked without installing it into the user's system. Cross-distribution support is not promised. Markdown resources outside the source directory tree are currently refused; there is no broader folder-grant UI in this preview.

## Reproduction

See the build and smoke commands in [README](../README.md). Use `--edit`, `--protected-fixture`, `--storage-failure`, `--rapid PATH`, `--stress`, or `--wayland` for specific flows. The fixture generator creates original PDF/EPUB/adversarial inputs. The protected fixture is created with pypdf by encrypting that original PDF with the documented test-only password `readero-fixture-password`.

The native smoke driver, qualification probes, and resource logging are behind `--features smoke` and absent from normal builds. Read-only renderer diagnostic helpers remain in the private script world. Rebuild without that feature before packaging. Local test logs may contain resource names; retain only the boolean `checks.json` summary when sharing results from personal documents.
