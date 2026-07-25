use quick_xml::XmlVersion;
use quick_xml::events::{BytesCData, BytesRef, BytesStart, BytesText};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::SourceId;

pub(super) const SPREADSHEETML_TRANSITIONAL: &[u8] =
    b"http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub(super) const SPREADSHEETML_STRICT: &[u8] = b"http://purl.oclc.org/ooxml/spreadsheetml/main";
pub(super) const DOCUMENT_RELATIONSHIPS_TRANSITIONAL: &[u8] =
    b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(super) const DOCUMENT_RELATIONSHIPS_STRICT: &[u8] =
    b"http://purl.oclc.org/ooxml/officeDocument/relationships";

#[derive(Debug)]
pub(super) struct XmlAttribute {
    local_name: Box<str>,
    namespace: Option<Box<[u8]>>,
    value: Box<str>,
}

#[derive(Debug)]
pub(super) struct XmlAttributes(Vec<XmlAttribute>);

impl XmlAttributes {
    pub(super) fn unqualified(&self, local_name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|attribute| {
                attribute.namespace.is_none() && attribute.local_name.as_ref() == local_name
            })
            .map(|attribute| attribute.value.as_ref())
    }

    pub(super) fn namespaced(&self, namespace: &[u8], local_name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|attribute| {
                attribute.namespace.as_deref() == Some(namespace)
                    && attribute.local_name.as_ref() == local_name
            })
            .map(|attribute| attribute.value.as_ref())
    }
}

#[derive(Debug)]
pub(super) struct XmlBudget {
    depth: u64,
    limits: ReadLimits,
    source: SourceId,
    invalid_code: XlsxErrorCode,
}

impl XmlBudget {
    pub(super) fn new(limits: ReadLimits, source: SourceId, invalid_code: XlsxErrorCode) -> Self {
        Self {
            depth: 0,
            limits,
            source,
            invalid_code,
        }
    }

    pub(super) fn start(&mut self) -> Result<u64, XlsxReadError> {
        let next = self.depth.saturating_add(1);
        if next > self.limits.max_xml_depth() {
            return Err(self.error(XlsxErrorCode::XmlDepthExceeded));
        }
        self.depth = next;
        Ok(next)
    }

    pub(super) fn empty(&self) -> Result<u64, XlsxReadError> {
        let next = self.depth.saturating_add(1);
        if next > self.limits.max_xml_depth() {
            return Err(self.error(XlsxErrorCode::XmlDepthExceeded));
        }
        Ok(next)
    }

    pub(super) fn end(&mut self) -> Result<u64, XlsxReadError> {
        let current = self.depth;
        self.depth = self
            .depth
            .checked_sub(1)
            .ok_or_else(|| self.error(self.invalid_code))?;
        Ok(current)
    }

    pub(super) fn finish(&self, saw_root: bool) -> Result<(), XlsxReadError> {
        if !saw_root || self.depth != 0 {
            return Err(self.error(self.invalid_code));
        }
        Ok(())
    }

    pub(super) const fn limits(&self) -> ReadLimits {
        self.limits
    }

    pub(super) const fn source_id(&self) -> &SourceId {
        &self.source
    }

    pub(super) fn error(&self, code: XlsxErrorCode) -> XlsxReadError {
        XlsxReadError::new(code).at_source(self.source.clone())
    }
}

pub(super) fn reader(bytes: &[u8]) -> NsReader<&[u8]> {
    let mut reader = NsReader::from_reader(bytes);
    let config = reader.config_mut();
    config.check_end_names = true;
    config.allow_unmatched_ends = false;
    config.expand_empty_elements = false;
    config.trim_text(false);
    reader
}

pub(super) fn read_attributes(
    element: &BytesStart<'_>,
    reader: &NsReader<&[u8]>,
    budget: &XmlBudget,
) -> Result<XmlAttributes, XlsxReadError> {
    let mut values = Vec::new();
    for (index, attribute) in element.attributes().with_checks(true).enumerate() {
        if index as u64 >= budget.limits().max_xml_attributes() {
            return Err(budget.error(XlsxErrorCode::XmlAttributesExceeded));
        }
        let attribute =
            attribute.map_err(|error| budget.error(budget.invalid_code).with_cause(error))?;
        if attribute.key.as_ref() == b"xmlns" || attribute.key.as_ref().starts_with(b"xmlns:") {
            continue;
        }
        let (namespace, local_name) = reader.resolver().resolve_attribute(attribute.key);
        let namespace = match namespace {
            ResolveResult::Unbound => None,
            ResolveResult::Bound(namespace) => Some(Box::<[u8]>::from(namespace.as_ref())),
            ResolveResult::Unknown(prefix) => {
                return Err(budget
                    .error(budget.invalid_code)
                    .with_detail(String::from_utf8_lossy(&prefix).into_owned()));
            }
        };
        let local_name = std::str::from_utf8(local_name.as_ref())
            .map_err(|error| budget.error(budget.invalid_code).with_cause(error))?;
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|error| budget.error(budget.invalid_code).with_cause(error))?;
        values.push(XmlAttribute {
            local_name: local_name.to_owned().into_boxed_str(),
            namespace,
            value: value.into_owned().into_boxed_str(),
        });
    }
    Ok(XmlAttributes(values))
}

pub(super) fn require_spreadsheet_element(
    is_spreadsheet: bool,
    local_name: &[u8],
    expected_local_name: &[u8],
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if local_name != expected_local_name || !is_spreadsheet {
        return Err(budget.error(budget.invalid_code));
    }
    Ok(())
}

pub(super) fn is_spreadsheet_element(
    reader: &NsReader<&[u8]>,
    name: QName<'_>,
    budget: &XmlBudget,
) -> Result<bool, XlsxReadError> {
    is_element_in_namespace(reader, name, SPREADSHEETML_TRANSITIONAL, budget)
}

pub(super) fn is_element_in_namespace(
    reader: &NsReader<&[u8]>,
    name: QName<'_>,
    expected_namespace: &[u8],
    budget: &XmlBudget,
) -> Result<bool, XlsxReadError> {
    match reader.resolver().resolve_element(name).0 {
        ResolveResult::Bound(namespace) => {
            let namespace = namespace.as_ref();
            Ok(namespace == expected_namespace
                || (expected_namespace == SPREADSHEETML_TRANSITIONAL
                    && namespace == SPREADSHEETML_STRICT))
        }
        ResolveResult::Unbound => Ok(false),
        ResolveResult::Unknown(prefix) => Err(budget
            .error(budget.invalid_code)
            .with_detail(String::from_utf8_lossy(&prefix).into_owned())),
    }
}

pub(super) fn decode_text(
    text: &BytesText<'_>,
    budget: &XmlBudget,
) -> Result<String, XlsxReadError> {
    text.xml_content(XmlVersion::Implicit1_0)
        .map(|value| value.into_owned())
        .map_err(|error| budget.error(budget.invalid_code).with_cause(error))
}

pub(super) fn decode_cdata(
    text: &BytesCData<'_>,
    budget: &XmlBudget,
) -> Result<String, XlsxReadError> {
    text.xml_content(XmlVersion::Implicit1_0)
        .map(|value| value.into_owned())
        .map_err(|error| budget.error(budget.invalid_code).with_cause(error))
}

pub(super) fn decode_reference(
    reference: &BytesRef<'_>,
    budget: &XmlBudget,
) -> Result<String, XlsxReadError> {
    if let Some(character) = reference
        .resolve_char_ref()
        .map_err(|error| budget.error(budget.invalid_code).with_cause(error))?
    {
        return Ok(character.to_string());
    }
    let name = reference
        .decode()
        .map_err(|error| budget.error(budget.invalid_code).with_cause(error))?;
    match name.as_ref() {
        "lt" => Ok("<".to_owned()),
        "gt" => Ok(">".to_owned()),
        "amp" => Ok("&".to_owned()),
        "apos" => Ok("'".to_owned()),
        "quot" => Ok("\"".to_owned()),
        _ => Err(budget
            .error(budget.invalid_code)
            .with_detail(name.into_owned())),
    }
}
