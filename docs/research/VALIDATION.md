# Stack validation record

Historical research snapshot, 4 September 2026. The app has since been implemented; see [implementation and QA](../IMPLEMENTATION.md) for later results. The statements below describe the earlier component-research stage.

Research deliverables: [stack decision record](../STACK_DECISIONS.md) and [MVP PRD](../MVP_PRD.md).

## Evidence levels

- **User requirement:** personal Ubuntu app; comfortable reading; PDF/EPUB/Markdown; selectable Scroll/Pages modes; performance/focus; saved position; future annotations and a document notebook with location links.
- **Observed environment/corpus:** directly inspected this machine and the authorized Downloads documents. Original files were read only. Reports use aliases; filename mappings and rendered samples remain under `/tmp/readero-research`, outside the project deliverables.
- **Source/API evidence:** read official project documentation and pinned source for relevant behavior. These establish available APIs and implementation constraints, not app-level quality.
- **Executed component evidence:** small native and web-renderer probes ran against installed libraries on an isolated Xvfb display. These demonstrate specified behaviors in the sampled cases.
- **Unmeasured release targets:** the app's startup, Wayland scrolling, display scaling, memory/CPU, battery behavior, accessibility, and full persistence/identity implementation still require the application.

## Environment

[environment.json](evidence/environment.json) records Ubuntu 26.04.1, the CPU/RAM, installed engine versions, and Rust 1.98.0. Runtime libraries were already present. For Python/GJS introspection, a few matching Ubuntu metadata packages were downloaded, checksum-checked, and extracted under `/tmp`; they were not installed into the system.

`pkg-config`, native development headers, Meson, and Flatpak are not available in the research setup. That did not prevent introspection tests, but a full native Rust application was not compiled/linked. Future implementation will need the appropriate development packages. No desktop setting, default application association, or installed package was changed by this work.

Xvfb produced expected DRI3 warnings and used software rendering. A first sandboxed GTK construction attempt failed; the isolated display tests ran successfully outside that filesystem/process sandbox. This is not evidence of a live-Wayland renderer defect, and no performance conclusion is drawn from it.

## Local document corpus

[corpus-profile.json](evidence/corpus-profile.json) contains structural metadata for 19 EPUBs and 14 PDFs. No Markdown samples were found. The EPUBs are technical publications with up to 230 image resources, 77 tables in one book, and large amounts of code markup; some contain MathML. Their largest XHTML file is about 265 KiB, much smaller than their image-heavy ZIP archives. This supports testing archive loading and decoded images separately from text-layout size.

All sampled EPUBs declare reflowable EPUB 3.0. There were no XHTML XML parse failures in the structural scan. These observations do not prove all sections render correctly. EPUB 2, fixed layout, RTL/vertical writing, malicious/malformed publications, and font-obfuscation paths were not validated by this sample.

PDFs range from one to six pages. `pdf-02` returned Poppler's encrypted-document error without a supplied password; the report's generic `error` field represents that expected protection case, not diagnosed corruption. `pdf-01` is an image scan with no extracted text. Password entry/unlock was not exercised because no password was requested or supplied.

No source text, passwords, personal filenames, or document screenshots are embedded in the PRD or stack report. Do not commit or publish the private `/tmp` filename mapping or rendered samples.

## PDF component execution

Probe: [native_pdf_probe.py](probes/native_pdf_probe.py). Results: [native-pdf-probe.json](evidence/native-pdf-probe.json).

Three samples were loaded through Papers 50.2 and displayed in GTK. Checks exercised mode changes, `select_all`/selected-text retrieval, document-coordinate lookup, navigation through a coordinate link, and search. The two text PDFs produced selection text and 45/21 search results for locally derived tokens. Both reading-mode transitions kept the current page in all three cases. The coordinate API returned document points and the link returned to the intended page.

Limit: this checks page preservation and coordinate availability, not pixel-exact restoration; it does not validate mouse selection of a multicolumn passage. Full-document selection is not a substitute for that test. The image scan's empty selection is expected. Its first page was rendered and visually inspected locally; the original scan's image quality remains a source limitation.

Initial simplified embeddings crashed because required models/contexts were omitted. Reading the pinned Papers application's builder definition identified the annotation model, undo/annotation contexts, and search-context wiring. With those supplied before document display, the probe completed. The final probe records that setup; it does not patch the installed library.

## EPUB component execution

Probe: [webkit_epub_probe.py](probes/webkit_epub_probe.py). WebKitGTK 2.52.6 loaded local ZIP entries via an application URI handler. The harness deliberately does not open a network HTTP server. Its CSP prevents automatic external web-resource requests in the tested paths. The harness is not an adversarial security qualification or a production bridge.

Foliate evidence:

- [epub-01](evidence/webkit-foliate-epub-01.json): the largest archive, 57 spine sections, produced a CFI; a switch through scrolled/paginated modes retained a resolvable section/passage and the captured anchor returned to the paginated visible range.
- [epub-02](evidence/webkit-foliate-epub-02.json): a table/code-rich book, 28 sections, passed the same checks.
- [epub-02 continuous proof](evidence/webkit-continuous-epub-02.json) and [epub-14 continuous proof](evidence/webkit-continuous-epub-14.json): three mounted section frames; adjacent chapters visible together; zero measured viewport drift after prepending the preceding section; a CFI resolved to the same paragraph; returning to Pages put that anchor in the visible range.

The initial Foliate mode test chooses the largest text resource, which can be an index rather than an ordinary chapter. The continuous proof also exercises ordinary neighboring spine sections. The scripts do not inspect every image/table/math expression in every book. Generated EPUB snapshots were visually inspected locally for a technical-text/chapter-boundary view and a page containing a code block and diagrams; that is representative inspection, not whole-corpus visual QA. The harness used Foliate's default column count; the PRD explicitly chooses a single-column paginated view for MVP.

The continuous proof mounts a fixed three-section window and compensates one insertion. It deliberately lacks production virtualization/unloading, repeated width changes, delayed-image stress, cross-frame selection, and cancellation/error handling. The evidence supports feasibility and the reusable locator model; it does not establish that the final scrolling controller is finished.

EPUB.js evidence: [epub-02 comparison](evidence/webkit-epubjs-epub-02.json) produced a CFI, kept the section after changing modes, and displayed two chapter frames in vertical flow with the boundary visible. The probe explicitly loads the neighboring section to test the manager's layout. It does not prove automatic prefetch correctness under rapid user scrolling. A missing optional Apple display-options file generated a harmless failed lookup in this corpus. The latest package publication date was checked against npm: version 0.3.93, February 2022.

Readium 2.8.2 was source-reviewed only. MuPDF, PDF.js, Qt, Electron, and Tauri were not benchmarked locally. Their comparison in the decision record is based on documented capabilities and integration cost, not measured speed rankings.

## Markdown parser and SQLite execution

[Markdown probe source](probes/markdown/src/main.rs) compiled and ran with pulldown-cmark 0.13.4. It asserts table/task-list/fence/footnote output, preserved relative links, and UTF-8-boundary source ranges. [Result](evidence/markdown-probe.json). It does not test visual pagination, HTML escaping/resource policy, edit recovery, syntax highlighting, or DOM-to-source mapping.

[SQLite probe](probes/sqlite_probe.py) committed a valid locator, killed a separate writer during an uncommitted replacement, then reopened the database. The preceding locator survived and `integrity_check` returned `ok`. [Result](evidence/sqlite-probe.json). This establishes the expected transactional behavior in that process-crash case; it is not a power-loss/disk-full test and does not implement Readero's saving pipeline.

## Rust compatibility

[rust-compatibility.json](evidence/rust-compatibility.json) and the [resolved lockfile](evidence/compatibility.Cargo.lock) show a successful dependency resolution using GTK 0.10 / GLib 0.21 / libadwaita 0.8 / webkit6 0.5 / Papers 50.2 bindings, plus the parser and SQLite wrapper. A single GLib family was present.

This does not prove every required Rust API was called successfully. Full compile/link and native runtime wiring are M0 requirements. The lockfile is research evidence; the future application should create its own reproducible manifest/lockfile around the vendored Papers bindings.

## Reproduction

Run from the target Ubuntu machine. Public downloads need internet access. The probes require Python GI/cairo, the installed native runtimes, and Xvfb. Inspect the scripts before adapting them to another machine; package/runtime versions must match.

1. Run `python3 docs/research/probes/fetch_public_inputs.py`. It downloads pinned public source and extracts introspection metadata under `/tmp/readero-research`; it does not install packages. [Original source/package checksums](evidence/public-inputs.json) record the research inputs.
2. Set `GI_TYPELIB_PATH=/tmp/readero-research/gi/usr/lib/x86_64-linux-gnu/girepository-1.0` for the Python probes.
3. Run `python3 docs/research/probes/profile_corpus.py`. It reads the current Downloads directory and generates a private alias/path mapping under `/tmp`. Aliases are tied to the current sorted corpus; use the recorded files/corpus for a like-for-like rerun.
4. Run the PDF probe and each EPUB probe through `xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo`, preserving `GI_TYPELIB_PATH`. EPUB arguments used were `foliate epub-01`, `foliate epub-02`, `continuous epub-02`, `continuous epub-14`, and `epubjs epub-02`.
5. Run `python3 docs/research/probes/sqlite_probe.py`.
6. Run `python3 docs/research/probes/resolve_rust.py` for dependency resolution. Run `cargo run --manifest-path docs/research/probes/markdown/Cargo.toml` to compile/execute the parser fixture. Use a task-specific `CARGO_HOME`/`CARGO_TARGET_DIR` if keeping caches outside the project.

The EPUB/PDF probes print diagnostic timings that include harness choices and settling delays. Do not rank the engines from these numbers or use them as PRD startup measurements.

## Remaining qualification

The research is complete enough to select an implementation baseline. The following require actual implementation and are explicitly preserved as M0–M3 release gates:

- Full Rust GUI compilation/linking and production resource/IPC boundaries.
- True continuous-flow virtualization, delayed font/image handling, reverse traversal, resize, memory bounds, and mode/locator recovery.
- Markdown visual pagination and changed-source anchor recovery.
- Long/malformed documents, EPUB 2, supported font behavior, PDF password entry, and reading-state recovery under storage failures.
- Native Wayland display scaling, touchpad smoothness, measured process-tree memory/idle CPU, and battery observations.
- Core keyboard flows, content accessibility with Orca, packaging/install/upgrade and Open With integration.

No unexecuted gate is labeled passed. No comparative performance improvement has been established.
