use cellrune_interop::{
    DefinedNameInspectionDto, DefinedNameInspectionResultDto, DefinedNameReferenceAreaDto,
    DefinedNameSheetSpanDto,
};
use napi_derive::napi;

#[napi(object)]
pub struct NativeDefinedNameSheetSpan {
    pub start_sheet_id: u32,
    pub start_sheet_name: String,
    pub end_sheet_id: u32,
    pub end_sheet_name: String,
}

#[napi(object)]
pub struct NativeDefinedNameReferenceArea {
    pub kind: String,
    pub sheet_id: Option<u32>,
    pub sheet_name: Option<String>,
    pub range: String,
    pub sheet_span: Option<NativeDefinedNameSheetSpan>,
}

#[napi(object)]
pub struct NativeDefinedNameInspectionResult {
    pub kind: String,
    pub sheet_id: Option<u32>,
    pub sheet_name: Option<String>,
    pub range: Option<String>,
    pub sheet_span: Option<NativeDefinedNameSheetSpan>,
    pub areas: Option<Vec<NativeDefinedNameReferenceArea>>,
    pub dynamic_kind: Option<String>,
    pub formula: Option<String>,
    pub locator: Option<String>,
    pub workbook: Option<String>,
    pub sheet: Option<String>,
    pub sheet_end: Option<String>,
    pub target_kind: Option<String>,
    pub target_text: Option<String>,
    pub reason: Option<String>,
    pub detail: Option<String>,
}

#[napi(object)]
pub struct NativeDefinedNameInspection {
    pub schema_version: u32,
    pub result: NativeDefinedNameInspectionResult,
}

pub(crate) fn defined_name_inspection(
    value: DefinedNameInspectionDto,
) -> NativeDefinedNameInspection {
    NativeDefinedNameInspection {
        schema_version: value.schema_version,
        result: defined_name_result(value.result),
    }
}

fn defined_name_result(value: DefinedNameInspectionResultDto) -> NativeDefinedNameInspectionResult {
    match value {
        DefinedNameInspectionResultDto::Rectangular {
            sheet_id,
            sheet_name,
            range,
        } => NativeDefinedNameInspectionResult {
            kind: "rectangular".to_owned(),
            sheet_id: Some(sheet_id),
            sheet_name: Some(sheet_name),
            range: Some(range),
            ..empty_result()
        },
        DefinedNameInspectionResultDto::ThreeDimensional { sheet_span, range } => {
            NativeDefinedNameInspectionResult {
                kind: "three_dimensional".to_owned(),
                range: Some(range),
                sheet_span: Some(defined_name_sheet_span(sheet_span)),
                ..empty_result()
            }
        }
        DefinedNameInspectionResultDto::NonRectangular { areas } => {
            NativeDefinedNameInspectionResult {
                kind: "non_rectangular".to_owned(),
                areas: Some(areas.into_iter().map(defined_name_area).collect()),
                ..empty_result()
            }
        }
        DefinedNameInspectionResultDto::EmptyReference => NativeDefinedNameInspectionResult {
            kind: "empty_reference".to_owned(),
            ..empty_result()
        },
        DefinedNameInspectionResultDto::DynamicFormula {
            dynamic_kind,
            formula,
        } => NativeDefinedNameInspectionResult {
            kind: "dynamic_formula".to_owned(),
            dynamic_kind: Some(dynamic_kind.as_str().to_owned()),
            formula: Some(formula),
            ..empty_result()
        },
        DefinedNameInspectionResultDto::Constant { formula } => NativeDefinedNameInspectionResult {
            kind: "constant".to_owned(),
            formula: Some(formula),
            ..empty_result()
        },
        DefinedNameInspectionResultDto::ExternalReference {
            locator,
            workbook,
            sheet,
            sheet_end,
            target_kind,
            target_text,
        } => NativeDefinedNameInspectionResult {
            kind: "external_reference".to_owned(),
            locator,
            workbook: Some(workbook),
            sheet,
            sheet_end,
            target_kind: Some(target_kind.as_str().to_owned()),
            target_text: Some(target_text),
            ..empty_result()
        },
        DefinedNameInspectionResultDto::Invalid { reason, detail } => {
            NativeDefinedNameInspectionResult {
                kind: "invalid".to_owned(),
                reason: Some(reason.as_str().to_owned()),
                detail,
                ..empty_result()
            }
        }
        DefinedNameInspectionResultDto::Unsupported { reason, detail } => {
            NativeDefinedNameInspectionResult {
                kind: "unsupported".to_owned(),
                reason: Some(reason.as_str().to_owned()),
                detail,
                ..empty_result()
            }
        }
        DefinedNameInspectionResultDto::NotFound => NativeDefinedNameInspectionResult {
            kind: "not_found".to_owned(),
            ..empty_result()
        },
    }
}

fn defined_name_area(value: DefinedNameReferenceAreaDto) -> NativeDefinedNameReferenceArea {
    match value {
        DefinedNameReferenceAreaDto::Rectangular {
            sheet_id,
            sheet_name,
            range,
        } => NativeDefinedNameReferenceArea {
            kind: "rectangular".to_owned(),
            sheet_id: Some(sheet_id),
            sheet_name: Some(sheet_name),
            range,
            sheet_span: None,
        },
        DefinedNameReferenceAreaDto::ThreeDimensional { sheet_span, range } => {
            NativeDefinedNameReferenceArea {
                kind: "three_dimensional".to_owned(),
                sheet_id: None,
                sheet_name: None,
                range,
                sheet_span: Some(defined_name_sheet_span(sheet_span)),
            }
        }
    }
}

fn defined_name_sheet_span(value: DefinedNameSheetSpanDto) -> NativeDefinedNameSheetSpan {
    NativeDefinedNameSheetSpan {
        start_sheet_id: value.start_sheet_id,
        start_sheet_name: value.start_sheet_name,
        end_sheet_id: value.end_sheet_id,
        end_sheet_name: value.end_sheet_name,
    }
}

fn empty_result() -> NativeDefinedNameInspectionResult {
    NativeDefinedNameInspectionResult {
        kind: String::new(),
        sheet_id: None,
        sheet_name: None,
        range: None,
        sheet_span: None,
        areas: None,
        dynamic_kind: None,
        formula: None,
        locator: None,
        workbook: None,
        sheet: None,
        sheet_end: None,
        target_kind: None,
        target_text: None,
        reason: None,
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_reference_dto_reaches_the_native_binding_shape() {
        let native = defined_name_inspection(DefinedNameInspectionDto {
            schema_version: 1,
            result: DefinedNameInspectionResultDto::EmptyReference,
        });

        assert_eq!(native.schema_version, 1);
        assert_eq!(native.result.kind, "empty_reference");
    }
}
