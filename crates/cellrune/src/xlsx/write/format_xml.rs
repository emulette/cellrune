//! SpreadsheetML records for the cell formatting model.
//!
//! Each record takes the namespace prefix of the part it is written into, `""` when SpreadsheetML
//! is the default namespace.

use std::fmt::Write as _;

use super::XlsxWriteError;
use super::serialization::escape_attribute;
use crate::{
    Alignment, Border, BorderEdge, BorderStyle, Color, ColorKind, Font, FontUnderline,
    HorizontalAlignment, PatternFill, VerticalAlignment,
};

/// Serializes a `<font>` record.
pub(super) fn font_xml(font: &Font, prefix: &str) -> Result<String, XlsxWriteError> {
    let mut xml = format!("<{prefix}font>");
    for (set, name) in [
        (font.bold(), "b"),
        (font.italic(), "i"),
        (font.strike(), "strike"),
    ] {
        if set {
            let _ = write!(xml, "<{prefix}{name}/>");
        }
    }
    if font.underline() != FontUnderline::None {
        let _ = write!(xml, "<{prefix}u val=\"{}\"/>", font.underline().as_str());
    }
    if let Some(size) = font.size() {
        let _ = write!(xml, "<{prefix}sz val=\"{size}\"/>");
    }
    if let Some(color) = font.color() {
        push_color(&mut xml, prefix, "color", color);
    }
    if let Some(name) = font.name() {
        let _ = write!(xml, "<{prefix}name val=\"{}\"/>", escape_attribute(name)?);
    }
    let _ = write!(xml, "</{prefix}font>");
    Ok(xml)
}

/// Serializes a `<fill>` record holding one pattern fill.
pub(super) fn fill_xml(fill: &PatternFill, prefix: &str) -> String {
    let mut xml = format!(
        "<{prefix}fill><{prefix}patternFill patternType=\"{}\"",
        fill.pattern().as_str()
    );
    if fill.foreground().is_none() && fill.background().is_none() {
        let _ = write!(xml, "/></{prefix}fill>");
        return xml;
    }
    xml.push('>');
    if let Some(color) = fill.foreground() {
        push_color(&mut xml, prefix, "fgColor", color);
    }
    if let Some(color) = fill.background() {
        push_color(&mut xml, prefix, "bgColor", color);
    }
    let _ = write!(xml, "</{prefix}patternFill></{prefix}fill>");
    xml
}

/// Serializes a `<border>` record with its edges in schema order.
pub(super) fn border_xml(border: &Border, prefix: &str) -> String {
    let mut xml = format!("<{prefix}border");
    if border.diagonal_up() {
        xml.push_str(" diagonalUp=\"1\"");
    }
    if border.diagonal_down() {
        xml.push_str(" diagonalDown=\"1\"");
    }
    xml.push('>');
    for (name, edge) in [
        ("left", border.left()),
        ("right", border.right()),
        ("top", border.top()),
        ("bottom", border.bottom()),
        ("diagonal", border.diagonal()),
    ] {
        push_edge(&mut xml, prefix, name, edge);
    }
    let _ = write!(xml, "</{prefix}border>");
    xml
}

/// Serializes an `<alignment>` child, or nothing for the default alignment.
pub(super) fn alignment_xml(alignment: &Alignment, prefix: &str) -> Option<String> {
    if *alignment == Alignment::default() {
        return None;
    }
    let mut xml = format!("<{prefix}alignment");
    if alignment.horizontal() != HorizontalAlignment::General {
        let _ = write!(xml, " horizontal=\"{}\"", alignment.horizontal().as_str());
    }
    if alignment.vertical() != VerticalAlignment::Bottom {
        let _ = write!(xml, " vertical=\"{}\"", alignment.vertical().as_str());
    }
    if alignment.text_rotation() != 0 {
        let _ = write!(xml, " textRotation=\"{}\"", alignment.text_rotation());
    }
    if alignment.wrap_text() {
        xml.push_str(" wrapText=\"1\"");
    }
    if alignment.indent() != 0 {
        let _ = write!(xml, " indent=\"{}\"", alignment.indent());
    }
    if alignment.shrink_to_fit() {
        xml.push_str(" shrinkToFit=\"1\"");
    }
    xml.push_str("/>");
    Some(xml)
}

fn push_edge(xml: &mut String, prefix: &str, name: &str, edge: BorderEdge) {
    if edge.style() == BorderStyle::None {
        let _ = write!(xml, "<{prefix}{name}/>");
        return;
    }
    let _ = write!(xml, "<{prefix}{name} style=\"{}\"", edge.style().as_str());
    match edge.color() {
        Some(color) => {
            xml.push('>');
            push_color(xml, prefix, "color", color);
            let _ = write!(xml, "</{prefix}{name}>");
        }
        None => xml.push_str("/>"),
    }
}

fn push_color(xml: &mut String, prefix: &str, name: &str, color: Color) {
    let _ = write!(xml, "<{prefix}{name}");
    match color.kind() {
        ColorKind::Argb(value) => {
            let _ = write!(xml, " rgb=\"{value:08X}\"");
        }
        ColorKind::Theme(index) => {
            let _ = write!(xml, " theme=\"{index}\"");
        }
        ColorKind::Indexed(index) => {
            let _ = write!(xml, " indexed=\"{index}\"");
        }
        ColorKind::Automatic => xml.push_str(" auto=\"1\""),
    }
    if color.tint() != 0.0 {
        let _ = write!(xml, " tint=\"{}\"", color.tint());
    }
    xml.push_str("/>");
}
