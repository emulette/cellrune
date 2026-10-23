//! Conversions between the cell formatting model and its DTOs.

use cellrune::{
    Alignment, AlignmentEdit, Border, BorderEdge, BorderEdit, BorderStyle, CellAddress, CellFormat,
    CellFormatEdit, Color, ColorKind, Column, Fill, FillPattern, FontEdit, FontUnderline,
    HorizontalAlignment, PatternFill, Row, SheetId, VerticalAlignment, WorkbookChange,
};

use crate::error::{
    DETAIL_BORDER_STYLE, DETAIL_COLOR_ARGB, DETAIL_FILL_PATTERN, DETAIL_FONT_UNDERLINE,
    DETAIL_HORIZONTAL_ALIGNMENT, DETAIL_VERTICAL_ALIGNMENT,
};
use crate::{
    AlignmentDto, AlignmentEditDto, BorderDto, BorderEdgeDto, BorderEditDto, CellFormatDto,
    ColorDto, FillDto, FontDto, FontEditDto, FormatChangeV2Dto, InteropError, PatternFillDto,
};

pub(crate) fn cell_format_dto(format: &CellFormat) -> CellFormatDto {
    let font = format.font();
    CellFormatDto {
        font: FontDto {
            name: font.name().map(str::to_owned),
            size: font.size(),
            bold: font.bold(),
            italic: font.italic(),
            underline: font.underline().as_str().to_owned(),
            strike: font.strike(),
            color: font.color().map(color_dto),
        },
        fill: match format.fill() {
            Fill::Pattern(fill) => FillDto::Pattern {
                pattern: fill.pattern().as_str().to_owned(),
                foreground: fill.foreground().map(color_dto),
                background: fill.background().map(color_dto),
            },
            _ => FillDto::Gradient,
        },
        border: border_dto(&format.border()),
        alignment: alignment_dto(&format.alignment()),
    }
}

fn color_dto(color: Color) -> ColorDto {
    let tint = color.tint();
    match color.kind() {
        ColorKind::Argb(value) => ColorDto::Argb {
            argb: format!("{value:08X}"),
            tint,
        },
        ColorKind::Theme(index) => ColorDto::Theme { index, tint },
        ColorKind::Indexed(index) => ColorDto::Indexed { index, tint },
        _ => ColorDto::Automatic { tint },
    }
}

fn border_dto(border: &Border) -> BorderDto {
    let edge = |edge: BorderEdge| BorderEdgeDto {
        style: edge.style().as_str().to_owned(),
        color: edge.color().map(color_dto),
    };
    BorderDto {
        left: edge(border.left()),
        right: edge(border.right()),
        top: edge(border.top()),
        bottom: edge(border.bottom()),
        diagonal: edge(border.diagonal()),
        diagonal_up: border.diagonal_up(),
        diagonal_down: border.diagonal_down(),
    }
}

fn alignment_dto(alignment: &Alignment) -> AlignmentDto {
    AlignmentDto {
        horizontal: alignment.horizontal().as_str().to_owned(),
        vertical: alignment.vertical().as_str().to_owned(),
        wrap_text: alignment.wrap_text(),
        shrink_to_fit: alignment.shrink_to_fit(),
        indent: alignment.indent(),
        text_rotation: alignment.text_rotation(),
    }
}

/// Converts one edit-schema-v2 format operation on a resolved sheet.
pub(crate) fn format_change(
    sheet_id: SheetId,
    change: FormatChangeV2Dto,
) -> Result<WorkbookChange, InteropError> {
    Ok(match change {
        FormatChangeV2Dto::SetCellFormat {
            address,
            font,
            fill,
            border,
            alignment,
            ..
        } => {
            let mut edit = CellFormatEdit::new();
            if let Some(font) = font {
                edit = edit.with_font(font_from_dto(*font)?);
            }
            if let Some(fill) = fill {
                edit = edit.with_fill(fill_from_dto(*fill)?);
            }
            if let Some(border) = border {
                edit = edit.with_border(border_from_dto(*border)?);
            }
            if let Some(alignment) = alignment {
                edit = edit.with_alignment(alignment_from_dto(&alignment)?);
            }
            WorkbookChange::set_cell_format(sheet_id, CellAddress::from_a1(&address)?, edit)
        }
        FormatChangeV2Dto::SetColumnWidth {
            first, last, width, ..
        } => WorkbookChange::set_column_width(
            sheet_id,
            Column::new(first)?,
            Column::new(last)?,
            width,
        ),
        FormatChangeV2Dto::SetRowHeight { row, height, .. } => {
            WorkbookChange::set_row_height(sheet_id, Row::new(row)?, height)
        }
    })
}

fn font_from_dto(font: FontEditDto) -> Result<FontEdit, InteropError> {
    let mut converted = FontEdit::new();
    if let Some(name) = font.name {
        converted = converted.with_name(name.as_deref())?;
    }
    if let Some(size) = font.size {
        converted = converted.with_size(size)?;
    }
    if let Some(bold) = font.bold {
        converted = converted.with_bold(bold);
    }
    if let Some(italic) = font.italic {
        converted = converted.with_italic(italic);
    }
    if let Some(underline) = font.underline {
        converted = converted.with_underline(
            FontUnderline::from_token(&underline)
                .ok_or_else(|| InteropError::invalid_change(DETAIL_FONT_UNDERLINE.to_owned()))?,
        );
    }
    if let Some(strike) = font.strike {
        converted = converted.with_strike(strike);
    }
    if let Some(color) = font.color {
        converted = converted.with_color(color.map(color_from_dto).transpose()?);
    }
    Ok(converted)
}

fn fill_from_dto(fill: PatternFillDto) -> Result<PatternFill, InteropError> {
    let pattern = FillPattern::from_token(&fill.pattern)
        .ok_or_else(|| InteropError::invalid_change(DETAIL_FILL_PATTERN.to_owned()))?;
    Ok(PatternFill::new(pattern)
        .with_foreground(fill.foreground.map(color_from_dto).transpose()?)
        .with_background(fill.background.map(color_from_dto).transpose()?))
}

fn border_from_dto(border: BorderEditDto) -> Result<BorderEdit, InteropError> {
    let mut converted = BorderEdit::new();
    if let Some(edge) = border.left {
        converted = converted.with_left(edge_from_dto(edge)?);
    }
    if let Some(edge) = border.right {
        converted = converted.with_right(edge_from_dto(edge)?);
    }
    if let Some(edge) = border.top {
        converted = converted.with_top(edge_from_dto(edge)?);
    }
    if let Some(edge) = border.bottom {
        converted = converted.with_bottom(edge_from_dto(edge)?);
    }
    if let Some(edge) = border.diagonal {
        converted = converted.with_diagonal(edge_from_dto(edge)?);
    }
    if let Some(up) = border.diagonal_up {
        converted = converted.with_diagonal_up(up);
    }
    if let Some(down) = border.diagonal_down {
        converted = converted.with_diagonal_down(down);
    }
    Ok(converted)
}

fn edge_from_dto(edge: BorderEdgeDto) -> Result<BorderEdge, InteropError> {
    let style = BorderStyle::from_token(&edge.style)
        .ok_or_else(|| InteropError::invalid_change(DETAIL_BORDER_STYLE.to_owned()))?;
    Ok(BorderEdge::new(
        style,
        edge.color.map(color_from_dto).transpose()?,
    ))
}

fn alignment_from_dto(alignment: &AlignmentEditDto) -> Result<AlignmentEdit, InteropError> {
    let mut converted = AlignmentEdit::new();
    if let Some(horizontal) = &alignment.horizontal {
        converted =
            converted.with_horizontal(HorizontalAlignment::from_token(horizontal).ok_or_else(
                || InteropError::invalid_change(DETAIL_HORIZONTAL_ALIGNMENT.to_owned()),
            )?);
    }
    if let Some(vertical) = &alignment.vertical {
        converted =
            converted.with_vertical(VerticalAlignment::from_token(vertical).ok_or_else(|| {
                InteropError::invalid_change(DETAIL_VERTICAL_ALIGNMENT.to_owned())
            })?);
    }
    if let Some(wrap_text) = alignment.wrap_text {
        converted = converted.with_wrap_text(wrap_text);
    }
    if let Some(shrink_to_fit) = alignment.shrink_to_fit {
        converted = converted.with_shrink_to_fit(shrink_to_fit);
    }
    if let Some(indent) = alignment.indent {
        converted = converted.with_indent(indent)?;
    }
    if let Some(text_rotation) = alignment.text_rotation {
        converted = converted.with_text_rotation(text_rotation)?;
    }
    Ok(converted)
}

fn color_from_dto(color: ColorDto) -> Result<Color, InteropError> {
    let (color, tint) = match color {
        ColorDto::Argb { argb, tint } => (Color::argb(parse_argb(&argb)?), tint),
        ColorDto::Theme { index, tint } => (Color::theme(index), tint),
        ColorDto::Indexed { index, tint } => (Color::indexed(index), tint),
        ColorDto::Automatic { tint } => (Color::automatic(), tint),
    };
    Ok(color.with_tint(tint)?)
}

fn parse_argb(value: &str) -> Result<u32, InteropError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InteropError::invalid_change(DETAIL_COLOR_ARGB.to_owned()));
    }
    u32::from_str_radix(value, 16)
        .map_err(|_| InteropError::invalid_change(DETAIL_COLOR_ARGB.to_owned()))
}
