use super::ast::{
    StructuredColumns, StructuredItem, StructuredReference,
    structured_column_character_needs_grouping,
};
use super::syntax::{SourceComponent, SourceComponentKind, SourceSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StructuredReferenceError {
    MissingSelector,
    InvalidBrackets,
    InvalidItem,
    UnescapedSpecialCharacter,
    TooManyColumns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedStructuredReference {
    pub reference: StructuredReference,
    pub components: Vec<SourceComponent>,
}

fn source_span(raw: &str, value: &str) -> SourceSpan {
    let start = value.as_ptr() as usize - raw.as_ptr() as usize;
    SourceSpan::new(start, start + value.len())
}

fn trim_syntax_spaces(value: &str) -> &str {
    value.trim_matches(' ')
}

fn source_component(raw: &str, kind: SourceComponentKind, value: &str) -> SourceComponent {
    SourceComponent::new(kind, source_span(raw, value))
}

fn ungrouped_column(value: &str) -> Result<Box<str>, StructuredReferenceError> {
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '\'' {
            let Some(escaped) = characters.next() else {
                return Err(StructuredReferenceError::InvalidBrackets);
            };
            if !matches!(escaped, '[' | ']' | '#' | '\'' | '@') {
                return Err(StructuredReferenceError::UnescapedSpecialCharacter);
            }
        } else if structured_column_character_needs_grouping(character) {
            return Err(StructuredReferenceError::InvalidBrackets);
        }
    }
    Ok(unescape_name(value)?.into_boxed_str())
}

fn unescape_name(value: &str) -> Result<String, StructuredReferenceError> {
    let mut result = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '\'' {
            let Some(escaped) = characters.next() else {
                return Err(StructuredReferenceError::InvalidBrackets);
            };
            if !matches!(escaped, '[' | ']' | '#' | '\'' | '@') {
                return Err(StructuredReferenceError::UnescapedSpecialCharacter);
            }
            result.push(escaped);
        } else if matches!(character, '[' | ']' | '#' | '@') {
            return Err(StructuredReferenceError::UnescapedSpecialCharacter);
        } else {
            result.push(character);
        }
    }
    if result.is_empty() {
        Err(StructuredReferenceError::MissingSelector)
    } else {
        Ok(result)
    }
}

fn split_top_level(value: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_u32;
    let mut escaped = false;
    let mut start = 0;
    for (offset, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\'' => escaped = true,
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            current if current == delimiter && depth == 0 => {
                parts.push(&value[start..offset]);
                start = offset + current.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

fn unwrap_component(value: &str) -> Result<&str, StructuredReferenceError> {
    value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or(StructuredReferenceError::InvalidBrackets)
}

fn item(value: &str) -> Option<StructuredItem> {
    if value.eq_ignore_ascii_case("#All") {
        Some(StructuredItem::All)
    } else if value.eq_ignore_ascii_case("#Data") {
        Some(StructuredItem::Data)
    } else if value.eq_ignore_ascii_case("#Headers") {
        Some(StructuredItem::Headers)
    } else if value.eq_ignore_ascii_case("#Totals") {
        Some(StructuredItem::Totals)
    } else if value.eq_ignore_ascii_case("#This Row") {
        Some(StructuredItem::ThisRow)
    } else {
        None
    }
}

pub(super) fn parse_structured_reference(
    raw: &str,
) -> Result<ParsedStructuredReference, StructuredReferenceError> {
    let Some(open) = raw.find('[') else {
        return Err(StructuredReferenceError::InvalidBrackets);
    };
    let table = (open > 0).then(|| raw[..open].to_owned().into_boxed_str());
    let mut source_components = Vec::new();
    if open > 0 {
        source_components.push(SourceComponent::new(
            SourceComponentKind::StructuredTable,
            SourceSpan::new(0, open),
        ));
    }
    let outer = raw[open..]
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or(StructuredReferenceError::InvalidBrackets)?;
    if outer.is_empty() {
        if table.is_none() {
            return Err(StructuredReferenceError::MissingSelector);
        }
        return Ok(ParsedStructuredReference {
            reference: StructuredReference {
                table,
                items: vec![StructuredItem::Data],
                columns: None,
            },
            components: source_components,
        });
    }
    let trimmed_outer = trim_syntax_spaces(outer);

    if trimmed_outer == "@" {
        source_components.push(source_component(
            raw,
            SourceComponentKind::StructuredItem(0),
            trimmed_outer,
        ));
        return Ok(ParsedStructuredReference {
            reference: StructuredReference {
                table,
                items: vec![StructuredItem::ThisRow],
                columns: None,
            },
            components: source_components,
        });
    }
    if let Some(column) = trimmed_outer.strip_prefix('@') {
        source_components.push(source_component(
            raw,
            SourceComponentKind::StructuredItem(0),
            &trimmed_outer[..1],
        ));
        let ranges = split_top_level(column, ':');
        let columns = match ranges.as_slice() {
            [single] if single.starts_with('[') => {
                let value = trim_syntax_spaces(unwrap_component(single)?);
                source_components.push(source_component(
                    raw,
                    SourceComponentKind::StructuredColumn { grouped: true },
                    value,
                ));
                StructuredColumns::Single(unescape_name(value)?.into_boxed_str())
            }
            [single] => {
                source_components.push(source_component(
                    raw,
                    SourceComponentKind::StructuredColumn { grouped: false },
                    single,
                ));
                StructuredColumns::Single(ungrouped_column(single)?)
            }
            [start, end] => {
                let start = trim_syntax_spaces(unwrap_component(start)?);
                let end = trim_syntax_spaces(unwrap_component(end)?);
                source_components.push(source_component(
                    raw,
                    SourceComponentKind::StructuredColumnStart { grouped: true },
                    start,
                ));
                source_components.push(source_component(
                    raw,
                    SourceComponentKind::StructuredColumnEnd { grouped: true },
                    end,
                ));
                StructuredColumns::Range {
                    start: unescape_name(start)?.into_boxed_str(),
                    end: unescape_name(end)?.into_boxed_str(),
                }
            }
            _ => return Err(StructuredReferenceError::TooManyColumns),
        };
        return Ok(ParsedStructuredReference {
            reference: StructuredReference {
                table,
                items: vec![StructuredItem::ThisRow],
                columns: Some(columns),
            },
            components: source_components,
        });
    }

    let grouped = trimmed_outer.starts_with('[');
    let selector = trimmed_outer;
    let components = if grouped {
        split_top_level(selector, ',')
    } else {
        vec![selector]
    };
    let mut items = Vec::new();
    let mut columns = None;
    for component in components {
        let component = if grouped {
            trim_syntax_spaces(component)
        } else {
            component
        };
        let ranges = split_top_level(component, ':');
        if ranges.len() > 2 {
            return Err(StructuredReferenceError::TooManyColumns);
        }
        if ranges.len() == 2 {
            if columns.is_some() {
                return Err(StructuredReferenceError::TooManyColumns);
            }
            let start = trim_syntax_spaces(unwrap_component(ranges[0])?);
            let end = trim_syntax_spaces(unwrap_component(ranges[1])?);
            source_components.push(source_component(
                raw,
                SourceComponentKind::StructuredColumnStart { grouped: true },
                start,
            ));
            source_components.push(source_component(
                raw,
                SourceComponentKind::StructuredColumnEnd { grouped: true },
                end,
            ));
            columns = Some(StructuredColumns::Range {
                start: unescape_name(start)?.into_boxed_str(),
                end: unescape_name(end)?.into_boxed_str(),
            });
            continue;
        }
        let component = if grouped {
            trim_syntax_spaces(unwrap_component(component)?)
        } else {
            component
        };
        if let Some(selector) = item(component) {
            if items.contains(&selector) {
                return Err(StructuredReferenceError::InvalidItem);
            }
            source_components.push(source_component(
                raw,
                SourceComponentKind::StructuredItem(
                    u16::try_from(items.len())
                        .map_err(|_| StructuredReferenceError::InvalidItem)?,
                ),
                component,
            ));
            items.push(selector);
        } else if component.starts_with('#') {
            return Err(StructuredReferenceError::InvalidItem);
        } else if columns.is_none() {
            source_components.push(source_component(
                raw,
                SourceComponentKind::StructuredColumn { grouped },
                component,
            ));
            columns = Some(StructuredColumns::Single(if grouped {
                unescape_name(component)?.into_boxed_str()
            } else {
                ungrouped_column(component)?
            }));
        } else {
            return Err(StructuredReferenceError::TooManyColumns);
        }
    }
    if items.is_empty() && columns.is_none() {
        return Err(StructuredReferenceError::MissingSelector);
    }
    let valid_item_sequence = items.len() <= 1
        || items.as_slice() == [StructuredItem::Headers, StructuredItem::Data]
        || items.as_slice() == [StructuredItem::Data, StructuredItem::Totals];
    if !valid_item_sequence {
        return Err(StructuredReferenceError::InvalidItem);
    }
    Ok(ParsedStructuredReference {
        reference: StructuredReference {
            table,
            items,
            columns,
        },
        components: source_components,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_structured_reference;
    use crate::calculation::ast::{StructuredColumns, StructuredItem};

    #[test]
    fn parses_items_columns_ranges_and_escapes() {
        let reference = parse_structured_reference("Table1[[#Headers],[Amount]]")
            .expect("structured")
            .reference;
        assert_eq!(reference.table.as_deref(), Some("Table1"));
        assert_eq!(reference.items, [StructuredItem::Headers]);
        assert_eq!(
            reference.columns,
            Some(StructuredColumns::Single("Amount".into()))
        );

        let reference = parse_structured_reference("Table1[[Col1]:[Col2]]")
            .expect("column range")
            .reference;
        assert_eq!(
            reference.columns,
            Some(StructuredColumns::Range {
                start: "Col1".into(),
                end: "Col2".into(),
            })
        );

        let reference = parse_structured_reference("Table1['[odd']name]")
            .expect("escaped column")
            .reference;
        assert_eq!(
            reference.columns,
            Some(StructuredColumns::Single("[odd]name".into()))
        );

        let reference = parse_structured_reference("Table1[['#Headers]:[Amount]]")
            .expect("escaped special-character range endpoint")
            .reference;
        assert_eq!(
            reference.columns,
            Some(StructuredColumns::Range {
                start: "#Headers".into(),
                end: "Amount".into(),
            })
        );

        for (input, expected) in [
            ("Table1[A|B]", "A|B"),
            ("Table1[😀]", "😀"),
            ("Table1[A\u{a0}B]", "A\u{a0}B"),
        ] {
            assert_eq!(
                parse_structured_reference(input)
                    .expect(input)
                    .reference
                    .columns,
                Some(StructuredColumns::Single(expected.into())),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_unescaped_reserved_column_characters_and_short_ranges() {
        for invalid in [
            "Table1[[#Headers]:[Amount]]",
            "Table1[[#Bogus]:[Amount]]",
            "Table1[[@Header]:[Amount]]",
            "Table1[[A[B]],[Amount]]",
            "Table1[@#Headers]",
            "Table1[A:B]",
            "Table1[Col,1]",
            "Table1[Col.1]",
            "Table1[Under_score]",
            "Table1[Col%]",
            "Table1[@% Commission]",
            "Table1['#ok,comma]",
            "Table1['#ok.period]",
        ] {
            assert!(
                parse_structured_reference(invalid).is_err(),
                "{invalid} must require the structured-reference escape grammar"
            );
        }
    }

    #[test]
    fn grouped_selectors_allow_syntax_whitespace_but_require_nested_brackets() {
        let spaced = parse_structured_reference("Table1[ [Sales]:[Region] ]")
            .expect("whitespace around a grouped column span")
            .reference;
        assert_eq!(
            spaced.columns,
            Some(StructuredColumns::Range {
                start: "Sales".into(),
                end: "Region".into(),
            })
        );
        let spaced = parse_structured_reference("Table1[[#Headers], [#Data], [Amount]]")
            .expect("whitespace between grouped selectors")
            .reference;
        assert_eq!(
            spaced.items,
            [StructuredItem::Headers, StructuredItem::Data]
        );

        for invalid in [
            "Table1[[Col1]:Col2]",
            "Table1[[#Data],Col]",
            "Table1[[#Headers],\t[Amount]]",
        ] {
            assert!(parse_structured_reference(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parses_current_row_bare_bracketed_and_column_span_forms() {
        let bare = parse_structured_reference("Table1[@]")
            .expect("bare current row")
            .reference;
        assert_eq!(bare.items, [StructuredItem::ThisRow]);
        assert_eq!(bare.columns, None);
        assert_eq!(
            parse_structured_reference("Table1[ @ ]")
                .expect("spaced current row")
                .reference,
            bare
        );

        let bracketed = parse_structured_reference("[@[Sales Amount]]")
            .expect("bracketed current-row column")
            .reference;
        assert_eq!(
            bracketed.columns,
            Some(StructuredColumns::Single("Sales Amount".into()))
        );
        assert_eq!(
            parse_structured_reference("Table1[ @Amount ]")
                .expect("spaced short current-row column")
                .reference
                .columns,
            Some(StructuredColumns::Single("Amount".into()))
        );

        let span = parse_structured_reference("Table1[@[January]:[December]]")
            .expect("current-row column span")
            .reference;
        assert_eq!(
            span.columns,
            Some(StructuredColumns::Range {
                start: "January".into(),
                end: "December".into(),
            })
        );
        assert!(parse_structured_reference("Table1[[#This Row],[#Data],[Amount]]").is_err());
    }

    #[test]
    fn enforces_keyword_sequences_and_empty_table_data_shorthand() {
        let shorthand = parse_structured_reference("Table1[]")
            .expect("qualified empty brackets mean table data")
            .reference;
        assert_eq!(shorthand.items, [StructuredItem::Data]);
        assert_eq!(shorthand.columns, None);
        assert!(parse_structured_reference("[]").is_err());

        for valid in [
            "Table1[[#Headers],[#Data],[Amount]]",
            "Table1[[#Data],[#Totals],[Amount]]",
        ] {
            parse_structured_reference(valid).expect(valid);
        }
        for invalid in [
            "Table1[[#All],[#Data],[Amount]]",
            "Table1[[#Totals],[#Headers],[Amount]]",
            "Table1[[#Data],[#Headers],[Amount]]",
            "Table1[[#Headers],[#Data],[#Totals],[Amount]]",
        ] {
            assert!(parse_structured_reference(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn trims_only_grammar_spaces_inside_nested_columns() {
        for input in [
            "Table1[[ Amount ]]",
            "Table1[[#Data], [ Amount ]]",
            "Table1[@[ Amount ]]",
        ] {
            let parsed = parse_structured_reference(input).expect(input).reference;
            assert_eq!(
                parsed.columns,
                Some(StructuredColumns::Single("Amount".into())),
                "{input}"
            );
        }
        assert_eq!(
            parse_structured_reference("Table1[ Amount ]")
                .expect("outer syntax spaces")
                .reference,
            parse_structured_reference("Table1[Amount]")
                .expect("plain column")
                .reference
        );
    }
}
