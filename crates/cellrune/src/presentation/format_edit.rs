//! Format edits: the font, fill, border, and alignment properties one edit sets.
//!
//! An edit merges into the format the cell displays with when the edit applies. A font, border,
//! or alignment edit sets only the properties it names and keeps every other property of the
//! cell's current font, border, or alignment; one built from a complete [`Font`], [`Border`], or
//! [`Alignment`] names every property and so replaces it. A fill is one pattern drawn in two
//! colors, so a fill edit replaces the whole fill.

use super::{
    Alignment, Border, BorderEdge, CellFormat, Color, Fill, Font, FontUnderline,
    HorizontalAlignment, PatternFill, VerticalAlignment,
};
use crate::ValidationError;

/// The font properties one edit sets; the others keep the cell's current value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontEdit {
    name: Option<Option<Box<str>>>,
    size: Option<Option<f64>>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<FontUnderline>,
    strike: Option<bool>,
    color: Option<Option<Color>>,
}

// Every float in an edit is validated finite, so equality is reflexive.
impl Eq for FontEdit {}
impl Eq for BorderEdit {}
impl Eq for CellFormatEdit {}

impl FontEdit {
    /// An edit that sets no font property.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this edit setting the typeface name, or removing it with `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FontNameInvalid`] for a name [`Font::with_name`] rejects.
    pub fn with_name(self, name: Option<&str>) -> Result<Self, ValidationError> {
        let name = name
            .map(|name| Font::new().with_name(name).map(|font| font.name))
            .transpose()?
            .flatten();
        Ok(Self {
            name: Some(name),
            ..self
        })
    }

    /// Returns this edit setting the size in points, or removing it with `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for a size outside 1 through 409.
    pub fn with_size(self, size: Option<f64>) -> Result<Self, ValidationError> {
        let size = size
            .map(|size| Font::new().with_size(size).map(|font| font.size))
            .transpose()?
            .flatten();
        Ok(Self {
            size: Some(size),
            ..self
        })
    }

    /// Returns this edit setting bold.
    #[must_use]
    pub fn with_bold(self, bold: bool) -> Self {
        Self {
            bold: Some(bold),
            ..self
        }
    }

    /// Returns this edit setting italic.
    #[must_use]
    pub fn with_italic(self, italic: bool) -> Self {
        Self {
            italic: Some(italic),
            ..self
        }
    }

    /// Returns this edit setting the underline style.
    #[must_use]
    pub fn with_underline(self, underline: FontUnderline) -> Self {
        Self {
            underline: Some(underline),
            ..self
        }
    }

    /// Returns this edit setting strikethrough.
    #[must_use]
    pub fn with_strike(self, strike: bool) -> Self {
        Self {
            strike: Some(strike),
            ..self
        }
    }

    /// Returns this edit setting the font color, or removing it with `None`.
    #[must_use]
    pub fn with_color(self, color: Option<Color>) -> Self {
        Self {
            color: Some(color),
            ..self
        }
    }

    /// Returns the typeface name the edit sets: `Some(None)` removes the name.
    pub fn name(&self) -> Option<Option<&str>> {
        self.name.as_ref().map(Option::as_deref)
    }

    /// Returns the size the edit sets: `Some(None)` removes the size.
    pub const fn size(&self) -> Option<Option<f64>> {
        self.size
    }

    /// Returns the bold flag the edit sets.
    pub const fn bold(&self) -> Option<bool> {
        self.bold
    }

    /// Returns the italic flag the edit sets.
    pub const fn italic(&self) -> Option<bool> {
        self.italic
    }

    /// Returns the underline style the edit sets.
    pub const fn underline(&self) -> Option<FontUnderline> {
        self.underline
    }

    /// Returns the strikethrough flag the edit sets.
    pub const fn strike(&self) -> Option<bool> {
        self.strike
    }

    /// Returns the color the edit sets: `Some(None)` removes the color.
    pub const fn color(&self) -> Option<Option<Color>> {
        self.color
    }

    fn apply(&self, font: &Font) -> Font {
        Font {
            name: self.name.clone().unwrap_or_else(|| font.name.clone()),
            size: self.size.unwrap_or(font.size),
            bold: self.bold.unwrap_or(font.bold),
            italic: self.italic.unwrap_or(font.italic),
            underline: self.underline.unwrap_or(font.underline),
            strike: self.strike.unwrap_or(font.strike),
            color: self.color.unwrap_or(font.color),
        }
    }
}

/// Sets every property, so the edit replaces the font.
impl From<Font> for FontEdit {
    fn from(font: Font) -> Self {
        Self {
            name: Some(font.name),
            size: Some(font.size),
            bold: Some(font.bold),
            italic: Some(font.italic),
            underline: Some(font.underline),
            strike: Some(font.strike),
            color: Some(font.color),
        }
    }
}

/// The border lines and diagonal directions one edit sets; the others keep the cell's current
/// value. Each line the edit sets replaces that line's style and color together.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderEdit {
    left: Option<BorderEdge>,
    right: Option<BorderEdge>,
    top: Option<BorderEdge>,
    bottom: Option<BorderEdge>,
    diagonal: Option<BorderEdge>,
    diagonal_up: Option<bool>,
    diagonal_down: Option<bool>,
}

impl BorderEdit {
    /// An edit that sets no border line.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this edit setting every outside line to `edge`.
    #[must_use]
    pub const fn with_outline(self, edge: BorderEdge) -> Self {
        Self {
            left: Some(edge),
            right: Some(edge),
            top: Some(edge),
            bottom: Some(edge),
            ..self
        }
    }

    /// Returns this edit setting the left line.
    #[must_use]
    pub const fn with_left(self, left: BorderEdge) -> Self {
        Self {
            left: Some(left),
            ..self
        }
    }

    /// Returns this edit setting the right line.
    #[must_use]
    pub const fn with_right(self, right: BorderEdge) -> Self {
        Self {
            right: Some(right),
            ..self
        }
    }

    /// Returns this edit setting the top line.
    #[must_use]
    pub const fn with_top(self, top: BorderEdge) -> Self {
        Self {
            top: Some(top),
            ..self
        }
    }

    /// Returns this edit setting the bottom line.
    #[must_use]
    pub const fn with_bottom(self, bottom: BorderEdge) -> Self {
        Self {
            bottom: Some(bottom),
            ..self
        }
    }

    /// Returns this edit setting the diagonal line.
    #[must_use]
    pub const fn with_diagonal(self, diagonal: BorderEdge) -> Self {
        Self {
            diagonal: Some(diagonal),
            ..self
        }
    }

    /// Returns this edit setting whether the diagonal runs from bottom left to top right.
    #[must_use]
    pub const fn with_diagonal_up(self, up: bool) -> Self {
        Self {
            diagonal_up: Some(up),
            ..self
        }
    }

    /// Returns this edit setting whether the diagonal runs from top left to bottom right.
    #[must_use]
    pub const fn with_diagonal_down(self, down: bool) -> Self {
        Self {
            diagonal_down: Some(down),
            ..self
        }
    }

    /// Returns the left line the edit sets.
    pub const fn left(&self) -> Option<BorderEdge> {
        self.left
    }

    /// Returns the right line the edit sets.
    pub const fn right(&self) -> Option<BorderEdge> {
        self.right
    }

    /// Returns the top line the edit sets.
    pub const fn top(&self) -> Option<BorderEdge> {
        self.top
    }

    /// Returns the bottom line the edit sets.
    pub const fn bottom(&self) -> Option<BorderEdge> {
        self.bottom
    }

    /// Returns the diagonal line the edit sets.
    pub const fn diagonal(&self) -> Option<BorderEdge> {
        self.diagonal
    }

    /// Returns the bottom-left-to-top-right flag the edit sets.
    pub const fn diagonal_up(&self) -> Option<bool> {
        self.diagonal_up
    }

    /// Returns the top-left-to-bottom-right flag the edit sets.
    pub const fn diagonal_down(&self) -> Option<bool> {
        self.diagonal_down
    }

    fn apply(&self, border: &Border) -> Border {
        Border {
            left: self.left.unwrap_or(border.left),
            right: self.right.unwrap_or(border.right),
            top: self.top.unwrap_or(border.top),
            bottom: self.bottom.unwrap_or(border.bottom),
            diagonal: self.diagonal.unwrap_or(border.diagonal),
            diagonal_up: self.diagonal_up.unwrap_or(border.diagonal_up),
            diagonal_down: self.diagonal_down.unwrap_or(border.diagonal_down),
        }
    }
}

/// Sets every line and direction, so the edit replaces the border.
impl From<Border> for BorderEdit {
    fn from(border: Border) -> Self {
        Self {
            left: Some(border.left),
            right: Some(border.right),
            top: Some(border.top),
            bottom: Some(border.bottom),
            diagonal: Some(border.diagonal),
            diagonal_up: Some(border.diagonal_up),
            diagonal_down: Some(border.diagonal_down),
        }
    }
}

/// The alignment properties one edit sets; the others keep the cell's current value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AlignmentEdit {
    horizontal: Option<HorizontalAlignment>,
    vertical: Option<VerticalAlignment>,
    wrap_text: Option<bool>,
    shrink_to_fit: Option<bool>,
    indent: Option<u32>,
    text_rotation: Option<u32>,
}

impl AlignmentEdit {
    /// An edit that sets no alignment property.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this edit setting the horizontal alignment.
    #[must_use]
    pub const fn with_horizontal(self, horizontal: HorizontalAlignment) -> Self {
        Self {
            horizontal: Some(horizontal),
            ..self
        }
    }

    /// Returns this edit setting the vertical alignment.
    #[must_use]
    pub const fn with_vertical(self, vertical: VerticalAlignment) -> Self {
        Self {
            vertical: Some(vertical),
            ..self
        }
    }

    /// Returns this edit setting text wrapping.
    #[must_use]
    pub const fn with_wrap_text(self, wrap_text: bool) -> Self {
        Self {
            wrap_text: Some(wrap_text),
            ..self
        }
    }

    /// Returns this edit setting shrink-to-fit.
    #[must_use]
    pub const fn with_shrink_to_fit(self, shrink_to_fit: bool) -> Self {
        Self {
            shrink_to_fit: Some(shrink_to_fit),
            ..self
        }
    }

    /// Returns this edit setting the indent level.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for an indent
    /// [`Alignment::with_indent`] rejects.
    pub fn with_indent(self, indent: u32) -> Result<Self, ValidationError> {
        Alignment::new().with_indent(indent)?;
        Ok(Self {
            indent: Some(indent),
            ..self
        })
    }

    /// Returns this edit setting the text rotation.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for a rotation
    /// [`Alignment::with_text_rotation`] rejects.
    pub fn with_text_rotation(self, text_rotation: u32) -> Result<Self, ValidationError> {
        Alignment::new().with_text_rotation(text_rotation)?;
        Ok(Self {
            text_rotation: Some(text_rotation),
            ..self
        })
    }

    /// Returns the horizontal alignment the edit sets.
    pub const fn horizontal(&self) -> Option<HorizontalAlignment> {
        self.horizontal
    }

    /// Returns the vertical alignment the edit sets.
    pub const fn vertical(&self) -> Option<VerticalAlignment> {
        self.vertical
    }

    /// Returns the wrapping flag the edit sets.
    pub const fn wrap_text(&self) -> Option<bool> {
        self.wrap_text
    }

    /// Returns the shrink-to-fit flag the edit sets.
    pub const fn shrink_to_fit(&self) -> Option<bool> {
        self.shrink_to_fit
    }

    /// Returns the indent level the edit sets.
    pub const fn indent(&self) -> Option<u32> {
        self.indent
    }

    /// Returns the text rotation the edit sets.
    pub const fn text_rotation(&self) -> Option<u32> {
        self.text_rotation
    }

    fn apply(&self, alignment: &Alignment) -> Alignment {
        Alignment {
            horizontal: self.horizontal.unwrap_or(alignment.horizontal),
            vertical: self.vertical.unwrap_or(alignment.vertical),
            wrap_text: self.wrap_text.unwrap_or(alignment.wrap_text),
            shrink_to_fit: self.shrink_to_fit.unwrap_or(alignment.shrink_to_fit),
            indent: self.indent.unwrap_or(alignment.indent),
            text_rotation: self.text_rotation.unwrap_or(alignment.text_rotation),
        }
    }
}

/// Sets every property, so the edit replaces the alignment.
impl From<Alignment> for AlignmentEdit {
    fn from(alignment: Alignment) -> Self {
        Self {
            horizontal: Some(alignment.horizontal),
            vertical: Some(alignment.vertical),
            wrap_text: Some(alignment.wrap_text),
            shrink_to_fit: Some(alignment.shrink_to_fit),
            indent: Some(alignment.indent),
            text_rotation: Some(alignment.text_rotation),
        }
    }
}

/// The font, fill, border, and alignment properties one edit sets, merged into the cell's
/// current format as the [module documentation](self) describes; components the edit leaves unset
/// keep their current value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellFormatEdit {
    font: Option<FontEdit>,
    fill: Option<PatternFill>,
    border: Option<BorderEdit>,
    alignment: Option<AlignmentEdit>,
}

impl CellFormatEdit {
    /// An edit that changes nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this edit setting font properties: a [`FontEdit`] sets the properties it names,
    /// and a [`Font`] replaces the whole font. It replaces any font edit set before.
    #[must_use]
    pub fn with_font(self, font: impl Into<FontEdit>) -> Self {
        Self {
            font: Some(font.into()),
            ..self
        }
    }

    /// Returns this edit replacing the fill, pattern and colors together.
    #[must_use]
    pub fn with_fill(self, fill: PatternFill) -> Self {
        Self {
            fill: Some(fill),
            ..self
        }
    }

    /// Returns this edit setting border lines: a [`BorderEdit`] sets the lines it names, and a
    /// [`Border`] replaces the whole border. It replaces any border edit set before.
    #[must_use]
    pub fn with_border(self, border: impl Into<BorderEdit>) -> Self {
        Self {
            border: Some(border.into()),
            ..self
        }
    }

    /// Returns this edit setting alignment properties: an [`AlignmentEdit`] sets the properties
    /// it names, and an [`Alignment`] replaces the whole alignment. It replaces any alignment edit
    /// set before.
    #[must_use]
    pub fn with_alignment(self, alignment: impl Into<AlignmentEdit>) -> Self {
        Self {
            alignment: Some(alignment.into()),
            ..self
        }
    }

    /// Returns the font properties the edit sets.
    pub const fn font(&self) -> Option<&FontEdit> {
        self.font.as_ref()
    }

    /// Returns the replacement fill.
    pub const fn fill(&self) -> Option<PatternFill> {
        self.fill
    }

    /// Returns the border lines the edit sets.
    pub const fn border(&self) -> Option<BorderEdit> {
        self.border
    }

    /// Returns the alignment properties the edit sets.
    pub const fn alignment(&self) -> Option<AlignmentEdit> {
        self.alignment
    }

    /// Returns whether the edit changes nothing.
    pub const fn is_empty(&self) -> bool {
        self.font.is_none()
            && self.fill.is_none()
            && self.border.is_none()
            && self.alignment.is_none()
    }

    /// Returns `format` with this edit merged into it.
    pub(crate) fn apply(&self, format: &CellFormat) -> CellFormat {
        CellFormat::from_parts(
            self.font
                .as_ref()
                .map_or_else(|| format.font().clone(), |edit| edit.apply(format.font())),
            self.fill.map_or(format.fill(), Fill::Pattern),
            self.border
                .map_or(format.border(), |edit| edit.apply(&format.border())),
            self.alignment
                .map_or(format.alignment(), |edit| edit.apply(&format.alignment())),
        )
    }
}
