use crate::ViewSelection;
use glib::translate::UnsafeFrom;

impl ViewSelection {
    #[inline]
    pub fn style(&self) -> papers_document::SelectionStyle {
        unsafe { glib::translate::from_glib((*self.as_ptr()).style) }
    }
    #[inline]
    pub fn page(&self) -> i32 {
        unsafe { (*self.as_ptr()).page }
    }

    #[inline]
    pub fn rect(&self) -> papers_document::Rectangle {
        unsafe { papers_document::Rectangle::unsafe_from((*self.as_ptr()).rect) }
    }
}
