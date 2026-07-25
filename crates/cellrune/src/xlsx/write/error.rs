use std::error::Error;
use std::fmt;

use crate::SourceId;

const MESSAGE_IO: &str = "failed to write XLSX output";
const MESSAGE_INVALID_PACKAGE_PLAN: &str = "invalid XLSX package write plan";
const MESSAGE_SOURCE_IDENTITY_MISMATCH: &str =
    "write plan does not belong to the supplied source package";
const MESSAGE_STALE_SEMANTIC_REVISION: &str =
    "calculation was produced for an older workbook revision";
const MESSAGE_INCOMPLETE_CALCULATION: &str =
    "calculation is incomplete under the selected write policy";
const MESSAGE_UNSUPPORTED_RESULT_MATERIALIZATION: &str =
    "calculation result cannot be represented by the XLSX writer";
const MESSAGE_OUTPUT_KIND_MISMATCH: &str =
    "output extension or package kind is incompatible with preserved content";
const MESSAGE_UNSUPPORTED_PRESERVATION: &str =
    "required package content cannot be preserved safely";
const MESSAGE_CONFLICTING_PART_OPERATION: &str =
    "multiple write operations target the same package part";
const MESSAGE_DANGLING_RELATIONSHIP: &str =
    "generated package relationship does not resolve to a valid target";
const MESSAGE_INVALID_GENERATED_XML: &str = "generated XLSX XML is invalid";
const MESSAGE_DESTINATION_EXISTS: &str = "destination already exists";
const MESSAGE_ATOMIC_REPLACE_FAILED: &str = "atomic destination replacement failed";
const MESSAGE_RESOURCE_LIMIT_EXCEEDED: &str = "XLSX write resource limit exceeded";
const MESSAGE_OUTPUT_VERIFICATION_FAILED: &str = "written XLSX output failed verification";

/// Stable machine-readable failure codes for XLSX writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum XlsxWriteErrorCode {
    /// The output sink or filesystem operation failed.
    Io,
    /// The immutable package plan is internally inconsistent.
    InvalidPackagePlan,
    /// A write plan or calculation belongs to a different input package.
    SourceIdentityMismatch,
    /// A calculation predates the current semantic workbook revision.
    StaleSemanticRevision,
    /// Strict output was requested but one or more required results are unavailable.
    IncompleteCalculation,
    /// A typed calculation result cannot be materialized safely.
    UnsupportedResultMaterialization,
    /// The requested output kind conflicts with the package content.
    OutputKindMismatch,
    /// Required package content cannot be preserved without loss.
    UnsupportedPreservation,
    /// More than one operation targets the same package part.
    ConflictingPartOperation,
    /// A generated relationship has no valid target.
    DanglingRelationship,
    /// Generated XML does not satisfy the writer's own validation contract.
    InvalidGeneratedXml,
    /// The destination exists and replacement was not explicitly enabled.
    DestinationExists,
    /// Explicit atomic replacement could not be completed.
    AtomicReplaceFailed,
    /// A configured write resource budget was exceeded.
    ResourceLimitExceeded,
    /// Reopening or validating the completed output failed.
    OutputVerificationFailed,
}

impl XlsxWriteErrorCode {
    /// Returns the stable dotted identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "xlsx.write.io",
            Self::InvalidPackagePlan => "xlsx.write.invalid_package_plan",
            Self::SourceIdentityMismatch => "xlsx.write.source_identity_mismatch",
            Self::StaleSemanticRevision => "xlsx.write.stale_semantic_revision",
            Self::IncompleteCalculation => "xlsx.write.incomplete_calculation",
            Self::UnsupportedResultMaterialization => {
                "xlsx.write.unsupported_result_materialization"
            }
            Self::OutputKindMismatch => "xlsx.write.output_kind_mismatch",
            Self::UnsupportedPreservation => "xlsx.write.unsupported_preservation",
            Self::ConflictingPartOperation => "xlsx.write.conflicting_part_operation",
            Self::DanglingRelationship => "xlsx.write.dangling_relationship",
            Self::InvalidGeneratedXml => "xlsx.write.invalid_generated_xml",
            Self::DestinationExists => "xlsx.write.destination_exists",
            Self::AtomicReplaceFailed => "xlsx.write.atomic_replace_failed",
            Self::ResourceLimitExceeded => "xlsx.write.resource_limit_exceeded",
            Self::OutputVerificationFailed => "xlsx.write.output_verification_failed",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::Io => MESSAGE_IO,
            Self::InvalidPackagePlan => MESSAGE_INVALID_PACKAGE_PLAN,
            Self::SourceIdentityMismatch => MESSAGE_SOURCE_IDENTITY_MISMATCH,
            Self::StaleSemanticRevision => MESSAGE_STALE_SEMANTIC_REVISION,
            Self::IncompleteCalculation => MESSAGE_INCOMPLETE_CALCULATION,
            Self::UnsupportedResultMaterialization => MESSAGE_UNSUPPORTED_RESULT_MATERIALIZATION,
            Self::OutputKindMismatch => MESSAGE_OUTPUT_KIND_MISMATCH,
            Self::UnsupportedPreservation => MESSAGE_UNSUPPORTED_PRESERVATION,
            Self::ConflictingPartOperation => MESSAGE_CONFLICTING_PART_OPERATION,
            Self::DanglingRelationship => MESSAGE_DANGLING_RELATIONSHIP,
            Self::InvalidGeneratedXml => MESSAGE_INVALID_GENERATED_XML,
            Self::DestinationExists => MESSAGE_DESTINATION_EXISTS,
            Self::AtomicReplaceFailed => MESSAGE_ATOMIC_REPLACE_FAILED,
            Self::ResourceLimitExceeded => MESSAGE_RESOURCE_LIMIT_EXCEEDED,
            Self::OutputVerificationFailed => MESSAGE_OUTPUT_VERIFICATION_FAILED,
        }
    }
}

/// A source-linked XLSX write failure with a stable error code.
#[derive(Debug)]
pub struct XlsxWriteError {
    code: XlsxWriteErrorCode,
    detail: Option<Box<str>>,
    source_id: Option<SourceId>,
    cause: Option<Box<dyn Error + Send + Sync>>,
}

impl XlsxWriteError {
    pub(crate) const fn new(code: XlsxWriteErrorCode) -> Self {
        Self {
            code,
            detail: None,
            source_id: None,
            cause: None,
        }
    }

    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into().into_boxed_str());
        self
    }

    pub(crate) fn at_source(mut self, source_id: SourceId) -> Self {
        self.source_id = Some(source_id);
        self
    }

    pub(crate) fn with_cause(mut self, cause: impl Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// Returns the stable error code.
    pub const fn code(&self) -> XlsxWriteErrorCode {
        self.code
    }

    /// Returns the package source identifier, when known.
    pub const fn source_id(&self) -> Option<&SourceId> {
        self.source_id.as_ref()
    }

    /// Returns source-specific context that supplements the stable error code.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for XlsxWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.message())?;
        if let Some(detail) = &self.detail {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for XlsxWriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn Error + 'static))
    }
}
