//! Native views of read-only worksheet metadata pages.

use cellrune_interop::{
    CommentDto, CommentPageDto, DataValidationDto, DataValidationPageDto, HyperlinkDto,
    HyperlinkPageDto,
};
use napi_derive::napi;

#[napi(object)]
pub struct NativeDataValidationPage {
    pub schema_version: u32,
    pub sheet: String,
    pub total: f64,
    pub offset: f64,
    pub next_offset: Option<f64>,
    pub entries: Vec<NativeDataValidation>,
}

#[napi(object)]
pub struct NativeDataValidation {
    pub ranges: Vec<String>,
    pub validation_type: String,
    pub operator: String,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
    pub allow_blank: bool,
    pub in_cell_dropdown: bool,
    pub show_input_message: bool,
    pub show_error_message: bool,
    pub error_style: String,
    pub error_title: Option<String>,
    pub error_message: Option<String>,
    pub prompt_title: Option<String>,
    pub prompt_message: Option<String>,
}

pub(crate) fn data_validation_page(value: DataValidationPageDto) -> NativeDataValidationPage {
    NativeDataValidationPage {
        schema_version: value.schema_version,
        sheet: value.sheet,
        total: value.total as f64,
        offset: value.offset as f64,
        next_offset: value.next_offset.map(|offset| offset as f64),
        entries: value.entries.into_iter().map(data_validation).collect(),
    }
}

fn data_validation(value: DataValidationDto) -> NativeDataValidation {
    NativeDataValidation {
        ranges: value.ranges,
        validation_type: value.validation_type,
        operator: value.operator,
        formula1: value.formula1,
        formula2: value.formula2,
        allow_blank: value.allow_blank,
        in_cell_dropdown: value.in_cell_dropdown,
        show_input_message: value.show_input_message,
        show_error_message: value.show_error_message,
        error_style: value.error_style,
        error_title: value.error_title,
        error_message: value.error_message,
        prompt_title: value.prompt_title,
        prompt_message: value.prompt_message,
    }
}

#[napi(object)]
pub struct NativeHyperlinkPage {
    pub schema_version: u32,
    pub sheet: String,
    pub total: f64,
    pub offset: f64,
    pub next_offset: Option<f64>,
    pub entries: Vec<NativeHyperlink>,
}

#[napi(object)]
pub struct NativeHyperlink {
    pub range: String,
    pub target: Option<String>,
    pub location: Option<String>,
    pub display: Option<String>,
    pub tooltip: Option<String>,
}

#[napi(object)]
pub struct NativeCommentPage {
    pub schema_version: u32,
    pub sheet: String,
    pub total: f64,
    pub offset: f64,
    pub next_offset: Option<f64>,
    pub entries: Vec<NativeComment>,
}

#[napi(object)]
pub struct NativeComment {
    pub address: String,
    pub author: Option<String>,
    pub text: String,
    pub replies: Vec<NativeCommentReply>,
}

#[napi(object)]
pub struct NativeCommentReply {
    pub author: Option<String>,
    pub text: String,
}

pub(crate) fn hyperlink_page(value: HyperlinkPageDto) -> NativeHyperlinkPage {
    NativeHyperlinkPage {
        schema_version: value.schema_version,
        sheet: value.sheet,
        total: value.total as f64,
        offset: value.offset as f64,
        next_offset: value.next_offset.map(|offset| offset as f64),
        entries: value
            .entries
            .into_iter()
            .map(|entry: HyperlinkDto| NativeHyperlink {
                range: entry.range,
                target: entry.target,
                location: entry.location,
                display: entry.display,
                tooltip: entry.tooltip,
            })
            .collect(),
    }
}

pub(crate) fn comment_page(value: CommentPageDto) -> NativeCommentPage {
    NativeCommentPage {
        schema_version: value.schema_version,
        sheet: value.sheet,
        total: value.total as f64,
        offset: value.offset as f64,
        next_offset: value.next_offset.map(|offset| offset as f64),
        entries: value
            .entries
            .into_iter()
            .map(|entry: CommentDto| NativeComment {
                address: entry.address,
                author: entry.author,
                text: entry.text,
                replies: entry
                    .replies
                    .into_iter()
                    .map(|reply| NativeCommentReply {
                        author: reply.author,
                        text: reply.text,
                    })
                    .collect(),
            })
            .collect(),
    }
}
