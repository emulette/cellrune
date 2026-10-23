use crate::{CellAddress, CellRange};

/// One worksheet hyperlink, read-only.
///
/// An external hyperlink names its target through a worksheet relationship; an internal one
/// names a place in this workbook through `location`, which a sheet rename rewrites as it does in
/// the saved part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    range: CellRange,
    external: bool,
    target: Option<Box<str>>,
    location: Option<Box<str>>,
    display: Option<Box<str>>,
    tooltip: Option<Box<str>>,
}

impl Hyperlink {
    /// `[display, tooltip]` are the recorded texts; `external` marks a relationship reference,
    /// whose target may not have resolved.
    pub(crate) fn from_xlsx(
        range: CellRange,
        external: bool,
        target: Option<Box<str>>,
        location: Option<Box<str>>,
        [display, tooltip]: [Option<Box<str>>; 2],
    ) -> Self {
        Self {
            range,
            external,
            target,
            location,
            display,
            tooltip,
        }
    }

    /// Returns the cells the hyperlink covers.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns the external target, such as a URL or file path, from the worksheet relationship.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Returns the place the hyperlink opens: a cell reference or defined name in this workbook,
    /// or, with a target, a place inside the target document.
    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }

    /// Returns the display text recorded with the hyperlink.
    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// Returns the screen tip.
    pub fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    /// Returns whether the hyperlink references a relationship; its location then points into
    /// the target document, so a sheet rename leaves it alone.
    pub(crate) const fn is_external(&self) -> bool {
        self.external
    }

    pub(crate) fn location_mut(&mut self) -> &mut Option<Box<str>> {
        &mut self.location
    }
}

/// One cell comment (a note), read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    address: CellAddress,
    author: Option<Box<str>>,
    text: Box<str>,
}

impl Comment {
    pub(crate) const fn from_xlsx(
        address: CellAddress,
        author: Option<Box<str>>,
        text: Box<str>,
    ) -> Self {
        Self {
            address,
            author,
            text,
        }
    }

    /// Returns the commented cell.
    pub const fn address(&self) -> CellAddress {
        self.address
    }

    /// Returns the author.
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    /// Returns the plain text of every run, without formatting or phonetic runs.
    pub fn text(&self) -> &str {
        &self.text
    }
}
