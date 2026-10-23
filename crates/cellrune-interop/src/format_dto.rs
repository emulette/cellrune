//! Cell formatting, column width, and row height DTOs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A spreadsheet color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColorDto {
    /// An explicit color.
    Argb {
        /// Eight hexadecimal digits, alpha first, such as `FFFF0000` for opaque red.
        argb: String,
        /// Lightening (positive) or darkening (negative) tint from -1.0 through 1.0.
        #[serde(default)]
        tint: f64,
    },
    /// A workbook theme color.
    Theme {
        /// Theme color index.
        index: u32,
        /// Lightening (positive) or darkening (negative) tint from -1.0 through 1.0.
        #[serde(default)]
        tint: f64,
    },
    /// A legacy palette color.
    Indexed {
        /// Palette index.
        index: u32,
        /// Lightening (positive) or darkening (negative) tint from -1.0 through 1.0.
        #[serde(default)]
        tint: f64,
    },
    /// The spreadsheet app's automatic color.
    Automatic {
        /// Lightening (positive) or darkening (negative) tint from -1.0 through 1.0.
        #[serde(default)]
        tint: f64,
    },
}

/// A cell font.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FontDto {
    /// Typeface name, 1 through 31 UTF-16 code units.
    #[serde(default)]
    pub name: Option<String>,
    /// Size in points from 1 through 409.
    #[serde(default)]
    pub size: Option<f64>,
    /// Bold.
    #[serde(default)]
    pub bold: bool,
    /// Italic.
    #[serde(default)]
    pub italic: bool,
    /// `none`, `single`, `double`, `singleAccounting`, or `doubleAccounting`.
    #[serde(default = "no_underline")]
    pub underline: String,
    /// Struck through.
    #[serde(default)]
    pub strike: bool,
    /// Font color.
    #[serde(default)]
    pub color: Option<ColorDto>,
}

fn no_underline() -> String {
    "none".to_owned()
}

/// A pattern fill: a pattern in the foreground color over the background color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatternFillDto {
    /// OOXML pattern token: `none`, `solid`, `gray125`, `darkGrid`, and the others of
    /// `ST_PatternType`.
    pub pattern: String,
    /// Pattern color; a solid fill is drawn in it.
    #[serde(default)]
    pub foreground: Option<ColorDto>,
    /// Color behind the pattern.
    #[serde(default)]
    pub background: Option<ColorDto>,
}

/// A cell fill as read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FillDto {
    /// A pattern fill, including no fill.
    Pattern {
        /// OOXML pattern token.
        pattern: String,
        /// Pattern color.
        foreground: Option<ColorDto>,
        /// Color behind the pattern.
        background: Option<ColorDto>,
    },
    /// A gradient fill, whose stops are not modeled.
    Gradient,
}

/// One border line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BorderEdgeDto {
    /// OOXML line style token: `none`, `thin`, `medium`, `dashed`, `dotted`, `thick`, `double`,
    /// `hair`, `mediumDashed`, `dashDot`, `mediumDashDot`, `dashDotDot`, `mediumDashDotDot`, or
    /// `slantDashDot`.
    pub style: String,
    /// Line color.
    #[serde(default)]
    pub color: Option<ColorDto>,
}

impl Default for BorderEdgeDto {
    fn default() -> Self {
        Self {
            style: "none".to_owned(),
            color: None,
        }
    }
}

/// A cell border.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BorderDto {
    /// Left line.
    #[serde(default)]
    pub left: BorderEdgeDto,
    /// Right line.
    #[serde(default)]
    pub right: BorderEdgeDto,
    /// Top line.
    #[serde(default)]
    pub top: BorderEdgeDto,
    /// Bottom line.
    #[serde(default)]
    pub bottom: BorderEdgeDto,
    /// Diagonal line.
    #[serde(default)]
    pub diagonal: BorderEdgeDto,
    /// Whether the diagonal runs from bottom left to top right.
    #[serde(default)]
    pub diagonal_up: bool,
    /// Whether the diagonal runs from top left to bottom right.
    #[serde(default)]
    pub diagonal_down: bool,
}

/// Cell text alignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlignmentDto {
    /// `general`, `left`, `center`, `right`, `fill`, `justify`, `centerContinuous`, or
    /// `distributed`.
    #[serde(default = "general_alignment")]
    pub horizontal: String,
    /// `top`, `center`, `bottom`, `justify`, or `distributed`.
    #[serde(default = "bottom_alignment")]
    pub vertical: String,
    /// Wrap text.
    #[serde(default)]
    pub wrap_text: bool,
    /// Shrink text to fit.
    #[serde(default)]
    pub shrink_to_fit: bool,
    /// Indent level from 0 through 250.
    #[serde(default)]
    pub indent: u32,
    /// 0 through 90 degrees counterclockwise, 91 through 180 for 1 through 90 degrees clockwise,
    /// or 255 for vertically stacked text.
    #[serde(default)]
    pub text_rotation: u32,
}

fn general_alignment() -> String {
    "general".to_owned()
}

fn bottom_alignment() -> String {
    "bottom".to_owned()
}

impl Default for AlignmentDto {
    fn default() -> Self {
        Self {
            horizontal: general_alignment(),
            vertical: bottom_alignment(),
            wrap_text: false,
            shrink_to_fit: false,
            indent: 0,
            text_rotation: 0,
        }
    }
}

/// The font, fill, border, and alignment a cell displays with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellFormatDto {
    /// Font.
    pub font: FontDto,
    /// Fill.
    pub fill: FillDto,
    /// Border.
    pub border: BorderDto,
    /// Alignment.
    pub alignment: AlignmentDto,
}

/// The format of one cell in a format page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellFormatEntryDto {
    /// Unqualified A1 address.
    pub address: String,
    /// Display format; the number format is on the range page's cells.
    pub format: CellFormatDto,
}

/// One span of columns that share an explicit width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnWidthDto {
    /// First one-based column.
    pub first: u32,
    /// Last one-based column.
    pub last: u32,
    /// Width in characters of the default font.
    pub width: f64,
}

/// One row with an explicit height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowHeightDto {
    /// One-based row.
    pub row: u32,
    /// Height in points.
    pub height: f64,
}

/// One bounded row-major page of cell formats with the layout of its columns and rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellFormatPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Inclusive range start.
    pub start: String,
    /// Inclusive range end.
    pub end: String,
    /// Total cell count in the requested rectangle.
    pub total_cells: u64,
    /// Offset represented by the first returned cell.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned cells in row-major order, including cells that hold no value.
    pub cells: Vec<CellFormatEntryDto>,
    /// Explicit widths of the requested columns, as sorted spans clipped to the range.
    pub column_widths: Vec<ColumnWidthDto>,
    /// Explicit heights of the rows the returned cells occupy.
    pub row_heights: Vec<RowHeightDto>,
}

impl FormatChangeV2Dto {
    /// Returns the sheet the operation targets.
    pub(crate) fn sheet(&self) -> &str {
        match self {
            Self::SetCellFormat { sheet, .. }
            | Self::SetColumnWidth { sheet, .. }
            | Self::SetRowHeight { sheet, .. }
            | Self::SetFrozenPane { sheet, .. }
            | Self::SetPhonetics { sheet, .. }
            | Self::ClearPhonetics { sheet, .. } => sheet,
        }
    }
}

/// Format and layout operations added to edit schema v2; they change presentation only and keep
/// the semantic revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FormatChangeV2Dto {
    /// Replaces the given components of one cell's format; omitted components keep their value.
    SetCellFormat {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address.
        address: String,
        /// Replacement font.
        #[serde(default)]
        font: Option<Box<FontDto>>,
        /// Replacement pattern fill.
        #[serde(default)]
        fill: Option<Box<PatternFillDto>>,
        /// Replacement border.
        #[serde(default)]
        border: Option<Box<BorderDto>>,
        /// Replacement alignment.
        #[serde(default)]
        alignment: Option<Box<AlignmentDto>>,
    },
    /// Sets or clears the width of an inclusive column span.
    SetColumnWidth {
        /// Case-insensitive sheet name.
        sheet: String,
        /// First one-based column.
        first: u32,
        /// Last one-based column.
        last: u32,
        /// Width in characters from 0 through 255; `null` restores the default width.
        width: Option<f64>,
    },
    /// Sets or clears the height of one row.
    SetRowHeight {
        /// Case-insensitive sheet name.
        sheet: String,
        /// One-based row.
        row: u32,
        /// Height in points from 0 through 409; `null` restores the default height.
        height: Option<f64>,
    },
    /// Sets the frozen rows and columns of the sheet's default view; zero rows and columns clear
    /// the pane.
    SetFrozenPane {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Frozen rows, below the 1,048,576-row grid.
        rows: u32,
        /// Frozen columns, below the 16,384-column grid.
        columns: u32,
    },
    /// Replaces the phonetic runs of an existing literal text cell.
    SetPhonetics {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address of a text cell.
        address: String,
        /// Runs over the cell text, in order and without overlap.
        runs: Vec<crate::PhoneticRunDto>,
        /// Whether the phonetic text is shown; omitted shows it.
        #[serde(default = "shown")]
        visible: bool,
        /// Display properties; omitted writes the default properties.
        #[serde(default)]
        properties: Option<crate::PhoneticPropertiesDto>,
    },
    /// Removes one cell's phonetic runs, properties, and visibility. A batch replaces annotated
    /// text by clearing its phonetics before setting the value.
    ClearPhonetics {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address.
        address: String,
    },
}

const fn shown() -> bool {
    true
}
