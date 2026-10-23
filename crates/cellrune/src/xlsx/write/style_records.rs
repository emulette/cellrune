//! Finds a source style record equal to one the writer would add, so that edits reuse it instead
//! of appending a duplicate in every session.

use std::collections::HashMap;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use super::format_xml;
use crate::xlsx::package::PartPath;
use crate::{FillPattern, PatternFill};

use super::XlsxWriteError;

/// A style record's elements and attributes, independent of namespace prefixes, attribute order,
/// the empty-element form, and whitespace between elements. Two records with the same shape mean
/// the same thing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct RecordShape(Vec<ShapeToken>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ShapeToken {
    Start {
        local_name: String,
        attributes: Vec<(String, String)>,
    },
    End,
}

impl RecordShape {
    /// Returns the shape of one record's events, or `None` when it holds text or other content
    /// the writer never generates, which no generated record can equal.
    pub(super) fn of_events(
        events: &[Event<'_>],
        source: &PartPath,
    ) -> Result<Option<Self>, XlsxWriteError> {
        let mut tokens = Vec::with_capacity(events.len() * 2);
        for event in events {
            match event {
                Event::Start(element) => tokens.push(start_token(element, source)?),
                Event::Empty(element) => {
                    tokens.push(start_token(element, source)?);
                    tokens.push(ShapeToken::End);
                }
                Event::End(_) => tokens.push(ShapeToken::End),
                Event::Text(text)
                    if text
                        .xml_content(XmlVersion::Implicit1_0)
                        .chars()
                        .all(char::is_whitespace) => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(Self(tokens)))
    }

    /// Returns the shape of one generated record.
    fn of_xml(xml: &str, source: &PartPath) -> Result<Option<Self>, XlsxWriteError> {
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut events = Vec::new();
        loop {
            match reader
                .read_event()
                .map_err(|error| super::styles_patch::invalid_xml(source, error))?
            {
                Event::Eof => break,
                event => events.push(event.into_owned()),
            }
        }
        Self::of_events(&events, source)
    }
}

fn start_token(element: &BytesStart<'_>, source: &PartPath) -> Result<ShapeToken, XlsxWriteError> {
    let mut attributes = Vec::new();
    for attribute in element.attributes().with_checks(true) {
        let attribute =
            attribute.map_err(|error| super::styles_patch::invalid_xml(source, error))?;
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|error| super::styles_patch::invalid_xml(source, error))?
            .into_owned();
        attributes.push((attribute.key.as_ref().to_owned(), value));
    }
    attributes.sort_unstable();
    Ok(ShapeToken::Start {
        local_name: element.local_name().as_ref().to_owned(),
        attributes,
    })
}

/// The first index of each distinct record shape in one source styles section.
#[derive(Debug, Default)]
pub(super) struct SourceRecords {
    fonts: HashMap<RecordShape, usize>,
    fills: HashMap<RecordShape, usize>,
    borders: HashMap<RecordShape, usize>,
    xfs: HashMap<RecordShape, usize>,
}

impl SourceRecords {
    /// Records the `index`th record of `section`.
    pub(super) fn insert(
        &mut self,
        section: &str,
        index: usize,
        events: &[Event<'_>],
        source: &PartPath,
    ) -> Result<(), XlsxWriteError> {
        let records = match section {
            "fonts" => &mut self.fonts,
            "fills" => &mut self.fills,
            "borders" => &mut self.borders,
            "cellXfs" => &mut self.xfs,
            _ => return Ok(()),
        };
        let Some(shape) = RecordShape::of_events(events, source)? else {
            return Ok(());
        };
        // Excel reads fill records 0 and 1 as `none` and `gray125` whatever they hold, so only a
        // reserved record that says so may stand in for a requested fill.
        if section == "fills"
            && index < 2
            && Some(&shape) != reserved_fill_shape(index, source)?.as_ref()
        {
            return Ok(());
        }
        records.entry(shape).or_insert(index);
        Ok(())
    }

    /// Returns the index of a source font that `xml` would duplicate.
    pub(super) fn font(
        &self,
        xml: &str,
        source: &PartPath,
    ) -> Result<Option<usize>, XlsxWriteError> {
        find(&self.fonts, xml, source)
    }

    /// Returns the index of a source fill that `xml` would duplicate.
    pub(super) fn fill(
        &self,
        xml: &str,
        source: &PartPath,
    ) -> Result<Option<usize>, XlsxWriteError> {
        find(&self.fills, xml, source)
    }

    /// Returns the index of a source border that `xml` would duplicate.
    pub(super) fn border(
        &self,
        xml: &str,
        source: &PartPath,
    ) -> Result<Option<usize>, XlsxWriteError> {
        find(&self.borders, xml, source)
    }

    /// Returns the index of a source `cellXfs` entry that `events` would duplicate.
    pub(super) fn xf(
        &self,
        events: &[Event<'_>],
        source: &PartPath,
    ) -> Result<Option<usize>, XlsxWriteError> {
        Ok(RecordShape::of_events(events, source)?.and_then(|shape| self.xfs.get(&shape).copied()))
    }
}

fn find(
    records: &HashMap<RecordShape, usize>,
    xml: &str,
    source: &PartPath,
) -> Result<Option<usize>, XlsxWriteError> {
    Ok(RecordShape::of_xml(xml, source)?.and_then(|shape| records.get(&shape).copied()))
}

fn reserved_fill_shape(
    index: usize,
    source: &PartPath,
) -> Result<Option<RecordShape>, XlsxWriteError> {
    let fill = if index == 0 {
        PatternFill::default()
    } else {
        PatternFill::new(FillPattern::Gray125)
    };
    RecordShape::of_xml(&format_xml::fill_xml(&fill, ""), source)
}
