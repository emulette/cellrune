use super::value::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SheetSpanPolicy {
    CollectAcrossSheets,
    ReturnExcelError(ErrorKind),
    Unsupported,
}

pub(super) const ARRAY_EXPRESSION_POLICY: SheetSpanPolicy =
    SheetSpanPolicy::ReturnExcelError(ErrorKind::Value);

pub(super) fn function_policy(normalized_name: &str) -> SheetSpanPolicy {
    match normalized_name {
        "SUM" | "AVERAGE" | "AVERAGEA" | "COUNT" | "COUNTA" | "MAX" | "MAXA" | "MIN" | "MINA"
        | "PRODUCT" | "STDEV.P" | "STDEV.S" | "VAR.P" | "VAR.S" => {
            SheetSpanPolicy::CollectAcrossSheets
        }
        "INDEX" | "VLOOKUP" => SheetSpanPolicy::ReturnExcelError(ErrorKind::Value),
        "OFFSET" => SheetSpanPolicy::ReturnExcelError(ErrorKind::Ref),
        _ => SheetSpanPolicy::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculation::functions::normalize_name;

    #[test]
    fn policy_is_explicit_for_every_supported_three_d_aggregate() {
        for name in [
            "SUM", "AVERAGE", "AVERAGEA", "COUNT", "COUNTA", "MAX", "MAXA", "MIN", "MINA",
            "PRODUCT", "STDEV.P", "STDEV.S", "VAR.P", "VAR.S",
        ] {
            assert_eq!(
                function_policy(name),
                SheetSpanPolicy::CollectAcrossSheets,
                "{name}",
            );
        }
    }

    #[test]
    fn legacy_statistical_aliases_share_the_canonical_policy() {
        for alias in ["STDEV", "STDEVP", "VAR", "VARP"] {
            assert_eq!(
                function_policy(&normalize_name(alias)),
                SheetSpanPolicy::CollectAcrossSheets,
                "{alias}",
            );
        }
    }

    #[test]
    fn excel_error_and_unsupported_contexts_remain_distinct() {
        assert_eq!(
            function_policy("INDEX"),
            SheetSpanPolicy::ReturnExcelError(ErrorKind::Value)
        );
        assert_eq!(
            function_policy("VLOOKUP"),
            SheetSpanPolicy::ReturnExcelError(ErrorKind::Value)
        );
        assert_eq!(
            function_policy("OFFSET"),
            SheetSpanPolicy::ReturnExcelError(ErrorKind::Ref)
        );
        assert_eq!(function_policy("COUNTBLANK"), SheetSpanPolicy::Unsupported);
        assert_eq!(function_policy("NOPE"), SheetSpanPolicy::Unsupported);
    }
}
