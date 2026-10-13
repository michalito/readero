//! A document's plain text, explicit passage references, and editing context.
use crate::document::{Error, Locator, Result};
use serde::{Deserialize, Serialize};

mod history;
pub use history::{Edit, History};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub id: String,
    pub label: String,
    pub locator: Locator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passage: Option<crate::passage::Passage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub start: i32,
    pub end: i32,
    pub passages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notebook {
    pub version: u32,
    pub text: String,
    pub references: Vec<Reference>,
    #[serde(default)]
    pub sections: Vec<Section>,
    /// GTK character offsets, not UTF-8 bytes or DOM UTF-16 offsets.
    pub cursor: i32,
    pub selection: i32,
    /// First visible editor character; stable when the panel width changes.
    pub top: i32,
    pub visible: bool,
    pub width: i32,
    pub height: i32,
}

impl Default for Notebook {
    fn default() -> Self {
        Self {
            version: 2,
            text: String::new(),
            references: Vec::new(),
            sections: Vec::new(),
            cursor: 0,
            selection: 0,
            top: 0,
            visible: false,
            width: Self::DEFAULT_WIDTH,
            height: Self::DEFAULT_HEIGHT,
        }
    }
}

impl Notebook {
    pub const MIN_WIDTH: i32 = 260;
    pub const MIN_HEIGHT: i32 = 180;
    pub const MAX_SIZE: i32 = 800;
    pub const DEFAULT_WIDTH: i32 = 340;
    pub const DEFAULT_HEIGHT: i32 = 280;

    pub fn validate(&self) -> Result<()> {
        let length = self.text.chars().count();
        if !(1..=2).contains(&self.version)
            || (self.version == 1
                && (!self.sections.is_empty()
                    || self.references.iter().any(|r| r.passage.is_some())))
            || self.text.contains('\0')
            || self.cursor < 0
            || self.selection < 0
            || self.cursor as usize > length
            || self.selection as usize > length
            || self.top < 0
            || self.top as usize > length
            || !(Self::MIN_WIDTH..=Self::MAX_SIZE).contains(&self.width)
            || !(Self::MIN_HEIGHT..=Self::MAX_SIZE).contains(&self.height)
        {
            return Err(Error::Invalid(
                "The notebook uses an unsupported or invalid format. It has been left unchanged."
                    .into(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for reference in &self.references {
            if reference.id.is_empty()
                || !ids.insert(&reference.id)
                || reference.label.contains('\0')
            {
                return Err(Error::Invalid(
                    "The notebook contains an invalid passage reference.".into(),
                ));
            }
            reference.locator.validate()?;
            if let Some(passage) = &reference.passage {
                passage.validate(&reference.locator)?;
            }
        }
        let mut section_ids = std::collections::HashSet::new();
        let mut previous_end = 0;
        for section in &self.sections {
            let mut connections = std::collections::HashSet::new();
            if section.id.is_empty()
                || !section_ids.insert(&section.id)
                || section.start < previous_end
                || section.end <= section.start
                || section.end as usize > length
                || section.passages.is_empty()
                || section
                    .passages
                    .iter()
                    .any(|id| !ids.contains(id) || !connections.insert(id))
            {
                return Err(Error::Invalid("The notebook contains invalid passage connections. It has been left unchanged.".into()));
            }
            previous_end = section.end;
        }
        Ok(())
    }

    pub fn upgrade(mut self) -> Result<Self> {
        self.validate()?;
        self.version = 2;
        Ok(self)
    }

    pub fn connect(&mut self, start: i32, end: i32, passage: &str) -> Result<()> {
        if start < 0
            || end <= start
            || end as usize > self.text.chars().count()
            || !self.references.iter().any(|r| r.id == passage)
        {
            return Err(Error::Invalid(
                "Select notebook text and a saved passage first.".into(),
            ));
        }
        if let Some(section) = self
            .sections
            .iter_mut()
            .find(|s| s.start == start && s.end == end)
        {
            if !section.passages.iter().any(|id| id == passage) {
                section.passages.push(passage.into());
            }
            return Ok(());
        }
        if self.sections.iter().any(|s| s.start < end && s.end > start) {
            return Err(Error::Invalid(
                "Select the existing linked section or separate text.".into(),
            ));
        }
        self.sections.push(Section {
            id: uuid::Uuid::new_v4().to_string(),
            start,
            end,
            passages: vec![passage.into()],
        });
        self.sections.sort_by_key(|s| s.start);
        Ok(())
    }

    pub fn disconnect(&mut self, section: &str, passage: &str) {
        if let Some(section) = self.sections.iter_mut().find(|s| s.id == section) {
            section.passages.retain(|id| id != passage);
        }
        self.sections.retain(|s| !s.passages.is_empty());
    }

    pub fn remove_passage(&mut self, id: &str) {
        self.references.retain(|r| r.id != id);
        for section in &mut self.sections {
            section.passages.retain(|p| p != id);
        }
        self.sections.retain(|s| !s.passages.is_empty());
    }

    pub fn section_at(&self, offset: i32) -> Option<&Section> {
        let index = self.sections.partition_point(|s| s.start <= offset);
        index
            .checked_sub(1)
            .and_then(|i| self.sections.get(i))
            .filter(|s| offset < s.end)
    }
}
