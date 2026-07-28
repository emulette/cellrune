mod path;
pub(crate) mod relationship_type;
mod summary;
mod xml;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};

use zip::CompressionMethod;
use zip::read::ZipArchive;

pub(super) use self::path::PartPath;
pub use self::summary::PackageSummary;
use self::xml::{ContentTypes, Relationship, RelationshipTarget};
use super::error::detail;
use super::{ReadLimits, ReadOptions, XlsxErrorCode, XlsxReadError};

const CONTENT_TYPES_PART: &[u8] = b"[Content_Types].xml";
const ROOT_RELATIONSHIPS_PART: &[u8] = b"_rels/.rels";

const CONTENT_WORKBOOK: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const CONTENT_WORKBOOK_MACRO_ENABLED: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";
const CONTENT_WORKSHEET: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CONTENT_STYLES: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const CONTENT_SHARED_STRINGS: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CONTENT_SHEET_METADATA: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml";
const CONTENT_TABLE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkbookPackageKind {
    Xlsx,
    Xlsm,
}

pub(super) struct OpenedPackage<R: Read + Seek> {
    archive: ZipArchive<R>,
    entries: BTreeMap<PartPath, usize>,
    content_types: ContentTypes,
    limits: ReadLimits,
    archive_bytes: u64,
    /// Uncompressed bytes actually produced so far, charged against
    /// [`ReadLimits::max_total_uncompressed_bytes`].
    uncompressed_spent: u64,
    workbook_part: PartPath,
    worksheet_parts: BTreeMap<Box<str>, PartPath>,
    styles_part: Option<PartPath>,
    shared_strings_part: Option<PartPath>,
    metadata_part: Option<PartPath>,
    external_relationship_count: usize,
    has_external_links: bool,
    has_macros: bool,
    workbook_kind: WorkbookPackageKind,
}

struct IndexedArchive<R: Read + Seek> {
    archive: ZipArchive<R>,
    entries: BTreeMap<PartPath, usize>,
    archive_bytes: u64,
}

impl<R: Read + Seek> OpenedPackage<R> {
    pub(super) const fn limits(&self) -> ReadLimits {
        self.limits
    }

    pub(super) const fn archive_bytes(&self) -> u64 {
        self.archive_bytes
    }

    pub(super) const fn workbook_part(&self) -> &PartPath {
        &self.workbook_part
    }

    pub(super) fn worksheet_part(&self, relationship_id: &str) -> Option<&PartPath> {
        self.worksheet_parts.get(relationship_id)
    }

    pub(super) fn worksheet_count(&self) -> usize {
        self.worksheet_parts.len()
    }

    pub(super) const fn styles_part(&self) -> Option<&PartPath> {
        self.styles_part.as_ref()
    }

    pub(super) const fn shared_strings_part(&self) -> Option<&PartPath> {
        self.shared_strings_part.as_ref()
    }

    pub(super) const fn metadata_part(&self) -> Option<&PartPath> {
        self.metadata_part.as_ref()
    }

    pub(super) const fn has_external_links(&self) -> bool {
        self.has_external_links
    }

    pub(super) const fn has_macros(&self) -> bool {
        self.has_macros
    }

    pub(super) const fn workbook_kind(&self) -> WorkbookPackageKind {
        self.workbook_kind
    }

    pub(super) fn read_part(&mut self, part: &PartPath) -> Result<Vec<u8>, XlsxReadError> {
        read_required_part(
            &mut self.archive,
            &self.entries,
            part,
            self.limits,
            &mut self.uncompressed_spent,
        )
    }

    /// Resolves one worksheet's table relationships to their package parts.
    ///
    /// A worksheet without a relationship part has no tables; that is the common case and
    /// returns an empty map. Relationship-level malformations (external targets, duplicate
    /// targets, missing parts, wrong content type) are package-integrity failures, matching
    /// how workbook-level support parts are treated.
    ///
    /// # Errors
    ///
    /// Returns an [`XlsxReadError`] when the relationship part cannot be read or a table
    /// relationship is structurally invalid.
    pub(super) fn worksheet_table_parts(
        &mut self,
        worksheet_part: &PartPath,
    ) -> Result<BTreeMap<Box<str>, PartPath>, XlsxReadError> {
        let relationship_part = worksheet_part.relationship_part()?;
        if !self.entries.contains_key(&relationship_part) {
            return Ok(BTreeMap::new());
        }
        let bytes = read_required_part(
            &mut self.archive,
            &self.entries,
            &relationship_part,
            self.limits,
            &mut self.uncompressed_spent,
        )?;
        let relationships = xml::parse_relationships(
            &bytes,
            &relationship_part,
            Some(worksheet_part),
            self.limits,
        )?;
        let mut parts = BTreeMap::new();
        let mut unique = BTreeSet::new();
        for relationship in relationships
            .iter()
            .filter(|relationship| relationship_type::is_table(&relationship.kind))
        {
            let RelationshipTarget::Internal(part) = &relationship.target else {
                return Err(XlsxReadError::new(XlsxErrorCode::InvalidRelationships)
                    .with_detail(detail::EXTERNAL_SUPPORT_PART)
                    .at_source(relationship_part.source_id()));
            };
            if !unique.insert(part.clone()) {
                return Err(XlsxReadError::new(XlsxErrorCode::InvalidRelationships)
                    .with_detail(detail::DUPLICATE_WORKSHEET_TARGET)
                    .at_source(relationship_part.source_id()));
            }
            ensure_part(&self.entries, part)?;
            ensure_content_type(&self.content_types, part, CONTENT_TABLE)?;
            parts.insert(relationship.id.clone(), part.clone());
        }
        Ok(parts)
    }

    pub(super) fn summary(&self) -> PackageSummary {
        PackageSummary::from_opened(self)
    }
}

/// Validates package budgets and discovers workbook-related parts without reading cells.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when the ZIP package is malformed, unsafe, incomplete, or exceeds
/// a configured resource limit.
pub fn inspect_package<R: Read + Seek>(
    reader: R,
    options: ReadOptions,
) -> Result<PackageSummary, XlsxReadError> {
    open_package(reader, options).map(|package| package.summary())
}

pub(super) fn open_package<R: Read + Seek>(
    reader: R,
    options: ReadOptions,
) -> Result<OpenedPackage<R>, XlsxReadError> {
    let limits = options.limits();
    let IndexedArchive {
        mut archive,
        entries,
        archive_bytes,
    } = index_archive(reader, limits)?;

    let mut uncompressed_spent = 0_u64;
    let content_types_part = PartPath::from_archive_name(CONTENT_TYPES_PART)?;
    let content_types_bytes = read_required_part(
        &mut archive,
        &entries,
        &content_types_part,
        limits,
        &mut uncompressed_spent,
    )?;
    let content_types =
        xml::parse_content_types(&content_types_bytes, &content_types_part, limits)?;

    let root_relationships_part = PartPath::from_archive_name(ROOT_RELATIONSHIPS_PART)?;
    let root_relationships_bytes = read_required_part(
        &mut archive,
        &entries,
        &root_relationships_part,
        limits,
        &mut uncompressed_spent,
    )?;
    let root_relationships = xml::parse_relationships(
        &root_relationships_bytes,
        &root_relationships_part,
        None,
        limits,
    )?;

    let workbook_part = select_workbook_part(&root_relationships)?;
    ensure_part(&entries, &workbook_part)?;
    ensure_content_type_one_of(
        &content_types,
        &workbook_part,
        &[CONTENT_WORKBOOK, CONTENT_WORKBOOK_MACRO_ENABLED],
    )?;
    let workbook_kind = match content_types.content_type(&workbook_part) {
        Some(CONTENT_WORKBOOK_MACRO_ENABLED) => WorkbookPackageKind::Xlsm,
        Some(CONTENT_WORKBOOK) => WorkbookPackageKind::Xlsx,
        _ => {
            return Err(XlsxReadError::new(XlsxErrorCode::UnsupportedContentType)
                .at_source(workbook_part.source_id()));
        }
    };

    let workbook_relationships_part = workbook_part.relationship_part()?;
    let workbook_relationships_bytes = read_required_part(
        &mut archive,
        &entries,
        &workbook_relationships_part,
        limits,
        &mut uncompressed_spent,
    )?;
    let workbook_relationships = xml::parse_relationships(
        &workbook_relationships_bytes,
        &workbook_relationships_part,
        Some(&workbook_part),
        limits,
    )?;

    let worksheet_parts = select_worksheet_parts(
        &workbook_relationships,
        &entries,
        &content_types,
        &workbook_relationships_part,
    )?;
    let styles_part = select_optional_part(
        &workbook_relationships,
        &entries,
        &workbook_relationships_part,
        relationship_type::is_styles,
    )?;
    if let Some(part) = &styles_part {
        ensure_content_type(&content_types, part, CONTENT_STYLES)?;
    }
    let shared_strings_part = select_optional_part(
        &workbook_relationships,
        &entries,
        &workbook_relationships_part,
        relationship_type::is_shared_strings,
    )?;
    if let Some(part) = &shared_strings_part {
        ensure_content_type(&content_types, part, CONTENT_SHARED_STRINGS)?;
    }
    let metadata_part = select_optional_part(
        &workbook_relationships,
        &entries,
        &workbook_relationships_part,
        relationship_type::is_sheet_metadata,
    )?;
    if let Some(part) = &metadata_part {
        ensure_content_type(&content_types, part, CONTENT_SHEET_METADATA)?;
    }
    let external_relationship_count = root_relationships
        .iter()
        .chain(&workbook_relationships)
        .filter(|relationship| {
            matches!(
                &relationship.target,
                RelationshipTarget::External(target) if !target.is_empty()
            )
        })
        .count();
    let has_external_links = workbook_relationships
        .iter()
        .any(|relationship| relationship_type::is_external_link(&relationship.kind));
    let has_macros = workbook_relationships
        .iter()
        .any(|relationship| relationship_type::is_vba_project(&relationship.kind));

    Ok(OpenedPackage {
        archive,
        entries,
        content_types,
        limits,
        archive_bytes,
        uncompressed_spent,
        workbook_part,
        worksheet_parts,
        styles_part,
        shared_strings_part,
        metadata_part,
        external_relationship_count,
        has_external_links,
        has_macros,
        workbook_kind,
    })
}

fn index_archive<R: Read + Seek>(
    mut reader: R,
    limits: ReadLimits,
) -> Result<IndexedArchive<R>, XlsxReadError> {
    let archive_bytes = reader.seek(SeekFrom::End(0)).map_err(io_error)?;
    if archive_bytes > limits.max_archive_bytes() {
        return Err(XlsxReadError::new(XlsxErrorCode::ArchiveTooLarge)
            .with_detail(archive_bytes.to_string()));
    }
    reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut archive = ZipArchive::new(reader).map_err(zip_error)?;
    if archive.len() as u64 > limits.max_entries() {
        return Err(XlsxReadError::new(XlsxErrorCode::TooManyEntries)
            .with_detail(archive.len().to_string()));
    }
    if archive.has_overlapping_files().map_err(zip_error)? {
        return Err(XlsxReadError::new(XlsxErrorCode::OverlappingEntries));
    }

    let mut entries = BTreeMap::new();
    let mut total_uncompressed = 0_u128;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(zip_error)?;
        let raw_name = file.name_raw();
        let normalized_name = raw_name.strip_suffix(b"/").unwrap_or(raw_name);
        if normalized_name.is_empty() {
            continue;
        }
        let part = PartPath::from_archive_name(normalized_name)?;
        if file.is_dir() {
            continue;
        }
        if file.encrypted() {
            return Err(
                XlsxReadError::new(XlsxErrorCode::EncryptedEntry).at_source(part.source_id())
            );
        }
        if !matches!(
            file.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(XlsxReadError::new(XlsxErrorCode::UnsupportedCompression)
                .with_detail(format!("{:?}", file.compression()))
                .at_source(part.source_id()));
        }

        let uncompressed = file.size();
        if uncompressed > limits.max_entry_uncompressed_bytes() {
            return Err(XlsxReadError::new(XlsxErrorCode::EntryTooLarge)
                .with_detail(uncompressed.to_string())
                .at_source(part.source_id()));
        }
        total_uncompressed += u128::from(uncompressed);
        if total_uncompressed > u128::from(limits.max_total_uncompressed_bytes()) {
            return Err(XlsxReadError::new(XlsxErrorCode::TotalUncompressedTooLarge));
        }
        if compression_ratio_exceeded(
            uncompressed,
            file.compressed_size(),
            limits.max_compression_ratio(),
        ) {
            return Err(XlsxReadError::new(XlsxErrorCode::CompressionRatioExceeded)
                .at_source(part.source_id()));
        }
        drop(file);

        if entries.insert(part.clone(), index).is_some() {
            return Err(
                XlsxReadError::new(XlsxErrorCode::DuplicatePart).at_source(part.source_id())
            );
        }
    }
    Ok(IndexedArchive {
        archive,
        entries,
        archive_bytes,
    })
}

/// Reads one required part and charges its real cost against the package budget.
///
/// The central-directory sizes consulted while indexing are attacker-controlled,
/// and the ZIP reader does not stop a Deflated entry at its declared length. Every
/// entry is therefore measured as it is inflated: the observed length must match
/// the declared length, and `spent` accumulates the bytes actually produced across
/// the whole package. Together those two checks make the declared metadata that
/// [`index_archive`] budgets against trustworthy for every part that is read.
fn read_required_part<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &BTreeMap<PartPath, usize>,
    part: &PartPath,
    limits: ReadLimits,
    spent: &mut u64,
) -> Result<Vec<u8>, XlsxReadError> {
    let index = entries.get(part).ok_or_else(|| {
        XlsxReadError::new(XlsxErrorCode::MissingPart).at_source(part.source_id())
    })?;
    let file = archive.by_index(*index).map_err(zip_error)?;
    let declared = file.size();
    let mut bytes = Vec::new();
    file.take(limits.max_entry_uncompressed_bytes().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            XlsxReadError::new(XlsxErrorCode::InvalidZip)
                .at_source(part.source_id())
                .with_cause(error)
        })?;
    let produced = bytes.len() as u64;
    if produced > limits.max_entry_uncompressed_bytes() {
        return Err(XlsxReadError::new(XlsxErrorCode::EntryTooLarge).at_source(part.source_id()));
    }
    if produced != declared {
        return Err(XlsxReadError::new(XlsxErrorCode::DeclaredSizeMismatch)
            .with_detail(format!("declared {declared} bytes, read {produced} bytes"))
            .at_source(part.source_id()));
    }
    *spent = spent.saturating_add(produced);
    if *spent > limits.max_total_uncompressed_bytes() {
        return Err(XlsxReadError::new(XlsxErrorCode::TotalUncompressedTooLarge)
            .with_detail(spent.to_string()));
    }
    Ok(bytes)
}

fn select_workbook_part(relationships: &[Relationship]) -> Result<PartPath, XlsxReadError> {
    let mut internal = Vec::new();
    for relationship in relationships
        .iter()
        .filter(|relationship| relationship_type::is_office_document(&relationship.kind))
    {
        match &relationship.target {
            RelationshipTarget::Internal(part) => internal.push(part.clone()),
            RelationshipTarget::External(_) => {
                return Err(XlsxReadError::new(
                    XlsxErrorCode::ExternalWorkbookRelationship,
                ));
            }
        }
    }
    match internal.as_slice() {
        [] => Err(XlsxReadError::new(
            XlsxErrorCode::MissingWorkbookRelationship,
        )),
        [part] => Ok(part.clone()),
        _ => Err(XlsxReadError::new(
            XlsxErrorCode::DuplicateWorkbookRelationship,
        )),
    }
}

fn select_worksheet_parts(
    relationships: &[Relationship],
    entries: &BTreeMap<PartPath, usize>,
    content_types: &ContentTypes,
    relationship_part: &PartPath,
) -> Result<BTreeMap<Box<str>, PartPath>, XlsxReadError> {
    let mut parts = BTreeMap::new();
    let mut unique = BTreeSet::new();
    for relationship in relationships
        .iter()
        .filter(|relationship| relationship_type::is_worksheet(&relationship.kind))
    {
        let RelationshipTarget::Internal(part) = &relationship.target else {
            return Err(XlsxReadError::new(XlsxErrorCode::InvalidRelationships)
                .with_detail(detail::EXTERNAL_WORKSHEET)
                .at_source(relationship_part.source_id()));
        };
        if !unique.insert(part.clone()) {
            return Err(XlsxReadError::new(XlsxErrorCode::InvalidRelationships)
                .with_detail(detail::DUPLICATE_WORKSHEET_TARGET)
                .at_source(relationship_part.source_id()));
        }
        ensure_part(entries, part)?;
        ensure_content_type(content_types, part, CONTENT_WORKSHEET)?;
        parts.insert(relationship.id.clone(), part.clone());
    }
    if parts.is_empty() {
        return Err(
            XlsxReadError::new(XlsxErrorCode::MissingWorksheetRelationship)
                .at_source(relationship_part.source_id()),
        );
    }
    Ok(parts)
}

fn select_optional_part(
    relationships: &[Relationship],
    entries: &BTreeMap<PartPath, usize>,
    relationship_part: &PartPath,
    predicate: fn(&str) -> bool,
) -> Result<Option<PartPath>, XlsxReadError> {
    let mut selected = None;
    for relationship in relationships
        .iter()
        .filter(|relationship| predicate(&relationship.kind))
    {
        let RelationshipTarget::Internal(part) = &relationship.target else {
            return Err(XlsxReadError::new(XlsxErrorCode::InvalidRelationships)
                .with_detail(detail::EXTERNAL_SUPPORT_PART)
                .at_source(relationship_part.source_id()));
        };
        if selected.replace(part.clone()).is_some() {
            return Err(XlsxReadError::new(XlsxErrorCode::InvalidRelationships)
                .with_detail(detail::DUPLICATE_SINGLETON_RELATIONSHIP)
                .at_source(relationship_part.source_id()));
        }
        ensure_part(entries, part)?;
    }
    Ok(selected)
}

fn ensure_part(entries: &BTreeMap<PartPath, usize>, part: &PartPath) -> Result<(), XlsxReadError> {
    if !entries.contains_key(part) {
        return Err(XlsxReadError::new(XlsxErrorCode::MissingPart).at_source(part.source_id()));
    }
    Ok(())
}

fn ensure_content_type(
    content_types: &ContentTypes,
    part: &PartPath,
    expected: &str,
) -> Result<(), XlsxReadError> {
    match content_types.content_type(part) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(XlsxReadError::new(XlsxErrorCode::UnsupportedContentType)
            .with_detail(actual.to_owned())
            .at_source(part.source_id())),
        None => Err(XlsxReadError::new(XlsxErrorCode::InvalidContentTypes)
            .with_detail(detail::MISSING_CONTENT_TYPE)
            .at_source(part.source_id())),
    }
}

fn ensure_content_type_one_of(
    content_types: &ContentTypes,
    part: &PartPath,
    expected: &[&str],
) -> Result<(), XlsxReadError> {
    match content_types.content_type(part) {
        Some(actual) if expected.contains(&actual) => Ok(()),
        Some(actual) => Err(XlsxReadError::new(XlsxErrorCode::UnsupportedContentType)
            .with_detail(actual.to_owned())
            .at_source(part.source_id())),
        None => Err(XlsxReadError::new(XlsxErrorCode::InvalidContentTypes)
            .with_detail(detail::MISSING_CONTENT_TYPE)
            .at_source(part.source_id())),
    }
}

fn compression_ratio_exceeded(uncompressed: u64, compressed: u64, maximum: u64) -> bool {
    uncompressed > 0
        && (compressed == 0
            || u128::from(uncompressed) > u128::from(compressed) * u128::from(maximum))
}

fn io_error(error: std::io::Error) -> XlsxReadError {
    XlsxReadError::new(XlsxErrorCode::Io).with_cause(error)
}

fn zip_error(error: zip::result::ZipError) -> XlsxReadError {
    XlsxReadError::new(XlsxErrorCode::InvalidZip).with_cause(error)
}
