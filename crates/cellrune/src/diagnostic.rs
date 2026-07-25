use crate::{CellAddress, SheetId, ValidationError};
use sha2::{Digest, Sha256};

/// A stable machine-readable diagnostic code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticCode(Box<str>);

impl DiagnosticCode {
    /// Accepts lowercase dotted identifiers such as `xlsx.external_relationship`.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::DiagnosticCodeInvalid`] when the value does not follow the
    /// stable diagnostic-code grammar.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if !is_valid_diagnostic_code(&value) {
            return Err(ValidationError::DiagnosticCodeInvalid);
        }
        Ok(Self(value.into_boxed_str()))
    }

    /// Returns the stable code string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Severity independent of Excel cell errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    /// Informational compatibility metadata.
    Info,
    /// A supported read with a compatibility caveat.
    Warning,
    /// A capability or validation failure.
    Error,
}

/// An opaque identifier for a source unit such as a package part.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(Box<str>);

impl SourceId {
    /// Validates a non-empty source identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::SourceIdEmpty`] when the identifier is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ValidationError::SourceIdEmpty);
        }
        Ok(Self(value.into_boxed_str()))
    }

    /// Returns the source identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A source-linked position with invariants enforced by dedicated constructors.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    source: SourceId,
    sheet_id: Option<SheetId>,
    cell: Option<CellAddress>,
    byte_offset: Option<u64>,
}

impl SourceLocation {
    /// Locates the source as a whole.
    pub const fn source(source: SourceId) -> Self {
        Self {
            source,
            sheet_id: None,
            cell: None,
            byte_offset: None,
        }
    }

    /// Locates a sheet within a source.
    pub const fn sheet(source: SourceId, sheet_id: SheetId) -> Self {
        Self {
            source,
            sheet_id: Some(sheet_id),
            cell: None,
            byte_offset: None,
        }
    }

    /// Locates a cell within a source and sheet.
    pub const fn cell(source: SourceId, sheet_id: SheetId, cell: CellAddress) -> Self {
        Self {
            source,
            sheet_id: Some(sheet_id),
            cell: Some(cell),
            byte_offset: None,
        }
    }

    /// Adds a zero-based byte offset within the source unit.
    pub const fn with_byte_offset(mut self, byte_offset: u64) -> Self {
        self.byte_offset = Some(byte_offset);
        self
    }

    /// Returns the source identifier.
    pub const fn source_id(&self) -> &SourceId {
        &self.source
    }

    /// Returns the sheet ID, when present.
    pub const fn sheet_id(&self) -> Option<SheetId> {
        self.sheet_id
    }

    /// Returns the cell address, when present.
    pub const fn cell_address(&self) -> Option<CellAddress> {
        self.cell
    }

    /// Returns the source byte offset, when present.
    pub const fn byte_offset(&self) -> Option<u64> {
        self.byte_offset
    }
}

/// A compatibility or capability diagnostic, separate from Excel values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
    message: Box<str>,
    location: Option<SourceLocation>,
}

impl Diagnostic {
    /// Creates a diagnostic with a non-empty human-readable message.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::DiagnosticMessageEmpty`] when `message` is empty.
    pub fn new(
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
        location: Option<SourceLocation>,
    ) -> Result<Self, ValidationError> {
        let message = message.into();
        if message.is_empty() {
            return Err(ValidationError::DiagnosticMessageEmpty);
        }
        Ok(Self {
            code,
            severity,
            message: message.into_boxed_str(),
            location,
        })
    }

    /// Returns the stable code.
    pub const fn code(&self) -> &DiagnosticCode {
        &self.code
    }

    /// Returns the severity.
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    /// Returns the human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the source location, when present.
    pub const fn location(&self) -> Option<&SourceLocation> {
        self.location.as_ref()
    }
}

/// The component that produced a snapshot or calculation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderIdentity {
    name: Box<str>,
    version: Box<str>,
}

impl ProviderIdentity {
    /// Validates provider name and version strings.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when either string is empty.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ValidationError::ProviderNameEmpty);
        }
        let version = version.into();
        if version.is_empty() {
            return Err(ValidationError::ProviderVersionEmpty);
        }
        Ok(Self {
            name: name.into_boxed_str(),
            version: version.into_boxed_str(),
        })
    }

    /// Returns the provider name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the provider version.
    pub fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn calculator() -> Self {
        Self {
            name: format!("{}.calculator", env!("CARGO_PKG_NAME")).into_boxed_str(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub(crate) fn writer() -> Self {
        Self {
            name: format!("{}.writer", env!("CARGO_PKG_NAME")).into_boxed_str(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

/// A SHA-256 digest associated with an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputHash([u8; 32]);

impl InputHash {
    /// Constructs a digest from its 32 bytes.
    pub const fn sha256(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the SHA-256 bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn for_bytes(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        Self(digest.into())
    }
}

/// Deterministic producer and input identity metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Provenance {
    provider: ProviderIdentity,
    input_hash: Option<InputHash>,
}

impl Provenance {
    /// Constructs provenance from validated parts.
    pub const fn new(provider: ProviderIdentity, input_hash: Option<InputHash>) -> Self {
        Self {
            provider,
            input_hash,
        }
    }

    /// Returns the producing component.
    pub const fn provider(&self) -> &ProviderIdentity {
        &self.provider
    }

    /// Returns the input digest, when known.
    pub const fn input_hash(&self) -> Option<InputHash> {
        self.input_hash
    }
}

fn is_valid_diagnostic_code(value: &str) -> bool {
    let mut segments = value.split('.');
    let mut count = 0;
    for segment in &mut segments {
        count += 1;
        let mut characters = segment.chars();
        if !matches!(characters.next(), Some('a'..='z')) {
            return false;
        }
        if !characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        }) {
            return false;
        }
    }
    count >= 2
}

#[cfg(test)]
mod tests {
    use super::InputHash;

    #[test]
    fn input_hash_uses_sha256_over_the_exact_input_bytes() {
        let hash = InputHash::for_bytes(b"abc");
        assert_eq!(
            hash.as_bytes(),
            &[
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }
}
