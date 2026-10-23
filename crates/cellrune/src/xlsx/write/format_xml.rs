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

/// `<font>` children in the order Excel writes them.
const FONT_CHILDREN: [&str; 15] = [
    "b",
    "i",
    "strike",
    "condense",
    "extend",
    "outline",
    "shadow",
    "u",
    "vertAlign",
    "sz",
    "color",
    "name",
    "family",
    "charset",
    "scheme",
];
/// `<font>` children the [`Font`] model carries.
pub(super) const MODELED_FONT_CHILDREN: [&str; 7] =
    ["b", "i", "strike", "u", "sz", "color", "name"];
/// `<font>` children that describe the font `name` names, which a new name invalidates.
pub(super) const NAME_BOUND_FONT_CHILDREN: [&str; 3] = ["family", "charset", "scheme"];
/// `<border>` attributes and children the [`Border`] model carries.
pub(super) const MODELED_BORDER_ATTRIBUTES: [&str; 2] = ["diagonalUp", "diagonalDown"];
pub(super) const MODELED_BORDER_CHILDREN: [&str; 7] =
    ["left", "start", "right", "end", "top", "bottom", "diagonal"];
/// `<alignment>` attributes the [`Alignment`] model carries.
pub(super) const MODELED_ALIGNMENT_ATTRIBUTES: [&str; 6] = [
    "horizontal",
    "vertical",
    "textRotation",
    "wrapText",
    "indent",
    "shrinkToFit",
];

/// A child element of a source record that the model does not carry, kept as written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct SourceChild {
    pub(super) local_name: String,
    pub(super) xml: String,
}

/// Serializes a `<font>` record.
pub(super) fn font_xml(font: &Font, prefix: &str) -> Result<String, XlsxWriteError> {
    font_xml_over(font, prefix, &[])
}

/// Serializes a `<font>` record with the source children the model does not carry, each in the
/// position Excel writes it.
pub(super) fn font_xml_over(
    font: &Font,
    prefix: &str,
    kept: &[SourceChild],
) -> Result<String, XlsxWriteError> {
    let mut xml = format!("<{prefix}font>");
    for name in FONT_CHILDREN {
        match name {
            "b" | "i" | "strike" => {
                let set = match name {
                    "b" => font.bold(),
                    "i" => font.italic(),
                    _ => font.strike(),
                };
                if set {
                    let _ = write!(xml, "<{prefix}{name}/>");
                }
            }
            "u" => {
                if font.underline() != FontUnderline::None {
                    let _ = write!(xml, "<{prefix}u val=\"{}\"/>", font.underline().as_str());
                }
            }
            "sz" => {
                if let Some(size) = font.size() {
                    let _ = write!(xml, "<{prefix}sz val=\"{size}\"/>");
                }
            }
            "color" => {
                if let Some(color) = font.color() {
                    push_color(&mut xml, prefix, "color", color);
                }
            }
            "name" => {
                if let Some(name) = font.name() {
                    let _ = write!(xml, "<{prefix}name val=\"{}\"/>", escape_attribute(name)?);
                }
            }
            _ => push_kept(&mut xml, kept, |kept| kept == name),
        }
    }
    push_kept(&mut xml, kept, |kept| !FONT_CHILDREN.contains(&kept));
    let _ = write!(xml, "</{prefix}font>");
    Ok(xml)
}

fn push_kept(xml: &mut String, kept: &[SourceChild], selected: impl Fn(&str) -> bool) {
    for child in kept.iter().filter(|child| selected(&child.local_name)) {
        xml.push_str(&child.xml);
    }
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
    border_xml_over(border, prefix, &[], &[])
}

/// Serializes a `<border>` record with the source attributes and children the model does not
/// carry; the kept children, such as `vertical` and `horizontal`, follow the edges.
pub(super) fn border_xml_over(
    border: &Border,
    prefix: &str,
    kept_attributes: &[(String, String)],
    kept_children: &[SourceChild],
) -> String {
    let mut xml = format!("<{prefix}border");
    if border.diagonal_up() {
        xml.push_str(" diagonalUp=\"1\"");
    }
    if border.diagonal_down() {
        xml.push_str(" diagonalDown=\"1\"");
    }
    push_kept_attributes(&mut xml, kept_attributes);
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
    push_kept(&mut xml, kept_children, |_| true);
    let _ = write!(xml, "</{prefix}border>");
    xml
}

/// Serializes an `<alignment>` child, or nothing for the default alignment.
pub(super) fn alignment_xml(alignment: &Alignment, prefix: &str) -> Option<String> {
    alignment_xml_over(alignment, prefix, &[])
}

/// Serializes an `<alignment>` child with the source attributes the model does not carry, such as
/// `readingOrder`, or nothing when neither leaves anything to write.
pub(super) fn alignment_xml_over(
    alignment: &Alignment,
    prefix: &str,
    kept_attributes: &[(String, String)],
) -> Option<String> {
    if *alignment == Alignment::default() && kept_attributes.is_empty() {
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
    push_kept_attributes(&mut xml, kept_attributes);
    xml.push_str("/>");
    Some(xml)
}

/// Appends source attributes; their values are kept escaped as the source wrote them.
fn push_kept_attributes(xml: &mut String, attributes: &[(String, String)]) {
    for (key, value) in attributes {
        let _ = write!(xml, " {key}=\"{value}\"");
    }
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
