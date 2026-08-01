use super::value::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SheetSpanPolicy {
    CollectAcrossSheets,
    ReturnExcelError(ErrorKind),
    Unsupported,
}

pub(super) const ARRAY_EXPRESSION_POLICY: SheetSpanPolicy =
    SheetSpanPolicy::ReturnExcelError(ErrorKind::Value);
