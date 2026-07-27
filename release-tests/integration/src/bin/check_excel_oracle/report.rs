use std::collections::BTreeMap;
use std::path::Path;

use cellrune::CalculationCellResult;
use cellrune_integration_tests::oracle::{
    Classification, Expectation, Expectations, ObservedValue, values_match,
};
use serde::Serialize;

use super::{calculated_result, load_oracle, observed_result, source_value};

#[derive(Debug, Serialize)]
struct Report {
    workbook: String,
    cases: Expectations,
}

pub(super) fn report(directory: &Path) -> Result<(), Vec<String>> {
    let loaded = load_oracle(directory, false).map_err(|error| vec![error])?;
    let mut cases = BTreeMap::new();
    for (key, id) in &loaded.selected {
        let existing = loaded.expectations.get(key);
        let saved_source =
            source_value(&loaded.workbook, *id).map_err(|error| vec![format!("{key}: {error}")])?;
        let source = existing
            .filter(|expectation| expectation.excel_rich_error && saved_source.is_some())
            .map_or(saved_source, |expectation| {
                Some(ObservedValue::from_expectation(expectation))
            });
        let result = calculated_result(&loaded, *id);
        cases.insert(
            key.clone(),
            report_expectation(existing, source, result)
                .map_err(|error| vec![format!("{key}: {error}")])?,
        );
    }
    let report = Report {
        workbook: loaded.metadata.workbook,
        cases,
    };
    let output = serde_json::to_string_pretty(&report)
        .map_err(|error| vec![format!("cannot serialize report: {error}")])?;
    println!("{output}");
    Ok(())
}

fn report_expectation(
    existing: Option<&Expectation>,
    source: Option<ObservedValue>,
    result: Option<&CalculationCellResult>,
) -> Result<Expectation, String> {
    let expected = source.clone().unwrap_or(ObservedValue {
        value: String::new(),
        value_type: "blank".to_owned(),
    });
    if let Some(existing) = existing
        && matches!(
            existing.classification,
            Classification::HostUnsupported | Classification::Excluded | Classification::Unreadable
        )
    {
        return Ok(existing.clone());
    }
    let unavailable_note = result.and_then(|result| match result {
        CalculationCellResult::Unavailable(issue) => Some(match issue.detail() {
            Some(detail) => format!(
                "CellRune reports {} ({detail}) for this oracle case.",
                issue.code().as_str()
            ),
            None => format!(
                "CellRune reports {} for this oracle case.",
                issue.code().as_str()
            ),
        }),
        CalculationCellResult::Value(_) => None,
    });
    let actual = observed_result(result)?;
    let comparator = existing.and_then(|value| value.comparator);
    let (classification, cellrune_value, cellrune_type, note) =
        match (source, actual) {
            (None, _) => (
                Classification::Excluded,
                None,
                None,
                Some("Saved workbook contains no comparable cache value.".to_owned()),
            ),
            (Some(expected), Some(actual)) if values_match(&actual, &expected, comparator)? => {
                (Classification::Match, None, None, None)
            }
            (Some(_), Some(actual)) => (
                Classification::Divergent,
                Some(actual.value),
                Some(actual.value_type),
                Some(
                    "CellRune produces a different scalar for this oracle case; retain until the \
                 underlying calculation semantics are corrected."
                        .to_owned(),
                ),
            ),
            (Some(_), None) => (
                Classification::NotImplemented,
                None,
                None,
                Some(unavailable_note.unwrap_or_else(|| {
                    "CellRune produced no result for this oracle case.".to_owned()
                })),
            ),
        };
    Ok(Expectation {
        classification,
        excel_value: expected.value,
        excel_type: expected.value_type,
        excel_rich_error: existing.is_some_and(|value| value.excel_rich_error),
        cellrune_value,
        cellrune_type,
        comparator,
        note,
    })
}

#[cfg(test)]
mod tests {
    use std::mem::discriminant;

    use cellrune::{CalculationCellResult, CellValue, FiniteNumber};
    use cellrune_integration_tests::oracle::{
        Classification, Comparator, Expectation, ObservedValue,
    };

    use super::report_expectation;

    fn existing(comparator: Comparator) -> Expectation {
        Expectation {
            classification: Classification::Match,
            excel_value: "0".to_owned(),
            excel_type: "n".to_owned(),
            excel_rich_error: false,
            cellrune_value: None,
            cellrune_type: None,
            comparator: Some(comparator),
            note: None,
        }
    }

    fn assert_strict_comparator_is_used(comparator: Comparator) {
        let existing = existing(comparator);
        let source = ObservedValue {
            value: "0".to_owned(),
            value_type: "n".to_owned(),
        };
        let result = CalculationCellResult::Value(CellValue::Number(
            FiniteNumber::new(5.551_115_123_125_783e-17).expect("finite residue"),
        ));
        let reported = report_expectation(Some(&existing), Some(source), Some(&result))
            .expect("report expectation");

        assert_eq!(reported.classification, Classification::Divergent);
        assert_eq!(
            discriminant(&reported.comparator.expect("preserved comparator")),
            discriminant(&comparator)
        );
    }

    #[test]
    fn report_classification_honors_each_existing_strict_numeric_comparator() {
        for comparator in [
            Comparator::Exact {},
            Comparator::ExactBits {},
            Comparator::Scaled { epsilon: 0.0 },
            Comparator::AbsRel { abs: 0.0, rel: 0.0 },
        ] {
            assert_strict_comparator_is_used(comparator);
        }
    }
}
