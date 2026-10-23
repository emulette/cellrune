//! Bounded pages of read-only worksheet metadata.

use cellrune::{CellRange, Comment, DataValidation, Hyperlink, Sheet};

use super::WorkbookSession;
use super::query::normalized_limit;
use crate::convert::count_u64;
use crate::{
    CommentDto, CommentPageDto, CommentReplyDto, DataValidationDto, DataValidationPageDto,
    HyperlinkDto, HyperlinkPageDto, INTEROP_SCHEMA_VERSION, InteropError, SheetPageRequestDto,
};

/// One bounded slice of a sheet's entries.
struct EntryPage<T> {
    sheet: String,
    total: u64,
    offset: u64,
    next_offset: Option<u64>,
    entries: Vec<T>,
}

impl WorkbookSession {
    /// Reads one page of a sheet's data validation rules in worksheet order.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for an unknown sheet, excessive page size, or out-of-range
    /// offset.
    pub fn data_validations(
        &self,
        request: &SheetPageRequestDto,
    ) -> Result<DataValidationPageDto, InteropError> {
        let page = self.sheet_page(request, Sheet::data_validations, data_validation_dto)?;
        Ok(DataValidationPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: page.sheet,
            total: page.total,
            offset: page.offset,
            next_offset: page.next_offset,
            entries: page.entries,
        })
    }

    /// Reads one page of a sheet's hyperlinks in worksheet order.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for an unknown sheet, excessive page size, or out-of-range
    /// offset.
    pub fn hyperlinks(
        &self,
        request: &SheetPageRequestDto,
    ) -> Result<HyperlinkPageDto, InteropError> {
        let page = self.sheet_page(request, Sheet::hyperlinks, hyperlink_dto)?;
        Ok(HyperlinkPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: page.sheet,
            total: page.total,
            offset: page.offset,
            next_offset: page.next_offset,
            entries: page.entries,
        })
    }

    /// Reads one page of a sheet's cell comments (notes) in comments-part order.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for an unknown sheet, excessive page size, or out-of-range
    /// offset.
    pub fn comments(&self, request: &SheetPageRequestDto) -> Result<CommentPageDto, InteropError> {
        let page = self.sheet_page(request, Sheet::comments, comment_dto)?;
        Ok(CommentPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: page.sheet,
            total: page.total,
            offset: page.offset,
            next_offset: page.next_offset,
            entries: page.entries,
        })
    }

    fn sheet_page<'a, T: 'a, D>(
        &'a self,
        request: &SheetPageRequestDto,
        entries: impl Fn(&'a Sheet) -> &'a [T],
        convert: impl Fn(&T) -> D,
    ) -> Result<EntryPage<D>, InteropError> {
        let sheet = self
            .engine
            .workbook()
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        let all = entries(sheet);
        let total = count_u64(all.len());
        if request.offset > total {
            return Err(InteropError::page_offset());
        }
        let limit = normalized_limit(request.limit)?;
        let returned = all
            .iter()
            .skip(usize::try_from(request.offset).unwrap_or(usize::MAX))
            .take(limit as usize)
            .map(convert)
            .collect::<Vec<_>>();
        let consumed = request.offset + count_u64(returned.len());
        Ok(EntryPage {
            sheet: sheet.name().as_str().to_owned(),
            total,
            offset: request.offset,
            next_offset: (consumed < total).then_some(consumed),
            entries: returned,
        })
    }
}

/// Renders a range as `sqref` does: one address for a one-cell range.
fn reference_text(range: &CellRange) -> String {
    if range.start() == range.end() {
        range.start().to_string()
    } else {
        format!("{}:{}", range.start(), range.end())
    }
}

fn data_validation_dto(validation: &DataValidation) -> DataValidationDto {
    DataValidationDto {
        ranges: validation.ranges().iter().map(reference_text).collect(),
        validation_type: validation.validation_type().as_str().to_owned(),
        operator: validation.operator().as_str().to_owned(),
        formula1: validation.formula1().map(str::to_owned),
        formula2: validation.formula2().map(str::to_owned),
        allow_blank: validation.allow_blank(),
        in_cell_dropdown: validation.in_cell_dropdown(),
        show_input_message: validation.show_input_message(),
        show_error_message: validation.show_error_message(),
        error_style: validation.error_style().as_str().to_owned(),
        error_title: validation.error_title().map(str::to_owned),
        error_message: validation.error_message().map(str::to_owned),
        prompt_title: validation.prompt_title().map(str::to_owned),
        prompt_message: validation.prompt_message().map(str::to_owned),
    }
}

fn hyperlink_dto(hyperlink: &Hyperlink) -> HyperlinkDto {
    HyperlinkDto {
        range: reference_text(&hyperlink.range()),
        target: hyperlink.target().map(str::to_owned),
        location: hyperlink.location().map(str::to_owned),
        display: hyperlink.display().map(str::to_owned),
        tooltip: hyperlink.tooltip().map(str::to_owned),
    }
}

fn comment_dto(comment: &Comment) -> CommentDto {
    CommentDto {
        address: comment.address().to_string(),
        author: comment.author().map(str::to_owned),
        text: comment.text().to_owned(),
        replies: comment
            .replies()
            .iter()
            .map(|reply| CommentReplyDto {
                author: reply.author().map(str::to_owned),
                text: reply.text().to_owned(),
            })
            .collect(),
    }
}
