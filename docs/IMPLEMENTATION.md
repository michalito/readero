# Implementation and quality record

5 September 2026 · Readero 0.1.1 personal preview

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

The [PRD](MVP_PRD.md) remains the acceptance baseline. This preview is ready for owner evaluation, not a declaration that every release gate has passed. Highlights and the linked notebook remain deliberately deferred.

## Module ownership

- `document.rs` defines durable identity, settings, locators, and validation.
- `desktop/shell/opening.rs` owns candidate preparation, native readiness and cancellation, history/link requests, source watching, deferred reloads, and Home/Close transitions. Active reading state remains saveable until a replacement is ready; candidate failure retains the working reader. Its private state replaces the standalone save gate and caller-managed pending fields.
- `reading_state.rs` owns save retries, provisional identity reconciliation, Locate, recents removal, and bookmark-save ordering on one worker. Operations enter its queue before their replies are awaited, and retained snapshots are independent of reply delivery. The shell supplies stable positions, debounce timers, and presentation; it reads the latest save status instead of replaying status from delayed replies.
- `state.rs` owns schema v1 and the SQLite implementation: transactional record updates, bookmarks, recents, and relinking. Reading state uses this implementation directly, including real temporary SQLite databases in its tests.
- `resource.rs` validates archive/resource limits and serves only capability-scoped resources. Markdown sidecars resolve within the source directory after canonicalization; EPUBs are not extracted to disk.
- `markdown.rs` renders the defined dialect and creates stable content-block IDs with source offsets.
- `desktop/pdf.rs` adapts Papers models, native page coordinates, async loading, search, and settled restoration.
- `desktop/reflow.rs` owns WebKit setup, local resource delivery, lifecycle cancellation, and typed messages. The native bridge exists only in a named script world.
- `desktop/shell.rs` coordinates a single active document; controls, home, navigation, and persistence live in separate modules.
- `assets/reader.js`, `continuous.js`, and `anchors.js` own reflow presentation and passage capture/restoration. Continuous reading keeps at most five neighboring chapter frames, including during replacement loads. The pinned foliate-js paginator has a local patch for standalone SVG spine documents: body-free DOM access and a single fitted page. Papers bindings remain unmodified.

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

Orca usability, the full 100–200% display-scale matrix, large/scanned PDF stress, EPUB 2 coverage, and the complete move/upgrade matrix still need broader qualification. The owner should complete at least five ordinary reading sessions across their actual formats and preferred modes before declaring the MVP finished. This pass substantially improves the preview; it does not replace that daily-use assessment.

The package has been inspected and its dependency resolution checked without installing it into the user's system. Cross-distribution support is not promised. Markdown resources outside the source directory tree are currently refused; there is no broader folder-grant UI in this preview.

## Reproduction

See the build and smoke commands in [README](../README.md). Use `--edit`, `--protected-fixture`, `--storage-failure`, `--rapid PATH`, `--stress`, or `--wayland` for specific flows. The fixture generator creates original PDF/EPUB/adversarial inputs. The protected fixture is created with pypdf by encrypting that original PDF with the documented test-only password `readero-fixture-password`.

The native smoke driver, qualification probes, and resource logging are behind `--features smoke` and absent from normal builds. Read-only renderer diagnostic helpers remain in the private script world. Rebuild without that feature before packaging. Local test logs may contain resource names; retain only the boolean `checks.json` summary when sharing results from personal documents.
