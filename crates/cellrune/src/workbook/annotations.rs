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

    /// Returns this hyperlink over other cells.
    pub(crate) fn with_range(&self, range: CellRange) -> Self {
        Self {
            range,
            ..self.clone()
        }
    }
}

/// One cell comment, read-only: a legacy note, or the first comment of an Excel 365 comment
/// thread, whose replies follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    address: CellAddress,
    author: Option<Box<str>>,
    text: Box<str>,
    replies: Vec<CommentReply>,
}

/// One reply in a comment thread, read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentReply {
    author: Option<Box<str>>,
    text: Box<str>,
}

impl CommentReply {
    pub(crate) const fn from_xlsx(author: Option<Box<str>>, text: Box<str>) -> Self {
        Self { author, text }
    }

    /// Returns the author's display name.
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    /// Returns the plain text.
    pub fn text(&self) -> &str {
        &self.text
    }
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
            replies: Vec::new(),
        }
    }

    pub(crate) fn with_replies(mut self, replies: Vec<CommentReply>) -> Self {
        self.replies = replies;
        self
    }

    /// Returns the replies of a comment thread in order; a legacy note has none.
    pub fn replies(&self) -> &[CommentReply] {
        &self.replies
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

    /// Returns this comment on another cell.
    pub(crate) fn moved_to(&self, address: CellAddress) -> Self {
        Self {
            address,
            ..self.clone()
        }
    }
}
