use std::hash::{BuildHasher, Hasher};

use super::report::{
    TRANSACTION_REPORT_CONTRACT_VERSION, TransactionDetailItem, TransactionDetailSection,
    WorkbookTransactionReport,
};
use super::{CancellationToken, SessionError, SessionErrorCode};

const TRANSACTION_CURSOR_TOKEN_BYTES: usize = 27;
const TRANSACTION_CURSOR_TOKEN_HEX_BYTES: usize = TRANSACTION_CURSOR_TOKEN_BYTES * 2;

/// Opaque report-local cursor for one transaction detail section.
#[derive(Debug, Clone)]
pub struct TransactionPageCursor {
    report_identity: u64,
    contract_version: u16,
    section: TransactionDetailSection,
    offset: usize,
    authenticator: u64,
}

impl TransactionPageCursor {
    /// Returns the opaque lowercase hexadecimal token used for interop round trips.
    pub fn to_token(&self) -> String {
        let mut bytes = [0_u8; TRANSACTION_CURSOR_TOKEN_BYTES];
        bytes[..2].copy_from_slice(&self.contract_version.to_be_bytes());
        bytes[2..10].copy_from_slice(&self.report_identity.to_be_bytes());
        bytes[10] = detail_section_code(self.section);
        bytes[11..19].copy_from_slice(&(self.offset as u64).to_be_bytes());
        bytes[19..].copy_from_slice(&self.authenticator.to_be_bytes());
        let mut token = String::with_capacity(TRANSACTION_CURSOR_TOKEN_HEX_BYTES);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut token, "{byte:02x}").expect("writing to a String cannot fail");
        }
        token
    }

    fn from_token(token: &str) -> Result<Self, SessionError> {
        if token.len() != TRANSACTION_CURSOR_TOKEN_HEX_BYTES || !token.is_ascii() {
            return Err(invalid_cursor());
        }
        let mut bytes = [0_u8; TRANSACTION_CURSOR_TOKEN_BYTES];
        for (index, chunk) in token.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_nibble(chunk[0]).ok_or_else(invalid_cursor)?;
            let low = hex_nibble(chunk[1]).ok_or_else(invalid_cursor)?;
            bytes[index] = (high << 4) | low;
        }
        let contract_version = u16::from_be_bytes([bytes[0], bytes[1]]);
        let report_identity = u64::from_be_bytes(
            bytes[2..10]
                .try_into()
                .expect("fixed cursor identity width"),
        );
        let section = detail_section_from_code(bytes[10]).ok_or_else(invalid_cursor)?;
        let wire_offset =
            u64::from_be_bytes(bytes[11..19].try_into().expect("fixed cursor offset width"));
        let offset = usize::try_from(wire_offset).map_err(|_| invalid_cursor())?;
        let authenticator = u64::from_be_bytes(
            bytes[19..]
                .try_into()
                .expect("fixed cursor authenticator width"),
        );
        Ok(Self {
            report_identity,
            contract_version,
            section,
            offset,
            authenticator,
        })
    }
}

/// One complete item-bounded transaction detail page.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionImpactPage {
    section: TransactionDetailSection,
    items: Vec<TransactionDetailItem>,
    next_cursor: Option<TransactionPageCursor>,
}

impl TransactionImpactPage {
    /// Returns the detail section represented by this page.
    pub const fn section(&self) -> TransactionDetailSection {
        self.section
    }

    /// Returns the complete items in deterministic cell order.
    pub fn items(&self) -> &[TransactionDetailItem] {
        &self.items
    }

    /// Returns the cursor for the next page, or `None` when this section is complete.
    pub const fn next_cursor(&self) -> Option<&TransactionPageCursor> {
        self.next_cursor.as_ref()
    }
}

impl PartialEq for TransactionPageCursor {
    fn eq(&self, other: &Self) -> bool {
        self.report_identity == other.report_identity
            && self.contract_version == other.contract_version
            && self.section == other.section
            && self.offset == other.offset
            && self.authenticator == other.authenticator
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

const fn detail_section_code(section: TransactionDetailSection) -> u8 {
    match section {
        TransactionDetailSection::Affected => 1,
        TransactionDetailSection::Evaluated => 2,
        TransactionDetailSection::PreviewResults => 3,
        TransactionDetailSection::PreviewIssues => 4,
        TransactionDetailSection::InstallResults => 5,
    }
}

const fn detail_section_from_code(code: u8) -> Option<TransactionDetailSection> {
    match code {
        1 => Some(TransactionDetailSection::Affected),
        2 => Some(TransactionDetailSection::Evaluated),
        3 => Some(TransactionDetailSection::PreviewResults),
        4 => Some(TransactionDetailSection::PreviewIssues),
        5 => Some(TransactionDetailSection::InstallResults),
        _ => None,
    }
}

fn invalid_cursor() -> SessionError {
    SessionError::new(SessionErrorCode::TransactionCursorInvalid, None)
}

impl WorkbookTransactionReport {
    /// Returns one complete item-bounded detail page.
    ///
    /// A zero `limit` selects the configured maximum. Cursors are bound to this report and
    /// section and cannot be replayed against another completed transaction.
    ///
    /// # Errors
    ///
    /// Returns a stable cursor or page-limit error.
    pub(super) fn page(
        &self,
        section: TransactionDetailSection,
        cursor: Option<&TransactionPageCursor>,
        limit: usize,
    ) -> Result<TransactionImpactPage, SessionError> {
        self.page_cancellable(section, cursor, limit, &CancellationToken::new())
    }

    /// Returns one complete item-bounded detail page with cooperative cancellation.
    ///
    /// A cancellation before the page has been fully cloned returns a retryable cancellation
    /// error and does not consume or otherwise change the report.
    ///
    /// # Errors
    ///
    /// Returns a stable cursor, page-limit, or cancellation error.
    pub(super) fn page_cancellable(
        &self,
        section: TransactionDetailSection,
        cursor: Option<&TransactionPageCursor>,
        limit: usize,
        cancellation: &CancellationToken,
    ) -> Result<TransactionImpactPage, SessionError> {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let offset = match cursor {
            Some(cursor)
                if self.identity == cursor.report_identity
                    && cursor.contract_version == TRANSACTION_REPORT_CONTRACT_VERSION
                    && cursor.section == section =>
            {
                let expected = self.cursor_authenticator(
                    cursor.contract_version,
                    cursor.report_identity,
                    cursor.section,
                    cursor.offset,
                );
                if cursor.authenticator != expected {
                    return Err(invalid_cursor());
                }
                cursor.offset
            }
            Some(_) => {
                return Err(invalid_cursor());
            }
            None => 0,
        };
        let limit = if limit == 0 {
            self.max_page_items
        } else {
            limit
        };
        if limit > self.max_page_items {
            return Err(SessionError::new(
                SessionErrorCode::PageLimitExceeded,
                Some(format!("requested={limit}, limit={}", self.max_page_items)),
            ));
        }
        let details = self.details(section);
        if offset > details.len() {
            return Err(invalid_cursor());
        }
        let end = offset.saturating_add(limit).min(details.len());
        let mut items = Vec::with_capacity(end.saturating_sub(offset));
        for item in &details[offset..end] {
            if cancellation.is_cancelled() {
                return Err(SessionError::new(SessionErrorCode::Cancelled, None));
            }
            items.push(item.clone());
        }
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let next_cursor = (end < details.len()).then(|| self.cursor(section, end));
        Ok(TransactionImpactPage {
            section,
            items,
            next_cursor,
        })
    }

    pub(super) fn cursor_from_token(
        &self,
        token: &str,
    ) -> Result<TransactionPageCursor, SessionError> {
        let cursor = TransactionPageCursor::from_token(token)?;
        if cursor.report_identity != self.identity
            || cursor.contract_version != TRANSACTION_REPORT_CONTRACT_VERSION
            || cursor.authenticator
                != self.cursor_authenticator(
                    cursor.contract_version,
                    cursor.report_identity,
                    cursor.section,
                    cursor.offset,
                )
        {
            return Err(invalid_cursor());
        }
        Ok(cursor)
    }

    fn cursor(&self, section: TransactionDetailSection, offset: usize) -> TransactionPageCursor {
        TransactionPageCursor {
            report_identity: self.identity,
            contract_version: TRANSACTION_REPORT_CONTRACT_VERSION,
            section,
            offset,
            authenticator: self.cursor_authenticator(
                TRANSACTION_REPORT_CONTRACT_VERSION,
                self.identity,
                section,
                offset,
            ),
        }
    }

    fn cursor_authenticator(
        &self,
        contract_version: u16,
        report_identity: u64,
        section: TransactionDetailSection,
        offset: usize,
    ) -> u64 {
        let mut hasher = self.cursor_hash_builder.build_hasher();
        hasher.write(b"cellrune.transaction.cursor.v1");
        hasher.write_u16(contract_version);
        hasher.write_u64(report_identity);
        hasher.write_u8(detail_section_code(section));
        hasher.write_u64(offset as u64);
        hasher.finish()
    }
}
