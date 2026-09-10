# Readero future features

Updated 7 September 2026. Consolidated from the recorded product discussion, the [MVP PRD](MVP_PRD.md), and the [stack decisions](STACK_DECISIONS.md). The working foundation is committed; implementation and qualification evidence live in [IMPLEMENTATION.md](IMPLEMENTATION.md).

This is the reference for future feature ideas. **Recorded direction** means an intended capability captured in the discussion or PRD; its detailed specification and delivery order can still change. **Optional** means an idea to evaluate, with no delivery commitment. The sequence below is a proposal, not an approved release schedule.

## Product priorities that continue to apply

Readero is a personal Ubuntu reader. Make it comfortable to return to reading, easy to keep a train of thought, and trustworthy about saved work. Performance, restrained design, keyboard access, and maintainable code remain requirements as features grow.

Scroll and Pages, saved reading position and appearance, bookmarks, navigation history, focus, and fullscreen already belong to the foundation. Improve them when ordinary reading exposes friction. Both reading modes remain available according to mood and content; focus and fullscreen remain independent choices.

## Recorded future direction

### F01 — A notebook beside the document

**Origin:** the owner's request for a side-panel notepad for notes and examples; PRD §8 and stack decision D10.

Provide a continuous notebook belonging to each document, suitable for questions, worked examples, snippets, and longer thoughts. The reader can keep it open while reading or hide it for an uncluttered view. The existing architecture allows a right sidebar; the final width, resizing behavior, and narrow-window layout need a design pass.

The notebook stays at the user's editing position while the document moves. Turning a page must not replace the note being written, move its cursor, or steal keyboard focus. Notes can exist without a location link.

Before implementation, decide the editing format—plain text or a defined Markdown subset—and the simplest useful structure for a document's notebook. A rich-text editor, multiple notebooks per document, and a workspace system have not been selected.

Proposed acceptance: write an example, navigate and search the document, switch reading modes, hide/reopen the panel, and restart the app. The text, attached references, and intended editing context remain intact. Saving failures are visible and preserve the current draft.

### F02 — Page and passage links in notes

**Origin:** the owner's request for notes that track pages and examples; PRD §8.

Show an unobtrusive indicator of the current reading location and provide an action to attach it to a note or example. Following a saved reference returns to the relevant passage, including after switching Scroll/Pages or changing typography. Back returns to the previous reading location.

Tracking updates the location indicator as the reader moves. It must not silently retarget references already attached to notes. PDF references use document pages/coordinates; reflowable content uses passage identity rather than a temporary screen-page number.

Before implementation, decide how a reference appears in the editor, whether one note can have several references, and whether attaching a selected quotation is useful in the first version. Selection-to-note is a design question, not an already approved interaction.

Proposed acceptance: attach a reference while writing an example, move elsewhere, change layout, restart, and follow the reference. The intended passage returns and the note remains stable.

### F03 — Highlights and annotation comments

**Origin:** the owner's request for convenient future annotation; PRD §8.

Select a passage, create a highlight, revisit it, and optionally attach a comment. Keep the interaction quick and unobtrusive. PDF, EPUB, and Markdown need explicit acceptance cases; they need not all receive annotation editing in the same release.

A reading-point locator is only the foundation for this work. Highlights require selected-range identity and rendering, revision-aware recovery, and clear handling of ambiguous or missing text. A reading position may fall back to a nearby passage; an annotation must not silently attach to different words. Preserve an unresolved annotation and provide a way to reattach it, as anticipated in the stack decisions.

Before implementation, decide which format goes first, how highlight comments relate to the notebook, the minimum annotation controls, and how removal/recovery should work. Highlight colors, tags, a toolbar design, and direct modification of PDF files have not been selected.

Proposed acceptance: highlight and comment on a passage; change mode, zoom/typography, and document location; restart; then revisit it. Edit or replace the source and verify that uncertain attachment is visible without losing the annotation.

### F04 — Export notes and annotations

**Origin:** eventual export recorded in PRD §8. The export formats remain undecided.

Allow the owner to take their notes, examples, comments, quotations where available, and document references out of the app in a useful form. Markdown is a candidate for notebook export; a versioned structured format is a candidate for preserving richer reference data. Neither has been selected yet.

Notebook export and PDF annotation interoperability are separate pieces of work. Writing annotations into a PDF, round-tripping with other readers, and import/restore behavior need their own specification and validation. Existing reading locators do not establish interoperability.

Proposed acceptance: export a representative notebook with multiple references and annotations, open it independently, and verify that text and reference meaning survive. Define what remains navigable outside Readero and explain any unsupported data.

## Previously recorded optional improvements

### F05 — Notes near the current passage

**Optional; PRD §8.** Offer a view or filter for notes attached near the current page/passage. Keep the continuous notebook available and preserve an active draft while the reading location changes. Define “near” for each format and evaluate whether this helps after the basic notebook has been used.

### F06 — Two-page spreads

**Optional; PRD FR04.** Offer a two-page spread for reflowable content when the screen and reading preference suit it. Single-column Pages and Scroll remain available. Specify narrow-window fallback, reading order, oversized content, and passage retention before implementation. PDF facing-page behavior would require a separate decision.

### F07 — Footnote and endnote popups

**Optional; PRD FR04.** Let the reader inspect a footnote without leaving the surrounding passage. Ordinary link navigation and Back remain usable. Validate long notes, keyboard dismissal, focus return, and screen-reader behavior.

### F08 — Syntax coloring for technical reading

**Optional; PRD FR05 and §9.** Evaluate syntax coloring for Markdown code blocks if everyday technical reading would benefit. Specify supported languages, theme contrast, loading cost, and behavior for large or unrecognized blocks. Plain code, whitespace preservation, copying, and scrolling must continue to work.

### F09 — Explicit access to another resource folder

**Optional extension of stack decision D07; current limitation recorded in IMPLEMENTATION.md.** Consider a folder-grant flow for Markdown whose local images or linked resources live outside the source directory tree. Define the scope, persistence, and revocation of a grant. The current resource boundary remains the baseline until that flow is designed and tested.

## Scope exclusions retained for reference

The PRD explicitly excludes the following from MVP. Recording an exclusion does **not** put it on the delivery roadmap; there is no commitment to build these:

- Cloud accounts/sync, managed libraries and catalog import.
- OCR, PDF reflow, PDF forms/signing/redaction, and editing source documents.
- DRM integration, fixed-layout EPUB guarantees, TTS/media overlays, and scripted publication interactivity.
- Multi-document tab workspaces, Mermaid/LaTeX Markdown extensions, and mobile/Windows/macOS distribution.
- AI summaries/chat and reading streaks.

Syntax coloring is also excluded from MVP and is retained above as an optional technical-reading improvement. Growth, monetization, social features, and engagement metrics remain outside the product's purpose.

## Proposed order

1. Use the foundation for ordinary reading and address concrete friction. Continue the outstanding MVP qualification work recorded in IMPLEMENTATION.md: Orca usability, display scaling, broader document coverage, and controlled performance/long-session measurements.
2. Specify and build the smallest useful notebook with location links (F01–F02). Prototype the side-panel reading and writing flow before choosing an editor implementation or expanding the data model.
3. Add highlights and comments (F03), with explicit format coverage and anchor-recovery rules.
4. Specify export (F04) alongside the note data model so information needed for export is retained. Delivery can move earlier if portability becomes a priority; it need not wait for every annotation format.
5. Evaluate F05–F09 individually against actual reading needs. Their numbering is an identifier, not a priority ranking.

## Quality requirements for each addition

Preserve the reading passage, original documents, and saved user work. Keep the notebook and annotation work local and usable offline. Define persistence, migration/recovery, and format limitations before storing new durable data.

Load and render only what is needed. Establish realistic long-notebook and annotation-volume cases before claiming bounded performance. Preserve keyboard navigation, text selection/copying, visible focus, and reading comfort with the new controls open and closed.

Use multiple passes: review the intended flow, review the implementation and data boundaries, exercise meaningful native regressions, inspect the design, and then collect ordinary-use feedback. Avoid adding empty future panels or speculative subsystems to the foundation.

When an idea is selected, give it a focused specification with acceptance criteria and open decisions. Update this file when an idea is delivered, deferred, or dropped, and put measured results in the implementation record.
