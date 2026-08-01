use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::array::{calculated_array_result, observed_array_result_comparison};
use super::{calculated_result, load_oracle, observed_result, observed_source_value, source_value};
use cellrune::CalculationCellResult;
use cellrune_integration_tests::oracle::{
    Classification, Expectation, ObservedValue, values_match,
};

pub(super) fn report(directory: &Path, output_path: Option<&Path>) -> Result<(), Vec<String>> {
    let loaded = load_oracle(directory, false).map_err(|error| vec![error])?;
    let extra = loaded
        .expectations
        .keys()
        .filter(|key| !loaded.selected.contains_key(*key))
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(vec![format!(
            "{}: existing expectations contain keys outside the selected manifest: {extra:?}",
            directory.display()
        )]);
    }
    let mut cases = BTreeMap::new();
    for (key, id) in &loaded.selected {
        let existing = loaded.expectations.get(key);
        let observation = loaded
            .observations
            .as_ref()
            .and_then(|observations| observations.get(key));
        let saved_source = match observation {
            Some(observation) => observed_source_value(observation),
            None => source_value(&loaded.workbook, *id)
                .map_err(|error| vec![format!("{key}: {error}")])?,
        };
        let source = saved_source;
        let excel_rich_error =
            observation.is_some_and(|observation| observation.rich_error.present);
        let result = calculated_result(&loaded, *id);
        let mut expectation = report_expectation(existing, source, result, excel_rich_error)
            .map_err(|error| vec![format!("{key}: {error}")])?;
        if let Some(observation) = observation
            && observation.result.is_some()
            && matches!(
                expectation.classification,
                Classification::Match | Classification::Divergent
            )
        {
            let calculated_array =
                calculated_array_result(&loaded.workbook, &loaded.calculation, *id)
                    .map_err(|error| vec![format!("{key}: {error}")])?;
            let (mismatch_count, signature_mismatch_count) =
                observed_array_result_comparison(observation, &calculated_array);
            let matches_all = mismatch_count == 0;
            if !matches_all {
                let calculated_signature =
                    (signature_mismatch_count > 0).then(|| calculated_array.clone());
                let reviewed_note = existing
                    .filter(|value| value.classification == Classification::Divergent)
                    .filter(
                        |value| match (&value.cellrune_result, &calculated_signature) {
                            (Some(recorded), Some(calculated)) => recorded == calculated,
                            (None, _) => true,
                            (Some(_), None) => false,
                        },
                    )
                    .and_then(|value| value.note.clone());
                if expectation.classification == Classification::Match {
                    expectation.classification = Classification::Divergent;
                    if let Some(actual) = calculated_result(&loaded, *id)
                        .and_then(|result| observed_result(Some(result)).ok().flatten())
                    {
                        expectation.cellrune_value = Some(actual.value);
                        expectation.cellrune_type = Some(actual.value_type);
                    }
                }
                expectation.cellrune_result = calculated_signature;
                if let Some(note) = reviewed_note {
                    expectation.note = Some(note);
                } else if expectation.note.is_none() {
                    expectation.note = Some(
                        "CellRune produces a different array result for this oracle case; retain until the underlying calculation semantics are corrected."
                            .to_owned(),
                    );
                }
            } else if matches_all && expectation.classification == Classification::Divergent {
                expectation.classification = Classification::Match;
                expectation.cellrune_value = None;
                expectation.cellrune_type = None;
                expectation.cellrune_result = None;
                expectation.note = None;
            }
        }
        cases.insert(key.clone(), expectation);
    }
    let output = serde_json::to_string_pretty(&cases)
        .map_err(|error| vec![format!("cannot serialize report: {error}")])?;
    if let Some(path) = output_path {
        fs::write(path, format!("{output}\n"))
            .map_err(|error| vec![format!("{}: {error}", path.display())])?;
    } else {
        println!("{output}");
    }
    Ok(())
}

fn report_expectation(
    existing: Option<&Expectation>,
    source: Option<ObservedValue>,
    result: Option<&CalculationCellResult>,
    excel_rich_error: bool,
) -> Result<Expectation, String> {
    let expected = source.clone().unwrap_or(ObservedValue {
        value: String::new(),
        value_type: "blank".to_owned(),
    });
    if let Some(existing) = existing
        && existing.classification == Classification::HostUnsupported
        && (source.is_none()
            || (excel_rich_error
                && source
                    .as_ref()
                    .is_some_and(|value| value.value_type == "e" && value.value == "#NAME?")))
    {
        let mut reviewed = existing.clone();
        reviewed.excel_value = expected.value;
        reviewed.excel_type = expected.value_type;
        reviewed.excel_rich_error = excel_rich_error;
        reviewed.note = Some(match source {
            None => "This required Excel host saved no semantic cache for the active oracle case."
                .to_owned(),
            Some(ref value) => format!(
                "This required Excel host does not implement the formula surface and saved {}.",
                value.value
            ),
        });
        return Ok(reviewed);
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
                Classification::HostUnsupported,
                None,
                None,
                Some(
                    "This host saved no semantic cache for the active oracle case."
                        .to_owned(),
                ),
            ),
            (Some(expected), _)
                if excel_rich_error
                    && expected.value_type == "e"
                    && expected.value == "#NAME?" =>
            {
                (
                    Classification::HostUnsupported,
                    None,
                    None,
                    Some(
                        "This required Excel host does not implement the formula surface and saved #NAME?."
                            .to_owned(),
                    ),
                )
            }
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
        excel_rich_error,
        cellrune_value,
        cellrune_type,
        cellrune_result: None,
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
            cellrune_result: None,
            comparator: Some(comparator),
            note: None,
        }
    }

    fn host_unsupported() -> Expectation {
        Expectation {
            classification: Classification::HostUnsupported,
            excel_value: "#NAME?".to_owned(),
            excel_type: "e".to_owned(),
            excel_rich_error: true,
            cellrune_value: None,
            cellrune_type: None,
            cellrune_result: None,
            comparator: None,
            note: Some("Legacy host observation.".to_owned()),
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
        let reported = report_expectation(Some(&existing), Some(source), Some(&result), false)
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

    #[test]
    fn semantic_cache_replaces_stale_host_unsupported_classification() {
        let existing = host_unsupported();
        let source = ObservedValue {
            value: "2".to_owned(),
            value_type: "n".to_owned(),
        };
        let result = CalculationCellResult::Value(CellValue::Number(
            FiniteNumber::new(2.0).expect("finite expected value"),
        ));

        let reported = report_expectation(Some(&existing), Some(source), Some(&result), false)
            .expect("report expectation");

        assert_eq!(reported.classification, Classification::Match);
        assert_eq!(reported.excel_value, "2");
        assert_eq!(reported.excel_type, "n");
        assert!(!reported.excel_rich_error);
        assert_eq!(reported.note, None);
    }

    #[test]
    fn missing_semantic_cache_is_host_unsupported() {
        let result = CalculationCellResult::Value(CellValue::Number(
            FiniteNumber::new(2.0).expect("finite result"),
        ));

        let reported =
            report_expectation(None, None, Some(&result), false).expect("report expectation");

        assert_eq!(reported.classification, Classification::HostUnsupported);
        assert_eq!(reported.excel_type, "blank");
        assert_eq!(
            reported.note.as_deref(),
            Some("This host saved no semantic cache for the active oracle case.")
        );
    }

    #[test]
    fn resolved_rich_name_error_is_host_unsupported() {
        let source = ObservedValue {
            value: "#NAME?".to_owned(),
            value_type: "e".to_owned(),
        };

        let reported =
            report_expectation(None, Some(source), None, true).expect("report expectation");

        assert_eq!(reported.classification, Classification::HostUnsupported);
        assert!(reported.excel_rich_error);
    }

    #[test]
    fn stale_host_unsupported_does_not_survive_another_excel_error() {
        let existing = host_unsupported();
        let source = ObservedValue {
            value: "#VALUE!".to_owned(),
            value_type: "e".to_owned(),
        };

        let reported = report_expectation(Some(&existing), Some(source), None, true)
            .expect("report expectation");

        assert_eq!(reported.classification, Classification::NotImplemented);
        assert_eq!(reported.excel_value, "#VALUE!");
    }
}
