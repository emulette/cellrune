const OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const OFFICE_DOCUMENT_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";
const WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const WORKSHEET_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/worksheet";
const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const STYLES_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/styles";
const SHARED_STRINGS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const SHARED_STRINGS_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/sharedStrings";
const SHEET_METADATA: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata";
const SHEET_METADATA_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/sheetMetadata";
const EXTERNAL_LINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink";
const EXTERNAL_LINK_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/externalLink";
const VBA_PROJECT: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const CALC_CHAIN: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain";
const CALC_CHAIN_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/calcChain";

pub(super) fn is_office_document(value: &str) -> bool {
    matches!(value, OFFICE_DOCUMENT | OFFICE_DOCUMENT_STRICT)
}

pub(super) fn is_worksheet(value: &str) -> bool {
    matches!(value, WORKSHEET | WORKSHEET_STRICT)
}

pub(super) fn is_styles(value: &str) -> bool {
    matches!(value, STYLES | STYLES_STRICT)
}

pub(super) fn is_shared_strings(value: &str) -> bool {
    matches!(value, SHARED_STRINGS | SHARED_STRINGS_STRICT)
}

pub(super) fn is_sheet_metadata(value: &str) -> bool {
    matches!(value, SHEET_METADATA | SHEET_METADATA_STRICT)
}

pub(super) fn is_external_link(value: &str) -> bool {
    matches!(value, EXTERNAL_LINK | EXTERNAL_LINK_STRICT)
}

pub(super) fn is_vba_project(value: &str) -> bool {
    value == VBA_PROJECT
}

pub(crate) fn is_calc_chain(value: &str) -> bool {
    matches!(value, CALC_CHAIN | CALC_CHAIN_STRICT)
}
