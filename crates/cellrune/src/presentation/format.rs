//! The minimal cell formatting model: font, fill, border, and alignment.

use crate::ValidationError;

/// Largest font size Excel accepts, in points.
const MAX_FONT_SIZE: f64 = 409.0;
/// Most UTF-16 code units Excel keeps in a font name.
const MAX_FONT_NAME_UTF16: usize = 31;
/// Largest indent Excel accepts.
const MAX_INDENT: u32 = 250;
/// Text rotation that stacks characters vertically.
pub(crate) const VERTICAL_TEXT_ROTATION: u32 = 255;

fn out_of_range(property: &'static str, value: impl ToString) -> ValidationError {
    ValidationError::FormatValueOutOfRange {
        property,
        value: value.to_string(),
    }
}

/// Where a [`Color`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorKind {
    /// An explicit color, `0xAARRGGBB` (`rgb`).
    Argb(u32),
    /// A workbook theme color index (`theme`).
    Theme(u32),
    /// A legacy palette index (`indexed`).
    Indexed(u32),
    /// The spreadsheet app's automatic color (`auto`).
    Automatic,
}

/// A spreadsheet color with an optional lightening or darkening tint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    kind: ColorKind,
    tint: f64,
}

// Every float in the format model is validated finite, so equality is reflexive.
impl Eq for Color {}
impl Eq for Font {}
impl Eq for PatternFill {}
impl Eq for Fill {}
impl Eq for BorderEdge {}
impl Eq for Border {}
impl Eq for CellFormat {}
impl Eq for CellFormatEdit {}

impl Color {
    /// An explicit color, `0xAARRGGBB`.
    pub const fn argb(value: u32) -> Self {
        Self::new(ColorKind::Argb(value))
    }

    /// A workbook theme color index.
    pub const fn theme(index: u32) -> Self {
        Self::new(ColorKind::Theme(index))
    }

    /// A legacy palette index.
    pub const fn indexed(index: u32) -> Self {
        Self::new(ColorKind::Indexed(index))
    }

    /// The spreadsheet app's automatic color.
    pub const fn automatic() -> Self {
        Self::new(ColorKind::Automatic)
    }

    const fn new(kind: ColorKind) -> Self {
        Self { kind, tint: 0.0 }
    }

    /// Returns this color with `tint`, from -1.0 (darkest) through 1.0 (lightest).
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for a tint outside -1.0 through 1.0.
    pub fn with_tint(self, tint: f64) -> Result<Self, ValidationError> {
        if !(-1.0..=1.0).contains(&tint) {
            return Err(out_of_range("color_tint", tint));
        }
        Ok(Self { tint, ..self })
    }

    /// Returns where the color comes from.
    pub const fn kind(&self) -> ColorKind {
        self.kind
    }

    /// Returns the tint, 0.0 when the color has none.
    pub const fn tint(&self) -> f64 {
        self.tint
    }
}

/// A font underline style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FontUnderline {
    /// No underline.
    #[default]
    None,
    /// A single underline (`single`).
    Single,
    /// A double underline (`double`).
    Double,
    /// A single accounting underline (`singleAccounting`).
    SingleAccounting,
    /// A double accounting underline (`doubleAccounting`).
    DoubleAccounting,
}

impl FontUnderline {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Single => "single",
            Self::Double => "double",
            Self::SingleAccounting => "singleAccounting",
            Self::DoubleAccounting => "doubleAccounting",
        }
    }

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "none" => Self::None,
            "single" => Self::Single,
            "double" => Self::Double,
            "singleAccounting" => Self::SingleAccounting,
            "doubleAccounting" => Self::DoubleAccounting,
            _ => return None,
        })
    }
}

/// A cell font.
///
/// Attributes the model does not carry, such as the font family, scheme, or character set, stay in
/// the source font record of a cell whose font is not edited.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Font {
    name: Option<Box<str>>,
    size: Option<f64>,
    bold: bool,
    italic: bool,
    underline: FontUnderline,
    strike: bool,
    color: Option<Color>,
}

impl Font {
    /// A font with no name, size, or color and every flag off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this font with a typeface name.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FontNameInvalid`] for an empty name, one longer than 31 UTF-16
    /// code units, or one with a control character.
    pub fn with_name(self, name: &str) -> Result<Self, ValidationError> {
        if name.is_empty()
            || name.encode_utf16().count() > MAX_FONT_NAME_UTF16
            || name.chars().any(char::is_control)
        {
            return Err(ValidationError::FontNameInvalid);
        }
        Ok(Self {
            name: Some(Box::from(name)),
            ..self
        })
    }

    /// Returns this font with a size in points, from 1 through 409.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for a size outside 1 through 409.
    pub fn with_size(self, size: f64) -> Result<Self, ValidationError> {
        if !(1.0..=MAX_FONT_SIZE).contains(&size) {
            return Err(out_of_range("font_size", size));
        }
        Ok(Self {
            size: Some(size),
            ..self
        })
    }

    /// Returns this font bold or not.
    #[must_use]
    pub fn with_bold(self, bold: bool) -> Self {
        Self { bold, ..self }
    }

    /// Returns this font italic or not.
    #[must_use]
    pub fn with_italic(self, italic: bool) -> Self {
        Self { italic, ..self }
    }

    /// Returns this font with an underline style.
    #[must_use]
    pub fn with_underline(self, underline: FontUnderline) -> Self {
        Self { underline, ..self }
    }

    /// Returns this font struck through or not.
    #[must_use]
    pub fn with_strike(self, strike: bool) -> Self {
        Self { strike, ..self }
    }

    /// Returns this font with a color, or without one.
    #[must_use]
    pub fn with_color(self, color: Option<Color>) -> Self {
        Self { color, ..self }
    }

    /// Returns the typeface name.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the size in points.
    pub const fn size(&self) -> Option<f64> {
        self.size
    }

    /// Returns whether the font is bold.
    pub const fn bold(&self) -> bool {
        self.bold
    }

    /// Returns whether the font is italic.
    pub const fn italic(&self) -> bool {
        self.italic
    }

    /// Returns the underline style.
    pub const fn underline(&self) -> FontUnderline {
        self.underline
    }

    /// Returns whether the font is struck through.
    pub const fn strike(&self) -> bool {
        self.strike
    }

    /// Returns the font color.
    pub const fn color(&self) -> Option<Color> {
        self.color
    }
}

/// A fill pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FillPattern {
    /// No fill (`none`).
    #[default]
    None,
    /// A solid fill in the foreground color (`solid`).
    Solid,
    /// `mediumGray`.
    MediumGray,
    /// `darkGray`.
    DarkGray,
    /// `lightGray`.
    LightGray,
    /// `darkHorizontal`.
    DarkHorizontal,
    /// `darkVertical`.
    DarkVertical,
    /// `darkDown`.
    DarkDown,
    /// `darkUp`.
    DarkUp,
    /// `darkGrid`.
    DarkGrid,
    /// `darkTrellis`.
    DarkTrellis,
    /// `lightHorizontal`.
    LightHorizontal,
    /// `lightVertical`.
    LightVertical,
    /// `lightDown`.
    LightDown,
    /// `lightUp`.
    LightUp,
    /// `lightGrid`.
    LightGrid,
    /// `lightTrellis`.
    LightTrellis,
    /// `gray125`.
    Gray125,
    /// `gray0625`.
    Gray0625,
}

impl FillPattern {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Solid => "solid",
            Self::MediumGray => "mediumGray",
            Self::DarkGray => "darkGray",
            Self::LightGray => "lightGray",
            Self::DarkHorizontal => "darkHorizontal",
            Self::DarkVertical => "darkVertical",
            Self::DarkDown => "darkDown",
            Self::DarkUp => "darkUp",
            Self::DarkGrid => "darkGrid",
            Self::DarkTrellis => "darkTrellis",
            Self::LightHorizontal => "lightHorizontal",
            Self::LightVertical => "lightVertical",
            Self::LightDown => "lightDown",
            Self::LightUp => "lightUp",
            Self::LightGrid => "lightGrid",
            Self::LightTrellis => "lightTrellis",
            Self::Gray125 => "gray125",
            Self::Gray0625 => "gray0625",
        }
    }

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "none" => Self::None,
            "solid" => Self::Solid,
            "mediumGray" => Self::MediumGray,
            "darkGray" => Self::DarkGray,
            "lightGray" => Self::LightGray,
            "darkHorizontal" => Self::DarkHorizontal,
            "darkVertical" => Self::DarkVertical,
            "darkDown" => Self::DarkDown,
            "darkUp" => Self::DarkUp,
            "darkGrid" => Self::DarkGrid,
            "darkTrellis" => Self::DarkTrellis,
            "lightHorizontal" => Self::LightHorizontal,
            "lightVertical" => Self::LightVertical,
            "lightDown" => Self::LightDown,
            "lightUp" => Self::LightUp,
            "lightGrid" => Self::LightGrid,
            "lightTrellis" => Self::LightTrellis,
            "gray125" => Self::Gray125,
            "gray0625" => Self::Gray0625,
            _ => return None,
        })
    }
}

/// A pattern fill: a pattern drawn in the foreground color over the background color.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PatternFill {
    pattern: FillPattern,
    foreground: Option<Color>,
    background: Option<Color>,
}

impl PatternFill {
    /// A fill with `pattern` and no colors.
    pub const fn new(pattern: FillPattern) -> Self {
        Self {
            pattern,
            foreground: None,
            background: None,
        }
    }

    /// A solid fill in `color`.
    pub const fn solid(color: Color) -> Self {
        Self {
            pattern: FillPattern::Solid,
            foreground: Some(color),
            background: None,
        }
    }

    /// Returns this fill with a foreground color, or without one.
    #[must_use]
    pub const fn with_foreground(self, foreground: Option<Color>) -> Self {
        Self { foreground, ..self }
    }

    /// Returns this fill with a background color, or without one.
    #[must_use]
    pub const fn with_background(self, background: Option<Color>) -> Self {
        Self { background, ..self }
    }

    /// Returns the pattern.
    pub const fn pattern(&self) -> FillPattern {
        self.pattern
    }

    /// Returns the pattern color; a solid fill is drawn in it.
    pub const fn foreground(&self) -> Option<Color> {
        self.foreground
    }

    /// Returns the color behind the pattern.
    pub const fn background(&self) -> Option<Color> {
        self.background
    }
}

/// A cell fill as read.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Fill {
    /// A pattern fill, including no fill.
    Pattern(PatternFill),
    /// A gradient fill; its stops stay in the source fill record and are not modeled.
    Gradient,
}

impl Default for Fill {
    fn default() -> Self {
        Self::Pattern(PatternFill::default())
    }
}

/// A border line style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum BorderStyle {
    /// No line (`none`).
    #[default]
    None,
    /// `thin`.
    Thin,
    /// `medium`.
    Medium,
    /// `dashed`.
    Dashed,
    /// `dotted`.
    Dotted,
    /// `thick`.
    Thick,
    /// `double`.
    Double,
    /// `hair`.
    Hair,
    /// `mediumDashed`.
    MediumDashed,
    /// `dashDot`.
    DashDot,
    /// `mediumDashDot`.
    MediumDashDot,
    /// `dashDotDot`.
    DashDotDot,
    /// `mediumDashDotDot`.
    MediumDashDotDot,
    /// `slantDashDot`.
    SlantDashDot,
}

impl BorderStyle {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Thin => "thin",
            Self::Medium => "medium",
            Self::Dashed => "dashed",
            Self::Dotted => "dotted",
            Self::Thick => "thick",
            Self::Double => "double",
            Self::Hair => "hair",
            Self::MediumDashed => "mediumDashed",
            Self::DashDot => "dashDot",
            Self::MediumDashDot => "mediumDashDot",
            Self::DashDotDot => "dashDotDot",
            Self::MediumDashDotDot => "mediumDashDotDot",
            Self::SlantDashDot => "slantDashDot",
        }
    }

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "none" => Self::None,
            "thin" => Self::Thin,
            "medium" => Self::Medium,
            "dashed" => Self::Dashed,
            "dotted" => Self::Dotted,
            "thick" => Self::Thick,
            "double" => Self::Double,
            "hair" => Self::Hair,
            "mediumDashed" => Self::MediumDashed,
            "dashDot" => Self::DashDot,
            "mediumDashDot" => Self::MediumDashDot,
            "dashDotDot" => Self::DashDotDot,
            "mediumDashDotDot" => Self::MediumDashDotDot,
            "slantDashDot" => Self::SlantDashDot,
            _ => return None,
        })
    }
}

/// One border line: its style and color.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderEdge {
    style: BorderStyle,
    color: Option<Color>,
}

impl BorderEdge {
    /// A line in `style` and `color`.
    pub const fn new(style: BorderStyle, color: Option<Color>) -> Self {
        Self { style, color }
    }

    /// Returns the line style.
    pub const fn style(&self) -> BorderStyle {
        self.style
    }

    /// Returns the line color.
    pub const fn color(&self) -> Option<Color> {
        self.color
    }
}

/// A cell border.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Border {
    left: BorderEdge,
    right: BorderEdge,
    top: BorderEdge,
    bottom: BorderEdge,
    diagonal: BorderEdge,
    diagonal_up: bool,
    diagonal_down: bool,
}

impl Border {
    /// A border with no lines.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this border with every outside line set to `edge`.
    #[must_use]
    pub const fn with_outline(self, edge: BorderEdge) -> Self {
        Self {
            left: edge,
            right: edge,
            top: edge,
            bottom: edge,
            ..self
        }
    }

    /// Returns this border with a left line.
    #[must_use]
    pub const fn with_left(self, left: BorderEdge) -> Self {
        Self { left, ..self }
    }

    /// Returns this border with a right line.
    #[must_use]
    pub const fn with_right(self, right: BorderEdge) -> Self {
        Self { right, ..self }
    }

    /// Returns this border with a top line.
    #[must_use]
    pub const fn with_top(self, top: BorderEdge) -> Self {
        Self { top, ..self }
    }

    /// Returns this border with a bottom line.
    #[must_use]
    pub const fn with_bottom(self, bottom: BorderEdge) -> Self {
        Self { bottom, ..self }
    }

    /// Returns this border with a diagonal line drawn up, down, or both.
    #[must_use]
    pub const fn with_diagonal(self, diagonal: BorderEdge, up: bool, down: bool) -> Self {
        Self {
            diagonal,
            diagonal_up: up,
            diagonal_down: down,
            ..self
        }
    }

    /// Returns the left line.
    pub const fn left(&self) -> BorderEdge {
        self.left
    }

    /// Returns the right line.
    pub const fn right(&self) -> BorderEdge {
        self.right
    }

    /// Returns the top line.
    pub const fn top(&self) -> BorderEdge {
        self.top
    }

    /// Returns the bottom line.
    pub const fn bottom(&self) -> BorderEdge {
        self.bottom
    }

    /// Returns the diagonal line.
    pub const fn diagonal(&self) -> BorderEdge {
        self.diagonal
    }

    /// Returns whether the diagonal runs from bottom left to top right.
    pub const fn diagonal_up(&self) -> bool {
        self.diagonal_up
    }

    /// Returns whether the diagonal runs from top left to bottom right.
    pub const fn diagonal_down(&self) -> bool {
        self.diagonal_down
    }
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum HorizontalAlignment {
    /// Text left, numbers right (`general`).
    #[default]
    General,
    /// `left`.
    Left,
    /// `center`.
    Center,
    /// `right`.
    Right,
    /// Repeat the content across the cell (`fill`).
    Fill,
    /// `justify`.
    Justify,
    /// Center across the following empty cells (`centerContinuous`).
    CenterContinuous,
    /// `distributed`.
    Distributed,
}

impl HorizontalAlignment {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Fill => "fill",
            Self::Justify => "justify",
            Self::CenterContinuous => "centerContinuous",
            Self::Distributed => "distributed",
        }
    }

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "general" => Self::General,
            "left" => Self::Left,
            "center" => Self::Center,
            "right" => Self::Right,
            "fill" => Self::Fill,
            "justify" => Self::Justify,
            "centerContinuous" => Self::CenterContinuous,
            "distributed" => Self::Distributed,
            _ => return None,
        })
    }
}

/// Vertical text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum VerticalAlignment {
    /// `top`.
    Top,
    /// `center`.
    Center,
    /// `bottom`, the default.
    #[default]
    Bottom,
    /// `justify`.
    Justify,
    /// `distributed`.
    Distributed,
}

impl VerticalAlignment {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Center => "center",
            Self::Bottom => "bottom",
            Self::Justify => "justify",
            Self::Distributed => "distributed",
        }
    }

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "top" => Self::Top,
            "center" => Self::Center,
            "bottom" => Self::Bottom,
            "justify" => Self::Justify,
            "distributed" => Self::Distributed,
            _ => return None,
        })
    }
}

/// Cell text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Alignment {
    horizontal: HorizontalAlignment,
    vertical: VerticalAlignment,
    wrap_text: bool,
    shrink_to_fit: bool,
    indent: u32,
    text_rotation: u32,
}

impl Alignment {
    /// General horizontal and bottom vertical alignment with no wrapping, indent, or rotation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this alignment with a horizontal alignment.
    #[must_use]
    pub const fn with_horizontal(self, horizontal: HorizontalAlignment) -> Self {
        Self { horizontal, ..self }
    }

    /// Returns this alignment with a vertical alignment.
    #[must_use]
    pub const fn with_vertical(self, vertical: VerticalAlignment) -> Self {
        Self { vertical, ..self }
    }

    /// Returns this alignment wrapping text or not.
    #[must_use]
    pub const fn with_wrap_text(self, wrap_text: bool) -> Self {
        Self { wrap_text, ..self }
    }

    /// Returns this alignment shrinking text to fit or not.
    #[must_use]
    pub const fn with_shrink_to_fit(self, shrink_to_fit: bool) -> Self {
        Self {
            shrink_to_fit,
            ..self
        }
    }

    /// Returns this alignment with an indent level from 0 through 250.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for an indent above 250.
    pub fn with_indent(self, indent: u32) -> Result<Self, ValidationError> {
        if indent > MAX_INDENT {
            return Err(out_of_range("indent", indent));
        }
        Ok(Self { indent, ..self })
    }

    /// Returns this alignment with a text rotation: 0 through 90 degrees counterclockwise, 91
    /// through 180 for 1 through 90 degrees clockwise, or 255 for vertically stacked text.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::FormatValueOutOfRange`] for any other value.
    pub fn with_text_rotation(self, text_rotation: u32) -> Result<Self, ValidationError> {
        if text_rotation > 180 && text_rotation != VERTICAL_TEXT_ROTATION {
            return Err(out_of_range("text_rotation", text_rotation));
        }
        Ok(Self {
            text_rotation,
            ..self
        })
    }

    /// Returns the horizontal alignment.
    pub const fn horizontal(&self) -> HorizontalAlignment {
        self.horizontal
    }

    /// Returns the vertical alignment.
    pub const fn vertical(&self) -> VerticalAlignment {
        self.vertical
    }

    /// Returns whether text wraps.
    pub const fn wrap_text(&self) -> bool {
        self.wrap_text
    }

    /// Returns whether text shrinks to fit.
    pub const fn shrink_to_fit(&self) -> bool {
        self.shrink_to_fit
    }

    /// Returns the indent level.
    pub const fn indent(&self) -> u32 {
        self.indent
    }

    /// Returns the text rotation.
    pub const fn text_rotation(&self) -> u32 {
        self.text_rotation
    }
}

/// The font, fill, border, and alignment a cell displays with.
///
/// The number format stays on [`crate::Cell::number_format`] because calculation reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellFormat {
    font: Font,
    fill: Fill,
    border: Border,
    alignment: Alignment,
}

impl CellFormat {
    pub(crate) const fn from_parts(
        font: Font,
        fill: Fill,
        border: Border,
        alignment: Alignment,
    ) -> Self {
        Self {
            font,
            fill,
            border,
            alignment,
        }
    }

    /// Returns the font.
    pub const fn font(&self) -> &Font {
        &self.font
    }

    /// Returns the fill.
    pub const fn fill(&self) -> Fill {
        self.fill
    }

    /// Returns the border.
    pub const fn border(&self) -> Border {
        self.border
    }

    /// Returns the alignment.
    pub const fn alignment(&self) -> Alignment {
        self.alignment
    }

    /// Returns this format with the components `edit` sets replaced.
    pub(crate) fn edited(&self, edit: &CellFormatEdit) -> Self {
        Self {
            font: edit.font.clone().unwrap_or_else(|| self.font.clone()),
            fill: edit.fill.map_or(self.fill, Fill::Pattern),
            border: edit.border.unwrap_or(self.border),
            alignment: edit.alignment.unwrap_or(self.alignment),
        }
    }
}

/// The format components one edit replaces; components it leaves unset keep their current value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellFormatEdit {
    font: Option<Font>,
    fill: Option<PatternFill>,
    border: Option<Border>,
    alignment: Option<Alignment>,
}

impl CellFormatEdit {
    /// An edit that replaces nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this edit replacing the font.
    #[must_use]
    pub fn with_font(self, font: Font) -> Self {
        Self {
            font: Some(font),
            ..self
        }
    }

    /// Returns this edit replacing the fill.
    #[must_use]
    pub fn with_fill(self, fill: PatternFill) -> Self {
        Self {
            fill: Some(fill),
            ..self
        }
    }

    /// Returns this edit replacing the border.
    #[must_use]
    pub fn with_border(self, border: Border) -> Self {
        Self {
            border: Some(border),
            ..self
        }
    }

    /// Returns this edit replacing the alignment.
    #[must_use]
    pub fn with_alignment(self, alignment: Alignment) -> Self {
        Self {
            alignment: Some(alignment),
            ..self
        }
    }

    /// Returns the replacement font.
    pub const fn font(&self) -> Option<&Font> {
        self.font.as_ref()
    }

    /// Returns the replacement fill.
    pub const fn fill(&self) -> Option<PatternFill> {
        self.fill
    }

    /// Returns the replacement border.
    pub const fn border(&self) -> Option<Border> {
        self.border
    }

    /// Returns the replacement alignment.
    pub const fn alignment(&self) -> Option<Alignment> {
        self.alignment
    }

    /// Returns whether the edit replaces nothing.
    pub const fn is_empty(&self) -> bool {
        self.font.is_none()
            && self.fill.is_none()
            && self.border.is_none()
            && self.alignment.is_none()
    }
}

/// Validates a column width in characters, from 0 through 255.
pub(crate) fn validate_column_width(width: f64) -> Result<f64, ValidationError> {
    if !(0.0..=255.0).contains(&width) {
        return Err(out_of_range("column_width", width));
    }
    Ok(width)
}

/// Validates a row height in points, from 0 through 409.
pub(crate) fn validate_row_height(height: f64) -> Result<f64, ValidationError> {
    if !(0.0..=MAX_FONT_SIZE).contains(&height) {
        return Err(out_of_range("row_height", height));
    }
    Ok(height)
}
