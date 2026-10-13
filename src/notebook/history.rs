//! One bounded history for native text splices and semantic connections.
use super::{Notebook, Reference, Section};
use crate::document::{Error, Result};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Edit {
    pub start: i32,
    pub removed: i32,
    pub text: String,
}

#[derive(Clone)]
struct Splice {
    at: i32,
    removed: String,
    inserted: String,
    before: Vec<Section>,
    after: Vec<Section>,
}

#[derive(Clone, PartialEq)]
struct Metadata {
    references: Vec<Reference>,
    sections: Vec<Section>,
}
impl Metadata {
    fn read(n: &Notebook) -> Self {
        Self {
            references: n.references.clone(),
            sections: n.sections.clone(),
        }
    }
    fn restore(&self, n: &mut Notebook) {
        n.references.clone_from(&self.references);
        n.sections.clone_from(&self.sections);
    }
}

/// Store only changed records. A rename must not retain another copy of every
/// quotation in the notebook for each keystroke.
struct Delta<T> {
    before: Vec<(usize, T)>,
    after: Vec<(usize, T)>,
}
impl<T: Clone + PartialEq> Delta<T> {
    fn between(before: Vec<T>, after: &[T], id: fn(&T) -> &str) -> Self {
        let old: std::collections::HashMap<_, _> = before.iter().map(|v| (id(v), v)).collect();
        let changed_after = after
            .iter()
            .enumerate()
            .filter(|(_, v)| old.get(id(v)).copied() != Some(v))
            .map(|(i, v)| (i, v.clone()))
            .collect();
        let new: std::collections::HashMap<_, _> = after.iter().map(|v| (id(v), v)).collect();
        let changed_before = before
            .into_iter()
            .enumerate()
            .filter(|(_, v)| new.get(id(v)).copied() != Some(v))
            .collect();
        Self {
            before: changed_before,
            after: changed_after,
        }
    }
    fn restore(&self, records: &mut Vec<T>, forward: bool, id: fn(&T) -> &str) {
        let changed: std::collections::HashSet<_> = self
            .before
            .iter()
            .chain(&self.after)
            .map(|(_, v)| id(v))
            .collect();
        records.retain(|v| !changed.contains(id(v)));
        let target = if forward { &self.after } else { &self.before };
        for (index, record) in target {
            records.insert((*index).min(records.len()), record.clone());
        }
    }
    fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty()
    }
}
struct MetadataChange {
    references: Delta<Reference>,
    sections: Delta<Section>,
}
enum Change {
    Text(Splice),
    Metadata(MetadataChange),
}
struct Transaction {
    changes: Vec<Change>,
    before: (i32, i32),
    after: (i32, i32),
}

#[derive(Default)]
pub struct History {
    undo: VecDeque<Transaction>,
    redo: Vec<Transaction>,
    pending: Option<Transaction>,
    depth: u32,
}

fn byte_at(text: &str, offset: i32) -> usize {
    text.char_indices()
        .nth(offset.max(0) as usize)
        .map_or(text.len(), |(i, _)| i)
}

fn transform(sections: &mut Vec<Section>, start: i32, end: i32, inserted: i32) {
    let delta = inserted - (end - start);
    let crosses_sections = inserted > 0
        && sections
            .iter()
            .filter(|s| s.start < end && s.end > start)
            .take(2)
            .count()
            > 1;
    sections.retain_mut(|s| {
        if s.end <= start {
            return true;
        }
        if s.start >= end {
            s.start += delta;
            s.end += delta;
            return true;
        }
        if crosses_sections {
            return false;
        }
        if s.start == start && s.end == end && inserted > 0 {
            s.end = start + inserted;
            return true;
        }
        if start <= s.start && end >= s.end {
            return false;
        }
        if s.start >= start {
            s.start = start + inserted;
        }
        s.end = if s.end <= end { start } else { s.end + delta };
        s.end > s.start
    });
}

impl Splice {
    fn apply(&self, n: &mut Notebook, forward: bool) -> Edit {
        let (remove, insert, restore) = if forward {
            (&self.removed, &self.inserted, &self.after)
        } else {
            (&self.inserted, &self.removed, &self.before)
        };
        let end = self.at + remove.chars().count() as i32;
        let a = byte_at(&n.text, self.at);
        let b = byte_at(&n.text, end);
        n.text.replace_range(a..b, insert);
        transform(&mut n.sections, self.at, end, insert.chars().count() as i32);
        // Restore only sections touched at the splice, including fully deleted
        // sections. All later ranges move through the inverse transform.
        n.sections
            .retain(|s| !self.before.iter().chain(&self.after).any(|r| r.id == s.id));
        n.sections.extend(restore.iter().cloned());
        n.sections.sort_by_key(|s| s.start);
        Edit {
            start: self.at,
            removed: end - self.at,
            text: insert.clone(),
        }
    }
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn begin(&mut self, n: &Notebook) {
        if self.depth == 0 {
            self.pending = Some(Transaction {
                changes: Vec::new(),
                before: (n.cursor, n.selection),
                after: (n.cursor, n.selection),
            });
        }
        self.depth += 1;
    }

    pub fn end(&mut self, n: &Notebook) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth == 0
            && let Some(mut transaction) = self.pending.take()
        {
            if transaction.changes.is_empty() {
                return;
            }
            transaction.after = (n.cursor, n.selection);
            self.redo.clear();
            self.undo.push_back(transaction);
            while self.undo.len() > 100 {
                self.undo.pop_front();
            }
        }
    }

    pub fn edit(&mut self, n: &mut Notebook, start: i32, end: i32, text: &str) -> Result<()> {
        if start < 0 || end < start || end as usize > n.text.chars().count() || text.contains('\0')
        {
            return Err(Error::Invalid(
                "The notebook edit has an invalid text range.".into(),
            ));
        }
        if start == end && text.is_empty() {
            return Ok(());
        }
        let own = self.depth == 0;
        if own {
            self.begin(n);
        }
        let mut end = end;
        // GTK replacement is a delete followed by an insert in one user action.
        // Treat that pair as one splice so replacing a linked section preserves it.
        if !own
            && start == end
            && !text.is_empty()
            && let Some(Change::Text(previous)) =
                self.pending.as_ref().and_then(|p| p.changes.last())
            && previous.at == start
            && previous.inserted.is_empty()
            && !previous.removed.is_empty()
        {
            let previous = previous.clone();
            self.pending.as_mut().unwrap().changes.pop();
            previous.apply(n, false);
            end += previous.removed.chars().count() as i32;
        }
        let before: Vec<_> = n
            .sections
            .iter()
            .filter(|s| s.end >= start && s.start <= end)
            .cloned()
            .collect();
        let a = byte_at(&n.text, start);
        let b = byte_at(&n.text, end);
        let removed = n.text[a..b].to_owned();
        n.text.replace_range(a..b, text);
        transform(&mut n.sections, start, end, text.chars().count() as i32);
        let after = n
            .sections
            .iter()
            .filter(|s| before.iter().any(|r| r.id == s.id))
            .cloned()
            .collect();
        n.cursor = start + text.chars().count() as i32;
        n.selection = n.cursor;
        n.top = n.top.min(n.text.chars().count() as i32);
        self.pending
            .as_mut()
            .unwrap()
            .changes
            .push(Change::Text(Splice {
                at: start,
                removed,
                inserted: text.into(),
                before,
                after,
            }));
        if own {
            self.end(n);
        }
        Ok(())
    }

    /// Metadata operations cannot change text. Commit the entire operation or
    /// restore it on validation failure, preserving the previous undo history.
    pub fn metadata<T>(
        &mut self,
        n: &mut Notebook,
        change: impl FnOnce(&mut Notebook) -> Result<T>,
    ) -> Result<T> {
        let before = Metadata::read(n);
        let result = change(n).and_then(|result| n.validate().map(|()| result));
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                before.restore(n);
                return Err(error);
            }
        };
        let delta = MetadataChange {
            references: Delta::between(before.references, &n.references, |r| &r.id),
            sections: Delta::between(before.sections, &n.sections, |s| &s.id),
        };
        if !delta.references.is_empty() || !delta.sections.is_empty() {
            let own = self.depth == 0;
            if own {
                self.begin(n);
            }
            self.pending
                .as_mut()
                .unwrap()
                .changes
                .push(Change::Metadata(delta));
            if own {
                self.end(n);
            }
        }
        Ok(result)
    }

    pub fn undo(&mut self, n: &mut Notebook, redo: bool) -> Vec<Edit> {
        while self.depth > 0 {
            self.end(n);
        }
        let transaction = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        let Some(transaction) = transaction else {
            return Vec::new();
        };
        let changes: Box<dyn Iterator<Item = &Change>> = if redo {
            Box::new(transaction.changes.iter())
        } else {
            Box::new(transaction.changes.iter().rev())
        };
        let mut edits = Vec::new();
        for change in changes {
            match change {
                Change::Text(splice) => edits.push(splice.apply(n, redo)),
                Change::Metadata(change) => {
                    change
                        .references
                        .restore(&mut n.references, redo, |r| &r.id);
                    change.sections.restore(&mut n.sections, redo, |s| &s.id);
                }
            }
        }
        let (cursor, selection) = if redo {
            transaction.after
        } else {
            transaction.before
        };
        n.cursor = cursor;
        n.selection = selection;
        n.top = n.top.min(n.text.chars().count() as i32);
        if redo {
            self.undo.push_back(transaction);
        } else {
            self.redo.push(transaction);
        }
        edits
    }
}
