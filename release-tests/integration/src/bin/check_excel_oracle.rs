//! Audit and reporting tool for committed Excel-saved workbook oracles.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cellrune::{
    CalculationCellId, CalculationCellResult, CellAddress, CellContent, CellValue, ReadOptions,
    SavedResult, WorkbookSnapshot, calculate_workbook, read_xlsx_path,
};
use cellrune_integration_tests::oracle::{
    CASE_MANIFEST_SCHEMA, CacheStatus, CaseManifest, Classification, Comparator, Expectation,
    Expectations, HostProfile, METADATA_SCHEMA, Metadata, OBSERVATIONS_SCHEMA, Observations,
    ObservedCase, ObservedValue, OracleMetadata, OracleSuite, SUITE_SCHEMA, values_match,
};

#[path = "check_excel_oracle/raw.rs"]
mod raw;
#[path = "check_excel_oracle/report.rs"]
mod report;
#[path = "check_excel_oracle/rewrite.rs"]
mod rewrite;
#[path = "check_excel_oracle/selection.rs"]
mod selection;
#[path = "check_excel_oracle/shared.rs"]
mod shared;

const USAGE: &str = "usage: check_excel_oracle [--report <oracle-directory> [output.json]]";
const METADATA_FILE: &str = "metadata.json";
const EXPECTATIONS_FILE: &str = "expectations.json";
const SUITE_FILE: &str = "suite.json";
const OBSERVATIONS_FILE: &str = "observations.json";
const MESSAGE_NO_ORACLES: &str = "no oracle metadata files found";
const MESSAGE_EXPECTATION_KEYS: &str =
    "expectation keys must exactly equal the selected workbook cases";
const MESSAGE_UNCLASSIFIED: &str = "unclassified oracle case";
const MESSAGE_WORKBOOK_FILENAME: &str = "workbook must be a filename within the oracle directory";
const MESSAGE_ITERATIVE_CALCULATION: &str =
    "workbook iterative-calculation setting does not match metadata";
const MESSAGE_SUITE_REQUIRED: &str =
    "suite.json is required when metadata declares a suite or host-profile identity";

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [] => audit_all(&oracle_root()),
        [flag, directory] if flag == "--report" => report::report(Path::new(directory), None),
        [flag, directory, output] if flag == "--report" => {
            report::report(Path::new(directory), Some(Path::new(output)))
        }
        _ => Err(vec![USAGE.to_owned()]),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problems) => {
            for problem in problems {
                eprintln!("error: {problem}");
            }
            ExitCode::FAILURE
        }
    }
}

fn oracle_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn audit_all(root: &Path) -> Result<(), Vec<String>> {
    let mut suite_files = Vec::new();
    collect_named_files(root, SUITE_FILE, &mut suite_files)?;
    suite_files.sort();
    let mut managed_metadata = BTreeSet::new();
    let mut problems = Vec::new();
    for suite_path in suite_files {
        audit_suite(&suite_path, &mut managed_metadata, &mut problems);
    }

    let mut metadata_files = Vec::new();
    collect_metadata(root, &mut metadata_files)?;
    metadata_files.sort();
    if metadata_files.is_empty() {
        problems.push(format!("{}: {MESSAGE_NO_ORACLES}", root.display()));
        return Err(problems);
    }
    for metadata_path in metadata_files {
        if managed_metadata.contains(&metadata_path) {
            continue;
        }
        match load_oracle(
            metadata_path
                .parent()
                .expect("metadata path always has a parent"),
            true,
        ) {
            Ok(loaded) => {
                let counts = audit_loaded(&loaded, &mut problems);
                println!(
                    "{}: cases={} match={} divergent={} not_implemented={} host_unsupported={} excluded={}",
                    loaded.directory.display(),
                    counts.total,
                    counts.matched,
                    counts.divergent,
                    counts.not_implemented,
                    counts.host_unsupported,
                    counts.excluded,
                );
            }
            Err(error) => problems.push(error),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn collect_named_files(
    directory: &Path,
    file_name: &str,
    output: &mut Vec<PathBuf>,
) -> Result<(), Vec<String>> {
    let entries = fs::read_dir(directory)
        .map_err(|error| vec![format!("{}: {error}", directory.display())])?;
    for entry in entries {
        let path = entry
            .map_err(|error| vec![format!("{}: {error}", directory.display())])?
            .path();
        if path.is_dir() {
            collect_named_files(&path, file_name, output)?;
        } else if path.file_name().is_some_and(|name| name == file_name) {
            output.push(path);
        }
    }
    Ok(())
}

fn collect_metadata(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), Vec<String>> {
    collect_named_files(directory, METADATA_FILE, output)
}

fn audit_suite(
    suite_path: &Path,
    managed_metadata: &mut BTreeSet<PathBuf>,
    problems: &mut Vec<String>,
) {
    let suite_directory = suite_path.parent().expect("suite path always has a parent");
    let suite: OracleSuite = match read_json(suite_path) {
        Ok(suite) => suite,
        Err(error) => {
            problems.push(error);
            return;
        }
    };
    if let Err(error) = validate_suite_contract(&suite) {
        problems.push(format!("{}: {error}", suite_path.display()));
        return;
    }
    if !is_filename(&suite.case_manifest) {
        problems.push(format!(
            "{}: case manifest must be a filename",
            suite_path.display()
        ));
        return;
    }
    let manifest_path = suite_directory.join(&suite.case_manifest);
    let manifest: CaseManifest = match read_json(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            problems.push(error);
            return;
        }
    };
    if let Err(error) = validate_manifest_contract(&suite, &manifest) {
        problems.push(format!("{}: {error}", manifest_path.display()));
        return;
    }

    for profile in &suite.profiles {
        let directory = suite_directory.join(&profile.directory);
        let metadata_path = directory.join(METADATA_FILE);
        managed_metadata.insert(metadata_path.clone());
        if !metadata_path.is_file() {
            problems.push(format!("{}: file is required", metadata_path.display()));
            continue;
        }
        match load_oracle(&directory, true) {
            Ok(loaded) => {
                let counts = audit_loaded(&loaded, problems);
                println!(
                    "{}: cases={} match={} divergent={} not_implemented={} host_unsupported={} excluded={}",
                    loaded.directory.display(),
                    counts.total,
                    counts.matched,
                    counts.divergent,
                    counts.not_implemented,
                    counts.host_unsupported,
                    counts.excluded,
                );
            }
            Err(error) => problems.push(error),
        }
    }
}

fn validate_suite_contract(suite: &OracleSuite) -> Result<(), String> {
    if suite.schema != SUITE_SCHEMA || suite.suite_id.is_empty() {
        return Err(format!("unsupported suite schema {}", suite.schema));
    }
    if !is_filename(&suite.case_manifest) {
        return Err("suite case manifest must be a filename".to_owned());
    }
    let mut profile_ids = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for profile in &suite.profiles {
        if profile.profile_id.is_empty() || !profile_ids.insert(profile.profile_id.as_str()) {
            return Err(format!("duplicate host profile {}", profile.profile_id));
        }
        if !is_filename(&profile.directory) || !directories.insert(profile.directory.as_str()) {
            return Err(format!(
                "duplicate or invalid host profile directory {}",
                profile.directory
            ));
        }
        if profile.application.is_empty() {
            return Err(format!("incomplete host profile {}", profile.profile_id));
        }
    }
    if suite.profiles.is_empty() {
        return Err("suite must list at least one workbook profile".to_owned());
    }
    Ok(())
}

fn validate_manifest_contract(suite: &OracleSuite, manifest: &CaseManifest) -> Result<(), String> {
    if manifest.schema != CASE_MANIFEST_SCHEMA || manifest.suite_id != suite.suite_id {
        return Err("case manifest identity does not match suite".to_owned());
    }
    let profile_ids = suite
        .profiles
        .iter()
        .map(|profile| profile.profile_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut keys = BTreeSet::new();
    let mut catalog_addresses = BTreeSet::new();
    let mut formula_addresses = BTreeSet::new();
    for oracle_case in &manifest.cases {
        if oracle_case.case_key.is_empty() || !keys.insert(oracle_case.case_key.as_str()) {
            return Err(format!(
                "duplicate or empty stable case key {}",
                oracle_case.case_key
            ));
        }
        if oracle_case.function.is_empty()
            || oracle_case.category.is_empty()
            || oracle_case.scenario.is_empty()
            || oracle_case.seed_classification.is_empty()
            || !catalog_addresses.insert(oracle_case.catalog_address.as_str())
        {
            return Err(format!(
                "incomplete or duplicate manifest record {}",
                oracle_case.case_key
            ));
        }
        if oracle_case.authored_formula.is_empty() || oracle_case.storage_formula.is_empty() {
            return Err(format!("empty formula in {}", oracle_case.case_key));
        }
        rewrite::validate_declarations(&oracle_case.allowed_host_rewrites, &profile_ids)
            .map_err(|error| format!("{}: {error}", oracle_case.case_key))?;
        match (oracle_case.active, oracle_case.formula_address.as_deref()) {
            (true, Some(address))
                if oracle_case.exclusion.is_none() && formula_addresses.insert(address) => {}
            (false, None)
                if oracle_case
                    .inactive_reason
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
                    && oracle_case.exclusion.is_some() => {}
            _ => {
                return Err(format!(
                    "active/formula-address invariant failed for {}",
                    oracle_case.case_key
                ));
            }
        }
        if oracle_case.exclusion.is_some() && oracle_case.active {
            return Err(format!(
                "excluded case is unexpectedly active: {}",
                oracle_case.case_key
            ));
        }
    }
    Ok(())
}

struct LoadedOracle {
    directory: PathBuf,
    expectations: Expectations,
    workbook: WorkbookSnapshot,
    selected: BTreeMap<String, CalculationCellId>,
    observations: Option<BTreeMap<String, ObservedCase>>,
    calculation: cellrune::CalculationSnapshot,
}

fn load_oracle(directory: &Path, require_expectations: bool) -> Result<LoadedOracle, String> {
    let metadata_path = directory.join(METADATA_FILE);
    let metadata: Metadata = read_json(&metadata_path)?;
    if metadata.schema != METADATA_SCHEMA {
        return Err(format!(
            "{}: unsupported metadata schema {}",
            metadata_path.display(),
            metadata.schema
        ));
    }
    let suite_path = resolve_suite_path(directory, &metadata.oracle)?;
    if metadata.workbook.is_empty()
        || metadata.workbook == "."
        || metadata.workbook == ".."
        || metadata.workbook.contains('/')
        || metadata.workbook.contains('\\')
    {
        return Err(format!(
            "{}: {MESSAGE_WORKBOOK_FILENAME}",
            metadata_path.display()
        ));
    }
    let workbook_path = directory.join(&metadata.workbook);
    raw::verify_package_invariants(&workbook_path)?;
    let workbook = read_xlsx_path(&workbook_path, ReadOptions::default())
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let formula_cells = workbook
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter(|cell| matches!(cell.content(), CellContent::Formula(_)))
        .count();
    if formula_cells != metadata.formula_cells {
        return Err(format!(
            "{}: formula cell count {} != metadata {}",
            workbook_path.display(),
            formula_cells,
            metadata.formula_cells
        ));
    }
    let actual_date_system = match workbook.date_system() {
        cellrune::DateSystem::Excel1900 => "excel1900",
        cellrune::DateSystem::Excel1904 => "excel1904",
    };
    if metadata.date_system != actual_date_system {
        return Err(format!(
            "{}: date system {} != metadata {}",
            workbook_path.display(),
            actual_date_system,
            metadata.date_system
        ));
    }
    verify_iterative_calculation(
        &workbook_path,
        metadata.iterative_calculation,
        workbook.calculation_hints().iterative_calculation(),
    )?;
    let selected_by_address = selection::select_cases(&workbook, &metadata.case_selection)?;
    let (selected, observations) = match suite_path {
        Some(path) => {
            let binding = load_suite_binding(
                directory,
                &path,
                &metadata,
                &workbook_path,
                &workbook,
                selected_by_address,
            )?;
            (binding.selected, Some(binding.observations))
        }
        None => (selected_by_address, None),
    };
    let expectations_path = directory.join(EXPECTATIONS_FILE);
    let expectations = if expectations_path.exists() {
        read_json(&expectations_path)?
    } else if require_expectations {
        return Err(format!("{}: file is required", expectations_path.display()));
    } else {
        Expectations::new()
    };
    let calculation = calculate_workbook(&workbook, cellrune::CalculationOptions::default());
    Ok(LoadedOracle {
        directory: directory.to_path_buf(),
        expectations,
        workbook,
        selected,
        observations,
        calculation,
    })
}

fn resolve_suite_path(
    directory: &Path,
    oracle: &OracleMetadata,
) -> Result<Option<PathBuf>, String> {
    let suite_path = directory.parent().map(|parent| parent.join(SUITE_FILE));
    if suite_path.as_ref().is_some_and(|path| path.is_file()) {
        return Ok(suite_path);
    }
    if oracle.suite_id.is_some() || oracle.host_profile_id.is_some() {
        return Err(format!(
            "{}: {MESSAGE_SUITE_REQUIRED}",
            directory.join(METADATA_FILE).display()
        ));
    }
    Ok(None)
}

struct SuiteBinding {
    selected: BTreeMap<String, CalculationCellId>,
    observations: BTreeMap<String, ObservedCase>,
}

fn load_suite_binding(
    directory: &Path,
    suite_path: &Path,
    metadata: &Metadata,
    workbook_path: &Path,
    workbook: &WorkbookSnapshot,
    mut selected_by_address: BTreeMap<String, CalculationCellId>,
) -> Result<SuiteBinding, String> {
    let suite: OracleSuite = read_json(suite_path)?;
    validate_suite_contract(&suite)
        .map_err(|error| format!("{}: {error}", suite_path.display()))?;
    let directory_name = directory
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "{}: oracle directory name is not UTF-8",
                directory.display()
            )
        })?;
    let profile = suite
        .profiles
        .iter()
        .find(|profile| profile.directory == directory_name)
        .ok_or_else(|| {
            format!(
                "{}: directory is not declared by suite {}",
                directory.display(),
                suite.suite_id
            )
        })?;
    validate_profile_metadata(metadata, &suite, profile, directory)?;

    let suite_directory = suite_path.parent().expect("suite path always has a parent");
    if !is_filename(&suite.case_manifest) {
        return Err(format!(
            "{}: case manifest must be a filename",
            suite_path.display()
        ));
    }
    let manifest_path = suite_directory.join(&suite.case_manifest);
    let manifest: CaseManifest = read_json(&manifest_path)?;
    validate_manifest_contract(&suite, &manifest)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;

    let observations_path = directory.join(OBSERVATIONS_FILE);
    let observations: Observations = read_json(&observations_path)?;
    validate_observation_header(&observations, &suite, profile, &observations_path)?;
    let raw_cells = raw::read_formula_cells(workbook_path)?;
    if raw_cells.len() != metadata.formula_cells {
        return Err(format!(
            "{}: raw formula cell count {} != metadata {}",
            workbook_path.display(),
            raw_cells.len(),
            metadata.formula_cells
        ));
    }
    for oracle_case in &manifest.cases {
        verify_catalog_identity(
            workbook,
            &oracle_case.catalog_address,
            &oracle_case.case_key,
        )?;
    }

    let active_cases = manifest
        .cases
        .iter()
        .filter(|oracle_case| oracle_case.active)
        .map(|oracle_case| (oracle_case.case_key.as_str(), oracle_case))
        .collect::<BTreeMap<_, _>>();
    let mut observed_by_key = BTreeMap::new();
    for observation in observations.cases {
        if observed_by_key
            .insert(observation.case_key.clone(), observation)
            .is_some()
        {
            return Err(format!(
                "{}: duplicate stable case observation",
                observations_path.display()
            ));
        }
    }
    let active_keys = active_cases.keys().copied().collect::<BTreeSet<_>>();
    let observed_keys = observed_by_key
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if observed_keys != active_keys {
        return Err(format!(
            "{}: observations do not exactly cover active manifest cases",
            observations_path.display()
        ));
    }

    let mut selected = BTreeMap::new();
    for (case_key, oracle_case) in active_cases {
        let formula_address = oracle_case
            .formula_address
            .as_deref()
            .expect("validated active manifest case has a formula address");
        let id = selected_by_address.remove(formula_address).ok_or_else(|| {
            format!(
                "{}: active manifest address is not selected: {formula_address}",
                manifest_path.display()
            )
        })?;
        let observation = observed_by_key
            .get(case_key)
            .expect("observations exactly cover active keys");
        validate_observed_case(
            observation,
            oracle_case,
            raw_cells.get(formula_address),
            &observations_path,
            &profile.profile_id,
        )?;
        selected.insert(case_key.to_owned(), id);
    }
    if !selected_by_address.is_empty() {
        return Err(format!(
            "{}: selected workbook formulas are absent from the active manifest: {:?}",
            manifest_path.display(),
            selected_by_address.keys().take(10).collect::<Vec<_>>()
        ));
    }
    Ok(SuiteBinding {
        selected,
        observations: observed_by_key,
    })
}

fn validate_profile_metadata(
    metadata: &Metadata,
    suite: &OracleSuite,
    profile: &HostProfile,
    directory: &Path,
) -> Result<(), String> {
    let oracle = &metadata.oracle;
    let matches = oracle.suite_id.as_deref() == Some(suite.suite_id.as_str())
        && oracle.host_profile_id.as_deref() == Some(profile.profile_id.as_str());
    if !matches {
        return Err(format!(
            "{}: metadata host identity does not match suite profile {}",
            directory.display(),
            profile.profile_id
        ));
    }
    Ok(())
}

fn validate_observation_header(
    observations: &Observations,
    suite: &OracleSuite,
    profile: &HostProfile,
    observations_path: &Path,
) -> Result<(), String> {
    if observations.schema != OBSERVATIONS_SCHEMA
        || observations.suite_id != suite.suite_id
        || observations.host_profile_id != profile.profile_id
    {
        return Err(format!(
            "{}: observation identity does not match suite profile",
            observations_path.display()
        ));
    }
    Ok(())
}

fn validate_observed_case(
    observation: &ObservedCase,
    oracle_case: &cellrune_integration_tests::oracle::ManifestCase,
    raw_cell: Option<&raw::RawFormulaCell>,
    observations_path: &Path,
    profile_id: &str,
) -> Result<(), String> {
    let context = format!("{}: {}", observations_path.display(), observation.case_key);
    let expected_address = oracle_case
        .formula_address
        .as_deref()
        .expect("active manifest case has a formula address");
    if observation.address != expected_address {
        return Err(format!(
            "{context}: observation does not join the active manifest record"
        ));
    }
    let raw_cell =
        raw_cell.ok_or_else(|| format!("{context}: saved formula cell is absent from raw XLSX"))?;
    let accepted_rewrites = rewrite::accepted_rewrites(
        &oracle_case.storage_formula,
        &raw_cell.formula,
        &oracle_case.allowed_host_rewrites,
        profile_id,
    )
    .map_err(|error| format!("{context}: {error}"))?;
    if raw_cell.formula != observation.saved_formula
        || raw_cell.value != observation.cache_value
        || raw_cell.value_type != observation.cache_type
        || raw_cell.vm_index != observation.rich_error.vm_index
        || observation.formula_rewrites != accepted_rewrites
    {
        return Err(format!(
            "{context}: observation does not exact-match raw XLSX formula/cache metadata"
        ));
    }
    let expected_status = if raw_cell
        .value
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        CacheStatus::Semantic
    } else {
        CacheStatus::MissingSemanticCache
    };
    if observation.cache_status != expected_status
        || observation.cache_status == CacheStatus::Circular
    {
        return Err(format!(
            "{context}: active-case cache status does not match the raw XLSX"
        ));
    }
    let expected_rich = raw_cell.rich_error.is_some();
    let expected_error = (raw_cell.value_type == "e")
        .then(|| raw_cell.value.clone())
        .flatten();
    let raw_rich = raw_cell.rich_error.as_ref();
    if observation.rich_error.present != expected_rich
        || observation.rich_error.raw_error != expected_error
        || observation.rich_error.record_index != raw_rich.map(|record| record.record_index)
        || observation.rich_error.structure_index != raw_rich.map(|record| record.structure_index)
        || observation.rich_error.error_type_code
            != raw_rich.and_then(|record| record.error_type_code)
        || observation.rich_error.resolved_error
            != raw_rich.and_then(|record| record.resolved_error.clone())
        || observation.rich_error.fallback_error
            != raw_rich.and_then(|record| record.fallback_error.clone())
    {
        return Err(format!(
            "{context}: rich-error observation does not match raw XLSX metadata"
        ));
    }
    Ok(())
}

fn verify_catalog_identity(
    workbook: &WorkbookSnapshot,
    catalog_address: &str,
    expected_key: &str,
) -> Result<(), String> {
    let (sheet_name, address) = split_qualified_address(catalog_address)?;
    let sheet = workbook
        .sheet_by_name(sheet_name)
        .ok_or_else(|| format!("{catalog_address}: unknown catalog sheet"))?;
    let address = CellAddress::from_a1(address)
        .map_err(|error| format!("{catalog_address}: invalid catalog address: {error:?}"))?;
    let actual = sheet.cell(address).and_then(|cell| match cell.content() {
        CellContent::Literal(CellValue::Text(value)) => Some(value.as_str()),
        _ => None,
    });
    if actual == Some(expected_key) {
        Ok(())
    } else {
        Err(format!(
            "{catalog_address}: stable case key is {:?}, expected {expected_key}",
            actual
        ))
    }
}

fn split_qualified_address(value: &str) -> Result<(&str, &str), String> {
    let (sheet, address) = value
        .rsplit_once('!')
        .ok_or_else(|| format!("{value}: expected Sheet!A1 address"))?;
    if sheet.is_empty() || address.is_empty() {
        Err(format!("{value}: expected Sheet!A1 address"))
    } else {
        Ok((sheet, address))
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn is_filename(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn verify_iterative_calculation(
    workbook_path: &Path,
    expected: bool,
    declared: Option<bool>,
) -> Result<(), String> {
    let actual = declared.unwrap_or(false);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{}: {MESSAGE_ITERATIVE_CALCULATION}; workbook={actual} metadata={expected}",
            workbook_path.display()
        ))
    }
}

#[derive(Default)]
struct Counts {
    total: usize,
    matched: usize,
    divergent: usize,
    not_implemented: usize,
    host_unsupported: usize,
    excluded: usize,
}

fn audit_loaded(loaded: &LoadedOracle, problems: &mut Vec<String>) -> Counts {
    let selected_keys = loaded.selected.keys().collect::<BTreeSet<_>>();
    let expectation_keys = loaded.expectations.keys().collect::<BTreeSet<_>>();
    if selected_keys != expectation_keys {
        let missing = selected_keys.difference(&expectation_keys).take(10);
        let extra = expectation_keys.difference(&selected_keys).take(10);
        problems.push(format!(
            "{}: {MESSAGE_EXPECTATION_KEYS}; missing={:?} extra={:?}",
            loaded.directory.display(),
            missing.collect::<Vec<_>>(),
            extra.collect::<Vec<_>>()
        ));
    }
    let mut counts = Counts {
        total: loaded.expectations.len(),
        ..Counts::default()
    };
    for (key, expectation) in &loaded.expectations {
        let context = format!("{}: {key}", loaded.directory.display());
        let Some(id) = loaded.selected.get(key).copied() else {
            continue;
        };
        if let Some(observation) = loaded
            .observations
            .as_ref()
            .and_then(|observations| observations.get(key))
        {
            if observation.cache_status == CacheStatus::Circular {
                problems.push(format!(
                    "{context}: active manifest case cannot use a circular cache"
                ));
            } else if is_host_unsupported_observation(observation)
                && expectation.classification != Classification::HostUnsupported
            {
                problems.push(format!(
                    "{context}: missing cache or resolved rich #NAME? must be reviewed as host_unsupported"
                ));
            } else if !is_host_unsupported_observation(observation)
                && expectation.classification == Classification::HostUnsupported
            {
                problems.push(format!(
                    "{context}: host_unsupported is stale because this host now has a semantic result"
                ));
            }
            if matches!(
                expectation.classification,
                Classification::Excluded | Classification::Unreadable
            ) {
                problems.push(format!(
                    "{context}: active manifest case cannot be classified as excluded/unreadable"
                ));
            }
        }
        audit_saved_cache(&context, key, loaded, id, expectation, problems);
        let result = calculated_result(loaded, id);
        match expectation.classification {
            Classification::Match => {
                let actual = match observed_result(result) {
                    Ok(Some(actual)) => actual,
                    Ok(None) => {
                        problems.push(format!("{context}: expected a value, got unavailable"));
                        continue;
                    }
                    Err(error) => {
                        problems.push(format!("{context}: {error}"));
                        continue;
                    }
                };
                match values_match(
                    &actual,
                    &ObservedValue::from_expectation(expectation),
                    expectation.comparator,
                ) {
                    Ok(true) => counts.matched += 1,
                    Ok(false) => problems.push(format!(
                        "{context}: expected {:?}, got {actual:?}",
                        ObservedValue::from_expectation(expectation)
                    )),
                    Err(error) => problems.push(format!("{context}: {error}")),
                }
            }
            Classification::Divergent => {
                let actual = match observed_result(result) {
                    Ok(Some(actual)) => actual,
                    Ok(None) => {
                        problems.push(format!("{context}: divergent case did not produce a value"));
                        continue;
                    }
                    Err(error) => {
                        problems.push(format!("{context}: {error}"));
                        continue;
                    }
                };
                let expected = ObservedValue::from_expectation(expectation);
                match values_match(&actual, &expected, expectation.comparator) {
                    Ok(true) => {
                        problems.push(format!("{context}: divergent case now matches Excel"));
                    }
                    Ok(false) => {}
                    Err(error) => problems.push(format!("{context}: {error}")),
                }
                let Some(recorded) = ObservedValue::from_recorded_cellrune(expectation) else {
                    problems.push(format!(
                        "{context}: divergent case lacks CellRune value/type"
                    ));
                    continue;
                };
                match values_match(&actual, &recorded, expectation.comparator) {
                    Ok(true) => counts.divergent += 1,
                    Ok(false) => problems.push(format!(
                        "{context}: CellRune side changed from {recorded:?} to {actual:?}"
                    )),
                    Err(error) => problems.push(format!("{context}: {error}")),
                }
            }
            Classification::NotImplemented => {
                if matches!(result, Some(CalculationCellResult::Unavailable(_))) {
                    counts.not_implemented += 1;
                } else {
                    problems.push(format!("{context}: not-implemented case now calculates"));
                }
            }
            Classification::HostUnsupported => {
                counts.host_unsupported += 1;
            }
            Classification::Excluded | Classification::Unreadable => {
                counts.excluded += 1;
            }
            Classification::Unclassified => {
                problems.push(format!("{context}: {MESSAGE_UNCLASSIFIED}"));
            }
        }
    }
    counts
}

fn audit_saved_cache(
    context: &str,
    key: &str,
    loaded: &LoadedOracle,
    id: CalculationCellId,
    expectation: &Expectation,
    problems: &mut Vec<String>,
) {
    let observation = loaded
        .observations
        .as_ref()
        .and_then(|observations| observations.get(key));
    if let Some(observation) = observation
        && expectation.excel_rich_error != observation.rich_error.present
    {
        problems.push(format!(
            "{context}: expectations rich-error flag does not match observations"
        ));
    }
    let source = observation.map_or_else(
        || source_value(&loaded.workbook, id),
        |observation| Ok(observed_source_value(observation)),
    );
    if matches!(
        expectation.classification,
        Classification::Excluded | Classification::Unreadable | Classification::HostUnsupported
    ) && source.as_ref().is_ok_and(Option::is_none)
    {
        return;
    }
    let Some(source) = source.unwrap_or_else(|error| {
        problems.push(format!("{context}: {error}"));
        None
    }) else {
        problems.push(format!("{context}: saved cache is missing"));
        return;
    };
    let expected = ObservedValue::from_expectation(expectation);
    let comparator = if expected.value_type == "n" {
        Some(Comparator::ExactBits {})
    } else {
        Some(Comparator::Exact {})
    };
    match values_match(&source, &expected, comparator) {
        Ok(true) => {}
        Ok(false) => problems.push(format!(
            "{context}: expectations record {expected:?}, saved cache contains {source:?}"
        )),
        Err(error) => problems.push(format!("{context}: {error}")),
    }
}

fn source_value(
    workbook: &WorkbookSnapshot,
    id: CalculationCellId,
) -> Result<Option<ObservedValue>, String> {
    let sheet = workbook
        .sheet_by_id(id.sheet_id())
        .ok_or_else(|| format!("unknown sheet id {}", id.sheet_id().get()))?;
    let Some(cell) = sheet.cell(id.address()) else {
        return Ok(None);
    };
    match cell.content() {
        CellContent::Literal(value) => ObservedValue::from_cell(value).map(Some),
        CellContent::Formula(formula) => match formula.saved_result() {
            SavedResult::Present(value) => ObservedValue::from_cell(value).map(Some),
            SavedResult::Missing => Ok(None),
            SavedResult::Invalid(issue) => Err(format!(
                "invalid saved result {} ({:?})",
                issue.code().as_str(),
                issue.raw_value()
            )),
        },
    }
}

fn observed_source_value(observation: &ObservedCase) -> Option<ObservedValue> {
    if observation.cache_status != CacheStatus::Semantic {
        return None;
    }
    if observation.rich_error.present {
        return observation
            .rich_error
            .resolved_error
            .as_ref()
            .or(observation.rich_error.fallback_error.as_ref())
            .or(observation.cache_value.as_ref())
            .map(|value| ObservedValue {
                value: value.clone(),
                value_type: "e".to_owned(),
            });
    }
    observation.cache_value.as_ref().map(|value| ObservedValue {
        value: value.clone(),
        value_type: observation.cache_type.clone(),
    })
}

fn is_host_unsupported_observation(observation: &ObservedCase) -> bool {
    match observation.cache_status {
        CacheStatus::MissingSemanticCache => true,
        CacheStatus::Circular => false,
        CacheStatus::Semantic => {
            observation.rich_error.present
                && observed_source_value(observation)
                    .is_some_and(|value| value.value_type == "e" && value.value == "#NAME?")
        }
    }
}

fn calculated_result(
    loaded: &LoadedOracle,
    id: CalculationCellId,
) -> Option<&CalculationCellResult> {
    loaded
        .calculation
        .materialized_cell(id)
        .map(cellrune::MaterializedCalculationCell::result)
        .or_else(|| loaded.calculation.cell(id))
}

fn observed_result(
    result: Option<&CalculationCellResult>,
) -> Result<Option<ObservedValue>, String> {
    result.map_or(Ok(None), ObservedValue::from_result)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cellrune_integration_tests::oracle::OracleMetadata;

    use super::{
        MESSAGE_ITERATIVE_CALCULATION, MESSAGE_SUITE_REQUIRED, audit_all, oracle_root,
        resolve_suite_path, verify_iterative_calculation,
    };

    #[test]
    fn committed_excel_oracles_match_reviewed_expectations() {
        if let Err(problems) = audit_all(&oracle_root()) {
            panic!(
                "committed Excel oracle audit failed:\n{}",
                problems.join("\n")
            );
        }
    }

    #[test]
    fn iterative_calculation_metadata_must_match_effective_workbook_setting() {
        assert!(verify_iterative_calculation(Path::new("workbook.xlsx"), false, None).is_ok());
        assert!(verify_iterative_calculation(Path::new("workbook.xlsx"), true, Some(true)).is_ok());

        let error = verify_iterative_calculation(Path::new("workbook.xlsx"), false, Some(true))
            .expect_err("mismatched iterative calculation");
        assert!(error.contains(MESSAGE_ITERATIVE_CALCULATION));
        assert!(error.contains("workbook=true metadata=false"));
    }

    #[test]
    fn suite_identity_cannot_fall_back_to_address_keys() {
        let oracle = OracleMetadata {
            application: "Microsoft Excel Online".to_owned(),
            version: "AppVersion 16.0300".to_owned(),
            channel: None,
            os: Some("web".to_owned()),
            locale: Some("en-US".to_owned()),
            saved_at: "2026-07-29T00:00:00Z".to_owned(),
            suite_id: Some("cellrune-excel-host-matrix-v1".to_owned()),
            host_profile_id: Some("excel-online".to_owned()),
            product_tier: Some("free".to_owned()),
            host_build: None,
        };
        let error = resolve_suite_path(Path::new("missing-suite/online"), &oracle)
            .expect_err("suite-bound metadata must require its suite");
        assert!(error.contains(MESSAGE_SUITE_REQUIRED));

        let legacy = OracleMetadata {
            suite_id: None,
            host_profile_id: None,
            ..oracle
        };
        assert_eq!(
            resolve_suite_path(Path::new("legacy/oracle"), &legacy)
                .expect("legacy metadata remains address-keyed"),
            None
        );
    }
}
