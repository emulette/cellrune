use std::io::{Read, Seek};

use super::{OpenedPackage, PartPath};
use crate::SourceId;

/// Safely discovered package parts before workbook values are interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSummary {
    workbook_part: SourceId,
    worksheet_parts: Vec<SourceId>,
    styles_part: Option<SourceId>,
    shared_strings_part: Option<SourceId>,
    metadata_part: Option<SourceId>,
    external_relationship_count: usize,
    has_external_links: bool,
    has_macros: bool,
    entry_count: usize,
}

impl PackageSummary {
    pub(super) fn from_opened<R: Read + Seek>(package: &OpenedPackage<R>) -> Self {
        Self {
            workbook_part: package.workbook_part.source_id(),
            worksheet_parts: package
                .worksheet_parts
                .values()
                .map(PartPath::source_id)
                .collect(),
            styles_part: package.styles_part.as_ref().map(PartPath::source_id),
            shared_strings_part: package
                .shared_strings_part
                .as_ref()
                .map(PartPath::source_id),
            metadata_part: package.metadata_part.as_ref().map(PartPath::source_id),
            external_relationship_count: package.external_relationship_count,
            has_external_links: package.has_external_links,
            has_macros: package.has_macros,
            entry_count: package.entries.len(),
        }
    }

    /// Returns the relationship-selected workbook part.
    pub const fn workbook_part(&self) -> &SourceId {
        &self.workbook_part
    }

    /// Returns worksheet parts in deterministic relationship-ID order.
    pub fn worksheet_parts(&self) -> &[SourceId] {
        &self.worksheet_parts
    }

    /// Returns the optional styles part.
    pub const fn styles_part(&self) -> Option<&SourceId> {
        self.styles_part.as_ref()
    }

    /// Returns the optional shared-strings part.
    pub const fn shared_strings_part(&self) -> Option<&SourceId> {
        self.shared_strings_part.as_ref()
    }

    /// Returns the optional cell-metadata part.
    pub const fn metadata_part(&self) -> Option<&SourceId> {
        self.metadata_part.as_ref()
    }

    /// Returns external relationships found in the inspected root and workbook relationship parts.
    pub const fn external_relationship_count(&self) -> usize {
        self.external_relationship_count
    }

    /// Returns whether workbook metadata advertises an external data link.
    pub const fn has_external_links(&self) -> bool {
        self.has_external_links
    }

    /// Returns whether the package advertises a VBA project relationship.
    pub const fn has_macros(&self) -> bool {
        self.has_macros
    }

    /// Returns the bounded ZIP entry count.
    pub const fn entry_count(&self) -> usize {
        self.entry_count
    }
}
