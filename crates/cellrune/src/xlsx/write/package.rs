use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use zip::CompressionMethod;
use zip::read::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

use super::{WriteLimits, WriteOptions, XlsxWriteError, XlsxWriteErrorCode};
use crate::InputHash;
use crate::xlsx::document::{XlsxDocument, XlsxDocumentKind};
use crate::xlsx::package::PartPath;

const LIMIT_MAX_ENTRIES: &str = "max_entries";
const LIMIT_MAX_ENTRY_UNCOMPRESSED_BYTES: &str = "max_entry_uncompressed_bytes";
const LIMIT_MAX_TOTAL_UNCOMPRESSED_BYTES: &str = "max_total_uncompressed_bytes";
const LIMIT_MAX_REWRITTEN_XML_BYTES: &str = "max_rewritten_xml_bytes";
const LIMIT_MAX_OUTPUT_ARCHIVE_BYTES: &str = "max_output_archive_bytes";
const LIMIT_MAX_TEMPORARY_STORAGE_BYTES: &str = "max_temporary_storage_bytes";
const DETAIL_DUPLICATE_SOURCE_INDEX: &str = "duplicate original ZIP entry index";
const DETAIL_DUPLICATE_PART: &str = "duplicate package part operation";
const DETAIL_UNKNOWN_REPLACEMENT_PART: &str = "replacement targets an unknown package part";
const DETAIL_UNKNOWN_REMOVAL_PART: &str = "removal targets an unknown package part";
const DETAIL_ADDITION_ALREADY_EXISTS: &str = "addition targets an existing package part";
const DETAIL_REPLACE_AND_REMOVE_PART: &str =
    "one package part cannot be replaced and removed in the same plan";
const DETAIL_SOURCE_PART_NOT_FOUND: &str = "source package part was not found";
const DETAIL_ARCHIVE_COMMENT_TOO_LONG: &str = "ZIP archive comment exceeds 65535 bytes";

/// Rebuilds an opened package by raw-copying every unchanged ZIP entry.
///
/// This foundation operation does not edit workbook semantics or materialize calculation results.
/// It is useful for preservation verification and produces a newly assembled XLSX or XLSM archive.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when the package plan is inconsistent or a configured write
/// resource limit is exceeded.
pub fn write_preserved_xlsx_bytes(
    document: &XlsxDocument,
    options: WriteOptions,
) -> Result<Vec<u8>, XlsxWriteError> {
    let source = document.preserved_package();
    let plan = PackageWritePlan::unchanged(source, options.limits())?;
    plan.write_to_vec(source)
}

#[derive(Clone)]
pub(crate) struct PreservedPackage {
    bytes: Arc<[u8]>,
    input_hash: InputHash,
    kind: XlsxDocumentKind,
}

impl PreservedPackage {
    pub(crate) const fn new(
        bytes: Arc<[u8]>,
        input_hash: InputHash,
        kind: XlsxDocumentKind,
    ) -> Self {
        Self {
            bytes,
            input_hash,
            kind,
        }
    }

    pub(crate) const fn input_hash(&self) -> InputHash {
        self.input_hash
    }

    pub(crate) const fn kind(&self) -> XlsxDocumentKind {
        self.kind
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(in crate::xlsx) fn read_part(&self, part: &PartPath) -> Result<Vec<u8>, XlsxWriteError> {
        let mut archive = ZipArchive::new(Cursor::new(self.bytes())).map_err(zip_read_error)?;
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).map_err(zip_read_error)?;
            if file.is_dir() {
                continue;
            }
            let candidate =
                PartPath::from_archive_name(file.name_raw()).map_err(read_plan_error)?;
            if &candidate != part {
                continue;
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|error| {
                XlsxWriteError::new(XlsxWriteErrorCode::Io)
                    .at_source(part.source_id())
                    .with_cause(error)
            })?;
            return Ok(bytes);
        }
        Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
            .with_detail(DETAIL_SOURCE_PART_NOT_FOUND)
            .at_source(part.source_id()))
    }
}

impl std::fmt::Debug for PreservedPackage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreservedPackage")
            .field("byte_length", &self.bytes.len())
            .field("input_hash", &self.input_hash)
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug, Clone)]
enum PackageWriteOperation {
    CopyOriginal {
        source_index: usize,
        source: crate::SourceId,
        part: Option<PartPath>,
        uncompressed_size: u64,
    },
    RewriteOriginal {
        source_index: usize,
        source: crate::SourceId,
        part: PartPath,
        bytes: Box<[u8]>,
    },
    AddNew {
        source: crate::SourceId,
        part: PartPath,
        bytes: Box<[u8]>,
    },
}

impl PackageWriteOperation {
    const fn source_index(&self) -> Option<usize> {
        match self {
            Self::CopyOriginal { source_index, .. }
            | Self::RewriteOriginal { source_index, .. } => Some(*source_index),
            Self::AddNew { .. } => None,
        }
    }

    const fn source(&self) -> &crate::SourceId {
        match self {
            Self::CopyOriginal { source, .. }
            | Self::RewriteOriginal { source, .. }
            | Self::AddNew { source, .. } => source,
        }
    }

    const fn part(&self) -> Option<&PartPath> {
        match self {
            Self::CopyOriginal { part, .. } => part.as_ref(),
            Self::RewriteOriginal { part, .. } | Self::AddNew { part, .. } => Some(part),
        }
    }

    const fn uncompressed_size(&self) -> u64 {
        match self {
            Self::CopyOriginal {
                uncompressed_size, ..
            } => *uncompressed_size,
            Self::RewriteOriginal { bytes, .. } | Self::AddNew { bytes, .. } => bytes.len() as u64,
        }
    }

    fn rewritten_size(&self) -> u64 {
        match self {
            Self::CopyOriginal { .. } => 0,
            Self::RewriteOriginal { bytes, .. } | Self::AddNew { bytes, .. } => bytes.len() as u64,
        }
    }
}

/// An immutable, validated package operation sequence.
#[derive(Debug, Clone)]
pub(crate) struct PackageWritePlan {
    source_hash: InputHash,
    output_kind: XlsxDocumentKind,
    operations: Vec<PackageWriteOperation>,
    archive_comment: Box<[u8]>,
    limits: WriteLimits,
}

impl PackageWritePlan {
    pub(crate) fn unchanged(
        source: &PreservedPackage,
        limits: WriteLimits,
    ) -> Result<Self, XlsxWriteError> {
        let mut archive = ZipArchive::new(Cursor::new(source.bytes())).map_err(zip_read_error)?;
        let archive_comment = archive.comment().to_vec().into_boxed_slice();
        let mut operations = Vec::with_capacity(archive.len());
        for source_index in 0..archive.len() {
            let file = archive.by_index(source_index).map_err(zip_read_error)?;
            let normalized_name = file
                .name_raw()
                .strip_suffix(b"/")
                .unwrap_or(file.name_raw());
            let normalized =
                PartPath::from_archive_name(normalized_name).map_err(read_plan_error)?;
            let source_id = normalized.source_id();
            let part = (!file.is_dir()).then_some(normalized);
            operations.push(PackageWriteOperation::CopyOriginal {
                source_index,
                source: source_id,
                part,
                uncompressed_size: file.size(),
            });
        }
        let plan = Self {
            source_hash: source.input_hash(),
            output_kind: source.kind(),
            operations,
            archive_comment,
            limits,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub(crate) fn modified(
        source: &PreservedPackage,
        replacements: BTreeMap<PartPath, Vec<u8>>,
        removals: &BTreeSet<PartPath>,
        limits: WriteLimits,
    ) -> Result<Self, XlsxWriteError> {
        Self::modified_with_additions(source, replacements, BTreeMap::new(), removals, limits)
    }

    pub(crate) fn modified_with_additions(
        source: &PreservedPackage,
        mut replacements: BTreeMap<PartPath, Vec<u8>>,
        additions: BTreeMap<PartPath, Vec<u8>>,
        removals: &BTreeSet<PartPath>,
        limits: WriteLimits,
    ) -> Result<Self, XlsxWriteError> {
        if let Some(conflict) = replacements.keys().find(|part| removals.contains(*part)) {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::ConflictingPartOperation)
                    .with_detail(DETAIL_REPLACE_AND_REMOVE_PART)
                    .at_source(conflict.source_id()),
            );
        }
        let mut archive = ZipArchive::new(Cursor::new(source.bytes())).map_err(zip_read_error)?;
        let archive_comment = archive.comment().to_vec().into_boxed_slice();
        let mut operations = Vec::with_capacity(archive.len().saturating_add(additions.len()));
        let mut removed = BTreeSet::new();
        let mut existing_parts = BTreeSet::new();
        for source_index in 0..archive.len() {
            let file = archive.by_index(source_index).map_err(zip_read_error)?;
            let normalized_name = file
                .name_raw()
                .strip_suffix(b"/")
                .unwrap_or(file.name_raw());
            let normalized =
                PartPath::from_archive_name(normalized_name).map_err(read_plan_error)?;
            let source_id = normalized.source_id();
            if !file.is_dir() {
                existing_parts.insert(normalized.clone());
            }
            if !file.is_dir() && removals.contains(&normalized) {
                removed.insert(normalized);
                continue;
            }
            let part = (!file.is_dir()).then_some(normalized);
            if let Some((part, bytes)) = part
                .as_ref()
                .and_then(|part| replacements.remove_entry(part))
            {
                operations.push(PackageWriteOperation::RewriteOriginal {
                    source_index,
                    source: source_id,
                    part,
                    bytes: bytes.into_boxed_slice(),
                });
            } else {
                operations.push(PackageWriteOperation::CopyOriginal {
                    source_index,
                    source: source_id,
                    part,
                    uncompressed_size: file.size(),
                });
            }
        }
        if let Some((part, _)) = replacements.first_key_value() {
            return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                .with_detail(DETAIL_UNKNOWN_REPLACEMENT_PART)
                .at_source(part.source_id()));
        }
        if let Some(part) = removals.iter().find(|part| !removed.contains(*part)) {
            return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                .with_detail(DETAIL_UNKNOWN_REMOVAL_PART)
                .at_source(part.source_id()));
        }
        if let Some((part, _)) = additions
            .iter()
            .find(|(part, _)| existing_parts.contains(*part))
        {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::ConflictingPartOperation)
                    .with_detail(DETAIL_ADDITION_ALREADY_EXISTS)
                    .at_source(part.source_id()),
            );
        }
        operations.extend(additions.into_iter().map(|(part, bytes)| {
            PackageWriteOperation::AddNew {
                source: part.source_id(),
                part,
                bytes: bytes.into_boxed_slice(),
            }
        }));
        let plan = Self {
            source_hash: source.input_hash(),
            output_kind: source.kind(),
            operations,
            archive_comment,
            limits,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub(crate) fn write_to_vec(
        &self,
        source: &PreservedPackage,
    ) -> Result<Vec<u8>, XlsxWriteError> {
        self.validate()?;
        if self.source_hash != source.input_hash() {
            return Err(XlsxWriteError::new(
                XlsxWriteErrorCode::SourceIdentityMismatch,
            ));
        }
        if self.output_kind != source.kind() {
            return Err(XlsxWriteError::new(XlsxWriteErrorCode::OutputKindMismatch));
        }

        let mut input = ZipArchive::new(Cursor::new(source.bytes())).map_err(zip_read_error)?;
        let output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(output);
        if self.archive_comment.len() > usize::from(u16::MAX) {
            return Err(XlsxWriteError::new(XlsxWriteErrorCode::Io)
                .with_detail(DETAIL_ARCHIVE_COMMENT_TOO_LONG));
        }
        // zip 7.2 through 8.4 returns `()`, while 8.5+ returns a `Result` only when the comment
        // exceeds the length checked above. Discarding the version-dependent success value keeps
        // the write path compatible without weakening the archive boundary.
        let _ = writer.set_raw_comment(self.archive_comment.clone());
        for operation in &self.operations {
            match operation {
                PackageWriteOperation::CopyOriginal {
                    source_index,
                    source,
                    ..
                } => {
                    let file = input
                        .by_index(*source_index)
                        .map_err(|error| zip_read_error(error).at_source(source.clone()))?;
                    writer
                        .raw_copy_file(file)
                        .map_err(|error| zip_write_error(error).at_source(source.clone()))?;
                }
                PackageWriteOperation::RewriteOriginal {
                    source_index,
                    source,
                    bytes,
                    ..
                } => {
                    let file = input
                        .by_index(*source_index)
                        .map_err(|error| zip_read_error(error).at_source(source.clone()))?;
                    let name = file.name().to_owned();
                    let file_options = file.options();
                    drop(file);
                    writer
                        .start_file(name, file_options)
                        .map_err(|error| zip_write_error(error).at_source(source.clone()))?;
                    writer.write_all(bytes).map_err(|error| {
                        XlsxWriteError::new(XlsxWriteErrorCode::Io)
                            .at_source(source.clone())
                            .with_cause(error)
                    })?;
                }
                PackageWriteOperation::AddNew {
                    source,
                    part,
                    bytes,
                } => {
                    writer
                        .start_file(
                            part.as_str(),
                            SimpleFileOptions::default()
                                .compression_method(CompressionMethod::Stored),
                        )
                        .map_err(|error| zip_write_error(error).at_source(source.clone()))?;
                    writer.write_all(bytes).map_err(|error| {
                        XlsxWriteError::new(XlsxWriteErrorCode::Io)
                            .at_source(source.clone())
                            .with_cause(error)
                    })?;
                }
            }
        }
        let bytes = writer.finish().map_err(zip_write_error)?.into_inner();
        enforce_size_limit(
            LIMIT_MAX_OUTPUT_ARCHIVE_BYTES,
            bytes.len() as u64,
            self.limits.max_output_archive_bytes(),
        )?;
        enforce_size_limit(
            LIMIT_MAX_TEMPORARY_STORAGE_BYTES,
            bytes.len() as u64,
            self.limits.max_temporary_storage_bytes(),
        )?;
        Ok(bytes)
    }

    fn validate(&self) -> Result<(), XlsxWriteError> {
        enforce_size_limit(
            LIMIT_MAX_ENTRIES,
            self.operations.len() as u64,
            self.limits.max_entries(),
        )?;
        let mut indexes = BTreeSet::new();
        let mut parts = BTreeSet::new();
        let mut total_uncompressed = 0_u128;
        let mut total_rewritten = 0_u128;
        for operation in &self.operations {
            if let Some(source_index) = operation.source_index()
                && !indexes.insert(source_index)
            {
                return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                    .with_detail(DETAIL_DUPLICATE_SOURCE_INDEX));
            }
            if let Some(part) = operation.part()
                && !parts.insert(part.clone())
            {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::ConflictingPartOperation)
                        .with_detail(DETAIL_DUPLICATE_PART)
                        .at_source(operation.source().clone()),
                );
            }
            enforce_size_limit(
                LIMIT_MAX_ENTRY_UNCOMPRESSED_BYTES,
                operation.uncompressed_size(),
                self.limits.max_entry_uncompressed_bytes(),
            )?;
            total_uncompressed += u128::from(operation.uncompressed_size());
            if total_uncompressed > u128::from(self.limits.max_total_uncompressed_bytes()) {
                return Err(resource_limit_error(
                    LIMIT_MAX_TOTAL_UNCOMPRESSED_BYTES,
                    total_uncompressed,
                    u128::from(self.limits.max_total_uncompressed_bytes()),
                ));
            }
            total_rewritten += u128::from(operation.rewritten_size());
            if total_rewritten > u128::from(self.limits.max_rewritten_xml_bytes()) {
                return Err(resource_limit_error(
                    LIMIT_MAX_REWRITTEN_XML_BYTES,
                    total_rewritten,
                    u128::from(self.limits.max_rewritten_xml_bytes()),
                ));
            }
        }
        Ok(())
    }
}

fn enforce_size_limit(name: &'static str, actual: u64, maximum: u64) -> Result<(), XlsxWriteError> {
    if actual > maximum {
        return Err(resource_limit_error(
            name,
            u128::from(actual),
            u128::from(maximum),
        ));
    }
    Ok(())
}

fn resource_limit_error(name: &'static str, actual: u128, maximum: u128) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!("{name}: {actual} > {maximum}"))
}

fn read_plan_error(error: crate::XlsxReadError) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_cause(error)
}

fn zip_read_error(error: zip::result::ZipError) -> XlsxWriteError {
    match error {
        zip::result::ZipError::Io(error) => {
            XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
        }
        other => XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_cause(other),
    }
}

fn zip_write_error(error: zip::result::ZipError) -> XlsxWriteError {
    match error {
        zip::result::ZipError::Io(error) => {
            XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
        }
        other => XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_cause(other),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{Cursor, Read, Write};
    use std::sync::Arc;

    use zip::CompressionMethod;
    use zip::read::ZipArchive;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::{
        DETAIL_DUPLICATE_SOURCE_INDEX, PackageWritePlan, PreservedPackage, XlsxDocumentKind,
    };
    use crate::xlsx::package::PartPath;
    use crate::{InputHash, WriteLimits, XlsxWriteErrorCode};

    #[test]
    fn unchanged_plan_raw_copies_every_entry_and_preserves_content() {
        let source_bytes = fixture_archive();
        let source = preserved(&source_bytes);
        let plan =
            PackageWritePlan::unchanged(&source, WriteLimits::default()).expect("identity plan");
        let output = plan.write_to_vec(&source).expect("raw-copy output");

        let mut source_archive =
            ZipArchive::new(Cursor::new(&source_bytes)).expect("source archive");
        let mut output_archive = ZipArchive::new(Cursor::new(&output)).expect("output archive");
        assert_eq!(source_archive.len(), output_archive.len());
        assert_eq!(source_archive.comment(), output_archive.comment());

        for index in 0..source_archive.len() {
            let (source_name, source_method, source_crc, mut source_contents) = {
                let mut file = source_archive.by_index(index).expect("source entry");
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).expect("source contents");
                (
                    file.name().to_owned(),
                    file.compression(),
                    file.crc32(),
                    contents,
                )
            };
            let (output_name, output_method, output_crc, mut output_contents) = {
                let mut file = output_archive.by_index(index).expect("output entry");
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).expect("output contents");
                (
                    file.name().to_owned(),
                    file.compression(),
                    file.crc32(),
                    contents,
                )
            };
            assert_eq!(source_name, output_name);
            assert_eq!(source_method, output_method);
            assert_eq!(source_crc, output_crc);
            assert_eq!(source_contents, output_contents);
            source_contents.clear();
            output_contents.clear();
        }

        assert_eq!(
            compressed_payload(&source_bytes, 1),
            compressed_payload(&output, 1),
            "deflated bytes must be copied without recompression"
        );
    }

    #[test]
    fn modified_plan_rewrites_declared_parts_and_raw_copies_every_other_entry() {
        let source_bytes = fixture_archive();
        let source = preserved(&source_bytes);
        let stored = PartPath::from_archive_name(b"stored.xml").expect("valid replacement part");
        let mut replacements = BTreeMap::new();
        replacements.insert(stored, b"<stored changed=\"yes\"/>".to_vec());
        let plan = PackageWritePlan::modified(
            &source,
            replacements,
            &BTreeSet::new(),
            WriteLimits::default(),
        )
        .expect("modified plan");
        let output = plan.write_to_vec(&source).expect("modified output");

        let mut archive = ZipArchive::new(Cursor::new(&output)).expect("output archive");
        let mut stored_output = String::new();
        archive
            .by_name("stored.xml")
            .expect("stored part")
            .read_to_string(&mut stored_output)
            .expect("stored XML");
        assert_eq!(stored_output, r#"<stored changed="yes"/>"#);
        assert_eq!(
            compressed_payload(&source_bytes, 1),
            compressed_payload(&output, 1),
            "unchanged deflated payload must be raw-copied"
        );
    }

    #[test]
    fn modified_plan_removes_only_declared_existing_parts() {
        let source_bytes = fixture_archive();
        let source = preserved(&source_bytes);
        let removed = PartPath::from_archive_name(b"stored.xml").expect("valid removal part");
        let plan = PackageWritePlan::modified(
            &source,
            BTreeMap::new(),
            &BTreeSet::from([removed.clone()]),
            WriteLimits::default(),
        )
        .expect("removal plan");
        let output = plan.write_to_vec(&source).expect("removal output");
        let mut archive = ZipArchive::new(Cursor::new(&output)).expect("output archive");
        assert!(archive.by_name("stored.xml").is_err());
        assert!(archive.by_name("deflated.xml").is_ok());

        let unknown = PartPath::from_archive_name(b"missing.xml").expect("valid missing part");
        let error = PackageWritePlan::modified(
            &source,
            BTreeMap::new(),
            &BTreeSet::from([unknown]),
            WriteLimits::default(),
        )
        .expect_err("unknown removal must fail before writing");
        assert_eq!(error.code(), XlsxWriteErrorCode::InvalidPackagePlan);

        let conflict = PackageWritePlan::modified(
            &source,
            BTreeMap::from([(removed.clone(), b"replacement".to_vec())]),
            &BTreeSet::from([removed]),
            WriteLimits::default(),
        )
        .expect_err("one part cannot be replaced and removed");
        assert_eq!(
            conflict.code(),
            XlsxWriteErrorCode::ConflictingPartOperation
        );
    }

    #[test]
    fn plan_rejects_a_different_source_identity() {
        let first_bytes = fixture_archive();
        let first = preserved(&first_bytes);
        let plan =
            PackageWritePlan::unchanged(&first, WriteLimits::default()).expect("identity plan");

        let mut second_bytes = first_bytes.clone();
        second_bytes.push(0);
        let second = preserved(&second_bytes);
        let error = plan
            .write_to_vec(&second)
            .expect_err("identity mismatch must fail");
        assert_eq!(error.code(), XlsxWriteErrorCode::SourceIdentityMismatch);
    }

    #[test]
    fn malformed_plan_is_rejected_before_archive_generation() {
        let bytes = fixture_archive();
        let source = preserved(&bytes);
        let mut plan =
            PackageWritePlan::unchanged(&source, WriteLimits::default()).expect("identity plan");
        plan.operations.push(plan.operations[0].clone());

        let error = plan
            .write_to_vec(&source)
            .expect_err("duplicate operation must fail");
        assert_eq!(error.code(), XlsxWriteErrorCode::InvalidPackagePlan);
        assert_eq!(error.detail(), Some(DETAIL_DUPLICATE_SOURCE_INDEX));
    }

    #[test]
    fn entry_limit_mutation_is_enforced_before_writing() {
        let bytes = fixture_archive();
        let source = preserved(&bytes);
        let limits = WriteLimits::default()
            .with_max_entries(1)
            .expect("positive limit");
        let error =
            PackageWritePlan::unchanged(&source, limits).expect_err("two entries exceed limit");
        assert_eq!(error.code(), XlsxWriteErrorCode::ResourceLimitExceeded);
        assert!(
            error
                .detail()
                .expect("resource detail")
                .starts_with("max_entries:")
        );
    }

    #[test]
    fn total_uncompressed_budget_accepts_only_the_exact_or_larger_boundary() {
        let bytes = fixture_archive();
        let source = preserved(&bytes);
        let baseline =
            PackageWritePlan::unchanged(&source, WriteLimits::default()).expect("baseline plan");
        let total = baseline
            .operations
            .iter()
            .map(|operation| operation.uncompressed_size())
            .sum::<u64>();

        let exact_limits = WriteLimits::default()
            .with_max_total_uncompressed_bytes(total)
            .expect("positive exact limit");
        PackageWritePlan::unchanged(&source, exact_limits).expect("exact total must be accepted");

        let below_limits = WriteLimits::default()
            .with_max_total_uncompressed_bytes(total - 1)
            .expect("positive lower limit");
        let error = PackageWritePlan::unchanged(&source, below_limits)
            .expect_err("combined entries must exceed the lower limit");
        assert_eq!(error.code(), XlsxWriteErrorCode::ResourceLimitExceeded);
    }

    #[test]
    fn output_archive_budget_accepts_the_exact_boundary_and_rejects_one_less() {
        let bytes = fixture_archive();
        let source = preserved(&bytes);
        let baseline =
            PackageWritePlan::unchanged(&source, WriteLimits::default()).expect("baseline plan");
        let output = baseline.write_to_vec(&source).expect("baseline output");
        let output_size = u64::try_from(output.len()).expect("output size");

        let exact_limits = WriteLimits::default()
            .with_max_output_archive_bytes(output_size)
            .expect("positive exact limit");
        let exact_plan =
            PackageWritePlan::unchanged(&source, exact_limits).expect("exact output plan");
        assert_eq!(
            exact_plan
                .write_to_vec(&source)
                .expect("exact output boundary")
                .len(),
            output.len()
        );

        let below_limits = WriteLimits::default()
            .with_max_output_archive_bytes(output_size - 1)
            .expect("positive lower limit");
        let below_plan =
            PackageWritePlan::unchanged(&source, below_limits).expect("lower output plan");
        let error = below_plan
            .write_to_vec(&source)
            .expect_err("output must exceed the lower limit");
        assert_eq!(error.code(), XlsxWriteErrorCode::ResourceLimitExceeded);
    }

    fn preserved(bytes: &[u8]) -> PreservedPackage {
        let bytes: Arc<[u8]> = Arc::from(bytes.to_vec());
        PreservedPackage::new(
            Arc::clone(&bytes),
            InputHash::for_bytes(&bytes),
            XlsxDocumentKind::Xlsx,
        )
    }

    fn fixture_archive() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            let _ = writer.set_comment("cellrune");
            writer
                .start_file(
                    "stored.xml",
                    SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
                )
                .expect("stored entry");
            writer.write_all(b"<stored/>").expect("stored contents");
            writer
                .start_file(
                    "deflated.xml",
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )
                .expect("deflated entry");
            writer
                .write_all(b"<deflated>repeated repeated repeated</deflated>")
                .expect("deflated contents");
            writer
                .add_directory("xl/", SimpleFileOptions::default())
                .expect("directory entry");
            writer.finish().expect("finish archive");
        }
        output.into_inner()
    }

    fn compressed_payload(bytes: &[u8], index: usize) -> Vec<u8> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("archive");
        let file = archive.by_index(index).expect("entry");
        let start = usize::try_from(DataStartResult::expect_data_start(file.data_start()))
            .expect("data start");
        let length = usize::try_from(file.compressed_size()).expect("compressed size");
        bytes[start..start + length].to_vec()
    }

    trait DataStartResult {
        fn expect_data_start(self) -> u64;
    }

    impl DataStartResult for u64 {
        fn expect_data_start(self) -> u64 {
            self
        }
    }

    impl DataStartResult for Option<u64> {
        fn expect_data_start(self) -> u64 {
            self.expect("data start must be available")
        }
    }
}
