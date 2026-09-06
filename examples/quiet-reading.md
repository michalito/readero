# A little room to read

Good reading starts with a small invitation: a clear page, a comfortable place,
and enough quiet to follow a thought wherever it leads.

This document is a small tour of Readero. Try **Scroll** for a continuous view,
or **Pages** when you want one screen at a time. Your place belongs to the
passage, even when the layout changes.

## Make the page yours

The **Aa** menu controls the reading surface. A warm page can feel gentler late
in the day. A wider column makes space for a technical table; a narrower one
keeps a long essay comfortable.

> Leave enough space around the words that the words can do their work.

Use **F9** to focus. Press it again, or Escape, to bring the controls back.
Fullscreen is separate, on **F11**.

## Keep a useful passage

Press **Ctrl+D** to bookmark the current passage. Open the sidebar to revisit
your bookmarks, browse the contents, or search through the whole document.

The next time you open this file, Readero restores your reading position and
the choices you made for this document. Your original file stays unchanged.

## Read the details

Technical reading is rarely a straight line. You follow a reference, inspect an
example, compare two definitions, and return to the sentence you were reading.
Back and Forward keep those intentional jumps separate from ordinary scrolling.

```rust
fn return_to_reading(passage: &Passage) {
    reader.restore(passage);
    reader.show_the_words();
}
```

Long code lines retain their whitespace and can scroll horizontally. The same
is true of wide tables: the document stays useful at a comfortable text size.

| A small action | A useful result |
| --- | --- |
| Open a document | Read it where it already lives |
| Change the layout | Keep the same passage visible |
| Save a bookmark | Return to an idea worth keeping |
| Enter focus mode | Give the page a little more room |

## A place for future thoughts

A future notebook will belong to each document. It will provide a stable place
for questions, examples, and notes, with links back to the passages that
inspired them. For now, the work is to make the reading itself dependable.

There is no account to create and no library to maintain. Just choose something
you want to read, settle in, and begin.

### Keyboard reference

- **Ctrl+O** — open a document
- **Ctrl+F** — search this document
- **Ctrl+D** — bookmark this passage
- **Alt+Left / Alt+Right** — back and forward
- **F8** — contents and bookmarks
- **F9** — focus
- **F11** — fullscreen
- **Escape** — return to the reading controls

### A note on text

Reading includes more than ASCII: café, naïve, Ελληνικά, 日本語, and 🦀 all belong
in a document. Passage references keep source bytes distinct from the browser's
text offsets, so changing the view does not depend on counting letters as bytes.

---

Take your time. The page will be here when you return.
