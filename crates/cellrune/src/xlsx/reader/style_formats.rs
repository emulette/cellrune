//! Reads the fonts, fills, borders, and alignments of `styles.xml` into one [`CellFormat`] per
//! `cellXfs` entry.
//!
//! These records were not read before they were modeled, so a value the model cannot represent,
//! such as an unknown token or an out-of-range record index, leaves that part at its default
//! instead of failing the read.

use super::super::xml::XmlAttributes;
use crate::presentation::VERTICAL_TEXT_ROTATION;
use crate::{
    Alignment, Border, BorderEdge, BorderStyle, CellFormat, Color, Fill, FillPattern, Font,
    FontUnderline, HorizontalAlignment, PatternFill, VerticalAlignment,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Fonts,
    Fills,
    Borders,
    CellFormats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    Diagonal,
}

#[derive(Debug, Default)]
struct PendingFill {
    gradient: bool,
    pattern: PatternFill,
}

#[derive(Debug)]
struct PendingBorder {
    border: Border,
    edge: Option<(Edge, BorderEdge)>,
    diagonal_up: bool,
    diagonal_down: bool,
}

#[derive(Debug, Default)]
struct FormatRecord {
    font: usize,
    fill: usize,
    border: usize,
    alignment: Alignment,
}

/// Collects the style records one `cellXfs` entry can point at.
#[derive(Debug, Default)]
pub(super) struct StyleFormatCollector {
    section: Option<Section>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    records: Vec<FormatRecord>,
    font: Option<Font>,
    fill: Option<PendingFill>,
    border: Option<PendingBorder>,
    record: Option<FormatRecord>,
}

impl StyleFormatCollector {
    /// Records one SpreadsheetML start or empty element at `depth`.
    pub(super) fn element(
        &mut self,
        local_name: &[u8],
        depth: u64,
        attributes: &XmlAttributes,
        empty: bool,
    ) {
        match depth {
            2 => {
                self.section = match local_name {
                    b"fonts" => Some(Section::Fonts),
                    b"fills" => Some(Section::Fills),
                    b"borders" => Some(Section::Borders),
                    b"cellXfs" => Some(Section::CellFormats),
                    _ => None,
                };
            }
            3 => self.start_record(local_name, attributes, empty),
            4 => self.record_child(local_name, attributes),
            5 => self.record_grandchild(local_name, attributes),
            _ => {}
        }
        if empty && depth == 4 {
            self.close_edge();
        }
    }

    /// Closes the element at `depth`.
    pub(super) fn end(&mut self, depth: u64) {
        match depth {
            2 => self.section = None,
            3 => self.finish_record(),
            4 => self.close_edge(),
            _ => {}
        }
    }

    /// Returns one format per `cellXfs` entry.
    pub(super) fn finish(self) -> Vec<CellFormat> {
        self.records
            .into_iter()
            .map(|record| {
                CellFormat::from_parts(
                    self.fonts.get(record.font).cloned().unwrap_or_default(),
                    self.fills.get(record.fill).copied().unwrap_or_default(),
                    self.borders.get(record.border).copied().unwrap_or_default(),
                    record.alignment,
                )
            })
            .collect()
    }

    fn start_record(&mut self, local_name: &[u8], attributes: &XmlAttributes, empty: bool) {
        match (self.section, local_name) {
            (Some(Section::Fonts), b"font") => self.font = Some(Font::default()),
            (Some(Section::Fills), b"fill") => self.fill = Some(PendingFill::default()),
            (Some(Section::Borders), b"border") => {
                self.border = Some(PendingBorder {
                    border: Border::default(),
                    edge: None,
                    diagonal_up: flag(attributes.unqualified("diagonalUp"), false),
                    diagonal_down: flag(attributes.unqualified("diagonalDown"), false),
                });
            }
            (Some(Section::CellFormats), b"xf") => {
                let index = |name| {
                    attributes
                        .unqualified(name)
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0)
                };
                self.record = Some(FormatRecord {
                    font: index("fontId"),
                    fill: index("fillId"),
                    border: index("borderId"),
                    alignment: Alignment::default(),
                });
            }
            _ => return,
        }
        if empty {
            self.finish_record();
        }
    }

    fn record_child(&mut self, local_name: &[u8], attributes: &XmlAttributes) {
        if let Some(font) = self.font.take() {
            self.font = Some(font_child(font, local_name, attributes));
        } else if let Some(fill) = &mut self.fill {
            match local_name {
                b"patternFill" => {
                    fill.pattern = PatternFill::new(
                        attributes
                            .unqualified("patternType")
                            .and_then(FillPattern::from_token)
                            .unwrap_or_default(),
                    );
                }
                b"gradientFill" => fill.gradient = true,
                _ => {}
            }
        } else if let Some(border) = &mut self.border {
            let edge = match local_name {
                b"left" | b"start" => Edge::Left,
                b"right" | b"end" => Edge::Right,
                b"top" => Edge::Top,
                b"bottom" => Edge::Bottom,
                b"diagonal" => Edge::Diagonal,
                _ => return,
            };
            let style = attributes
                .unqualified("style")
                .and_then(BorderStyle::from_token)
                .unwrap_or_default();
            border.edge = Some((edge, BorderEdge::new(style, None)));
        } else if let Some(record) = &mut self.record
            && local_name == b"alignment"
        {
            record.alignment = alignment(attributes);
        }
    }

    fn record_grandchild(&mut self, local_name: &[u8], attributes: &XmlAttributes) {
        if let Some(fill) = &mut self.fill {
            let color = color(attributes);
            fill.pattern = match local_name {
                b"fgColor" => fill.pattern.with_foreground(color),
                b"bgColor" => fill.pattern.with_background(color),
                _ => return,
            };
        } else if let Some(PendingBorder {
            edge: Some((_, edge)),
            ..
        }) = &mut self.border
            && local_name == b"color"
        {
            *edge = BorderEdge::new(edge.style(), color(attributes));
        }
    }

    fn close_edge(&mut self) {
        let Some(pending) = &mut self.border else {
            return;
        };
        let Some((side, edge)) = pending.edge.take() else {
            return;
        };
        pending.border = match side {
            Edge::Left => pending.border.with_left(edge),
            Edge::Right => pending.border.with_right(edge),
            Edge::Top => pending.border.with_top(edge),
            Edge::Bottom => pending.border.with_bottom(edge),
            Edge::Diagonal => pending.border.with_diagonal(
                edge,
                pending.border.diagonal_up(),
                pending.border.diagonal_down(),
            ),
        };
    }

    fn finish_record(&mut self) {
        if let Some(font) = self.font.take() {
            self.fonts.push(font);
        } else if let Some(fill) = self.fill.take() {
            self.fills.push(if fill.gradient {
                Fill::Gradient
            } else {
                Fill::Pattern(fill.pattern)
            });
        } else if let Some(pending) = self.border.take() {
            let border = pending.border;
            self.borders.push(border.with_diagonal(
                border.diagonal(),
                pending.diagonal_up,
                pending.diagonal_down,
            ));
        } else if let Some(record) = self.record.take() {
            self.records.push(record);
        }
    }
}

fn font_child(font: Font, local_name: &[u8], attributes: &XmlAttributes) -> Font {
    let value = attributes.unqualified("val");
    match local_name {
        b"b" => font.with_bold(flag(value, true)),
        b"i" => font.with_italic(flag(value, true)),
        b"strike" => font.with_strike(flag(value, true)),
        b"u" => font.with_underline(
            value
                .map_or(Some(FontUnderline::Single), FontUnderline::from_token)
                .unwrap_or_default(),
        ),
        b"sz" => match value.and_then(|value| value.parse::<f64>().ok()) {
            Some(size) => font.clone().with_size(size).unwrap_or(font),
            None => font,
        },
        b"name" => match value {
            Some(name) => font.clone().with_name(name).unwrap_or(font),
            None => font,
        },
        b"color" => font.with_color(color(attributes)),
        _ => font,
    }
}

/// Reads an OOXML boolean attribute whose absence means `default`.
fn flag(value: Option<&str>, default: bool) -> bool {
    match value {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    }
}

/// Reads a `CT_Color`: `rgb`, else `theme`, else `indexed`, else `auto`, each with its tint.
pub(super) fn color(attributes: &XmlAttributes) -> Option<Color> {
    let number = |name| {
        attributes
            .unqualified(name)
            .and_then(|value| value.parse::<u32>().ok())
    };
    let color = if let Some(rgb) = attributes.unqualified("rgb") {
        Color::argb(parse_argb(rgb)?)
    } else if let Some(theme) = number("theme") {
        Color::theme(theme)
    } else if let Some(indexed) = number("indexed") {
        Color::indexed(indexed)
    } else if flag(attributes.unqualified("auto"), false) {
        Color::automatic()
    } else {
        return None;
    };
    let tint = attributes
        .unqualified("tint")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0);
    Some(color.with_tint(tint).unwrap_or(color))
}

/// Parses eight hex digits `AARRGGBB`, or six `RRGGBB` as an opaque color.
fn parse_argb(value: &str) -> Option<u32> {
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match value.len() {
        8 => u32::from_str_radix(value, 16).ok(),
        6 => u32::from_str_radix(value, 16)
            .ok()
            .map(|rgb| 0xFF00_0000 | rgb),
        _ => None,
    }
}

fn alignment(attributes: &XmlAttributes) -> Alignment {
    let number = |name| {
        attributes
            .unqualified(name)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let mut alignment = Alignment::default()
        .with_horizontal(
            attributes
                .unqualified("horizontal")
                .and_then(HorizontalAlignment::from_token)
                .unwrap_or_default(),
        )
        .with_vertical(
            attributes
                .unqualified("vertical")
                .and_then(VerticalAlignment::from_token)
                .unwrap_or_default(),
        )
        .with_wrap_text(flag(attributes.unqualified("wrapText"), false))
        .with_shrink_to_fit(flag(attributes.unqualified("shrinkToFit"), false));
    alignment = alignment.with_indent(number("indent")).unwrap_or(alignment);
    let rotation = number("textRotation");
    if rotation <= 180 || rotation == VERTICAL_TEXT_ROTATION {
        alignment = alignment.with_text_rotation(rotation).unwrap_or(alignment);
    }
    alignment
}
