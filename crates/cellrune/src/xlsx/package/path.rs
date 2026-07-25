use super::super::error::detail;
use super::super::{XlsxErrorCode, XlsxReadError};
use crate::SourceId;

/// A normalized package-root-relative part path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::xlsx) struct PartPath(SourceId);

impl PartPath {
    pub(in crate::xlsx) fn from_archive_name(value: &[u8]) -> Result<Self, XlsxReadError> {
        let value = std::str::from_utf8(value).map_err(|error| {
            XlsxReadError::new(XlsxErrorCode::InvalidPartName)
                .with_detail(detail::ZIP_ENTRY_NAME_NOT_UTF8)
                .with_cause(error)
        })?;
        if value.starts_with('/') {
            return Err(
                XlsxReadError::new(XlsxErrorCode::InvalidPartName).with_detail(value.to_owned())
            );
        }
        normalize(value, &[], false, XlsxErrorCode::InvalidPartName)
    }

    pub(in crate::xlsx) fn from_content_type_override(value: &str) -> Result<Self, XlsxReadError> {
        if !value.starts_with('/') {
            return Err(XlsxReadError::new(XlsxErrorCode::InvalidContentTypes)
                .with_detail(detail::CONTENT_TYPE_PART_NOT_ABSOLUTE));
        }
        normalize(value, &[], true, XlsxErrorCode::InvalidPartName)
    }

    pub(in crate::xlsx) fn resolve_relationship(
        source: Option<&Self>,
        target: &str,
    ) -> Result<Self, XlsxReadError> {
        let mut base = Vec::new();
        if !target.starts_with('/')
            && let Some(source) = source
        {
            base.extend(source.as_str().split('/'));
            base.pop();
        }
        normalize(
            target,
            &base,
            true,
            XlsxErrorCode::InvalidRelationshipTarget,
        )
    }

    pub(in crate::xlsx) fn relationship_part(&self) -> Result<Self, XlsxReadError> {
        let Some((directory, file_name)) = self.as_str().rsplit_once('/') else {
            return Self::from_archive_name(format!("_rels/{}.rels", self.as_str()).as_bytes());
        };
        Self::from_archive_name(format!("{directory}/_rels/{file_name}.rels").as_bytes())
    }

    pub(in crate::xlsx) fn source_id(&self) -> SourceId {
        self.0.clone()
    }

    pub(in crate::xlsx) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(in crate::xlsx) fn extension(&self) -> Option<&str> {
        self.as_str()
            .rsplit_once('.')
            .map(|(_, extension)| extension)
    }
}

fn normalize(
    value: &str,
    base: &[&str],
    allow_parent: bool,
    error_code: XlsxErrorCode,
) -> Result<PartPath, XlsxReadError> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.contains(':')
    {
        return Err(XlsxReadError::new(error_code).with_detail(value.to_owned()));
    }

    let normalized = normalize_percent_encoding(value, error_code)?;
    let mut segments: Vec<&str> = base.to_vec();
    for segment in normalized.trim_start_matches('/').split('/') {
        match segment {
            "" | "." => {}
            ".." if allow_parent && !segments.is_empty() => {
                segments.pop();
            }
            ".." => {
                return Err(XlsxReadError::new(error_code).with_detail(value.to_owned()));
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return Err(XlsxReadError::new(error_code).with_detail(value.to_owned()));
    }
    let joined = segments.join("/");
    let source = SourceId::new(joined.clone()).map_err(|error| {
        XlsxReadError::new(error_code)
            .with_detail(joined)
            .with_cause(error)
    })?;
    Ok(PartPath(source))
}

fn normalize_percent_encoding(
    value: &str,
    error_code: XlsxErrorCode,
) -> Result<String, XlsxReadError> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(XlsxReadError::new(error_code).with_detail(value.to_owned()));
        }
        let high = hex_value(bytes[index + 1]);
        let low = hex_value(bytes[index + 2]);
        let (Some(high), Some(low)) = (high, low) else {
            return Err(XlsxReadError::new(error_code).with_detail(value.to_owned()));
        };
        let decoded = high * 16 + low;
        if decoded.is_ascii_alphanumeric() || matches!(decoded, b'-' | b'.' | b'_' | b'~') {
            output.push(decoded);
        } else {
            output.push(b'%');
            output.push(upper_hex(high));
            output.push(upper_hex(low));
        }
        index += 3;
    }
    String::from_utf8(output).map_err(|error| {
        XlsxReadError::new(error_code)
            .with_detail(value.to_owned())
            .with_cause(error)
    })
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

const fn upper_hex(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'A' + value - 10,
    }
}
