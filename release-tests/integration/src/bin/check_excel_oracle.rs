//! Audit and reporting tool for committed Excel-saved workbook oracles.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cellrune::{
    CalculationCellId, CalculationCellResult, CellAddress, CellContent, CellRange, CellValue,
    ReadOptions, SavedResult, WorkbookSnapshot, calculate_workbook, read_xlsx_path,
};
use cellrune_integration_tests::oracle::{
    ArtifactReference, CASE_MANIFEST_SCHEMA, CacheStatus, CaseManifest, Classification, Comparator,
    Expectation, Expectations, GeneratorMetadata, HostProfile, METADATA_SCHEMA, Metadata,
    OBSERVATIONS_SCHEMA, Observations, ObservedCase, ObservedValue, OracleMetadata, OracleSuite,
    SUITE_SCHEMA, values_match,
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
const CELLRUNE_SUITE_ID: &str = "cellrune-excel-host-matrix-v1";
const CELLRUNE_FEATURE_SET_ID: &str = "cellrune-excel-function-pool-522-v1";
const CELLRUNE_GENERATOR_NAME: &str = "CellRune deterministic Excel oracle generator";
const CELLRUNE_GENERATOR_REVISION: &str = "oracle-harness-v2";
const CELLRUNE_HARNESS_SHA256: &str =
    "6bd9416b08809e429b20d447afec54f3a8bd79b467d8e3760ccc7f54e8a2b1be";
const CELLRUNE_SOURCE_SHA256: &str =
    "87690d792af82fec157ac4d4316ac1f4a62accb7ca3e5184f8624de43423934f";
const CELLRUNE_ONLINE_SHA256: &str =
    "a576920db7f6ff0d04f75b7c2c568fb8798a74f2a586a83da8cebc07c89d0161";
const CELLRUNE_DESKTOP_SHA256: &str =
    "9e923cb70d2c056a59f5fc2505f789ab8f2497e8f99a470da53b92a913f6b0b7";
const CELLRUNE_PRIMARY_CASES: usize = 1_527;
const CELLRUNE_ACTIVE_CASES: usize = 1_496;
const CELLRUNE_FORMULA_CELLS: usize = 1_892;

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
    if !is_filename(&suite.case_manifest.file) {
        problems.push(format!(
            "{}: case manifest must be a filename",
            suite_path.display()
        ));
        return;
    }
    let manifest_path = suite_directory.join(&suite.case_manifest.file);
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
    if suite.schema != SUITE_SCHEMA || suite.suite_id != CELLRUNE_SUITE_ID {
        return Err(format!("unsupported suite schema {}", suite.schema));
    }
    if !is_filename(&suite.case_manifest.file) || !is_sha256(&suite.case_manifest.sha256) {
        return Err("suite case manifest must be a filename".to_owned());
    }
    if suite.feature_set_id.as_deref() != Some(CELLRUNE_FEATURE_SET_ID)
        || suite
            .planned_release_range
            .as_ref()
            .is_none_or(|range| range.first != "0.1.8" || range.last != "0.1.19")
        || suite.state.as_deref() != Some("frozen")
        || suite.active_case_count != Some(CELLRUNE_ACTIVE_CASES)
        || suite.source_workbook.as_ref().is_none_or(|source| {
            source.sha256 != CELLRUNE_SOURCE_SHA256
                || source.formula_cells != CELLRUNE_FORMULA_CELLS
        })
        || !matches!(
            suite.case_selection,
            Some(cellrune_integration_tests::oracle::CaseSelection::ManifestAddresses)
        )
        || !valid_generator(&suite.generator)
    {
        return Err("suite provenance is incomplete".to_owned());
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
        if profile.application.is_empty()
            || profile.os.is_empty()
            || profile.product_tier.is_empty()
            || profile.locale.is_empty()
            || profile.add_ins.is_empty()
            || !profile_definition_is_expected(profile)
            || profile.artifacts.as_ref().is_some_and(|artifacts| {
                !valid_artifact(&artifacts.workbook)
                    || !valid_artifact(&artifacts.metadata)
                    || !valid_artifact(&artifacts.observations)
            })
        {
            return Err(format!("incomplete host profile {}", profile.profile_id));
        }
    }
    if suite.profiles.len() != 2 {
        return Err("v2 suite must list exactly two workbook profiles".to_owned());
    }
    if suite.state.as_deref() == Some("frozen")
        && suite
            .profiles
            .iter()
            .any(|profile| profile.artifacts.is_none())
    {
        return Err("frozen suite profiles must pin artifact hashes".to_owned());
    }
    Ok(())
}

fn validate_manifest_contract(suite: &OracleSuite, manifest: &CaseManifest) -> Result<(), String> {
    if manifest.schema != CASE_MANIFEST_SCHEMA || manifest.suite_id != suite.suite_id {
        return Err("case manifest identity does not match suite".to_owned());
    }
    let active_case_count = manifest
        .cases
        .iter()
        .filter(|oracle_case| oracle_case.active)
        .count();
    if manifest.feature_set_id != suite.feature_set_id
        || manifest.case_count != Some(CELLRUNE_PRIMARY_CASES)
        || manifest.case_count != Some(manifest.cases.len())
        || manifest.active_case_count != Some(active_case_count)
        || suite.active_case_count != Some(active_case_count)
        || !generators_match(&manifest.generator, &suite.generator)
    {
        return Err("case manifest provenance does not match suite".to_owned());
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
    if metadata.schema != METADATA_SCHEMA && metadata.schema != "cellrune_excel_oracle_metadata_v1"
    {
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
    if suite_path.is_some() {
        raw::verify_no_absolute_path_provenance(&workbook_path)?;
    }
    if let Some(expected_hash) = metadata.sha256.as_deref() {
        let actual_hash = sha256_file(&workbook_path)?;
        if actual_hash != expected_hash {
            return Err(format!(
                "{}: workbook sha256 {} != metadata {}",
                workbook_path.display(),
                actual_hash,
                expected_hash
            ));
        }
    }
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
    validate_profile_artifacts(profile, directory, metadata)?;

    let suite_directory = suite_path.parent().expect("suite path always has a parent");
    if !is_filename(&suite.case_manifest.file) || suite.case_manifest.sha256.len() != 64 {
        return Err(format!(
            "{}: case manifest must be a filename",
            suite_path.display()
        ));
    }
    let manifest_path = suite_directory.join(&suite.case_manifest.file);
    let manifest: CaseManifest = read_json(&manifest_path)?;
    let manifest_hash = sha256_file(&manifest_path)?;
    if manifest_hash != suite.case_manifest.sha256 {
        return Err(format!(
            "{}: case manifest sha256 {} != suite {}",
            manifest_path.display(),
            manifest_hash,
            suite.case_manifest.sha256
        ));
    }
    validate_manifest_contract(&suite, &manifest)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;

    let observations_path = directory.join(OBSERVATIONS_FILE);
    let observations: Observations = read_json(&observations_path)?;
    validate_observation_header(
        &observations,
        &suite,
        profile,
        metadata,
        &manifest_hash,
        &observations_path,
    )?;
    let raw_cells = raw::read_formula_cells(workbook_path)?;
    let raw_cached_cells = raw::read_cached_cells(workbook_path)?;
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
    if metadata.selected_cases != Some(active_cases.len()) {
        return Err(format!(
            "{}: selected_cases does not match active manifest count {}",
            directory.display(),
            active_cases.len()
        ));
    }
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
        validate_observed_result_cells(
            observation,
            &raw_cells,
            &raw_cached_cells,
            &observations_path,
        )?;
        selected.insert(case_key.to_owned(), id);
    }
    if !matches!(
        metadata.case_selection,
        cellrune_integration_tests::oracle::CaseSelection::ManifestAddresses
    ) && !selected_by_address.is_empty()
    {
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
    let workbook_path = directory.join(&metadata.workbook);
    let raw_provenance = raw::read_workbook_provenance(&workbook_path)?;
    let expected_version = format!("AppVersion {}", raw_provenance.app_version);
    let expected_host_build = format!("{expected_version}; rupBuild {}", raw_provenance.rup_build);
    let source_workbook = suite
        .source_workbook
        .as_ref()
        .ok_or_else(|| format!("{}: suite source workbook is absent", directory.display()))?;
    let matches = metadata.sha256.as_deref().is_some_and(is_sha256)
        && metadata.source_workbook_sha256.as_deref() == Some(source_workbook.sha256.as_str())
        && metadata.case_manifest_sha256.as_deref() == Some(suite.case_manifest.sha256.as_str())
        && metadata.formula_cells == source_workbook.formula_cells
        && metadata.selected_cases == suite.active_case_count
        && matches!(
            metadata.case_selection,
            cellrune_integration_tests::oracle::CaseSelection::ManifestAddresses
        )
        && !metadata.source.name.is_empty()
        && metadata.source.name == "CellRune deterministic Excel formula oracle"
        && metadata.source.license == "Apache-2.0"
        && metadata.source.url.as_deref() == Some("https://github.com/emulette/cellrune")
        && metadata.source.revision.as_deref() == Some("0.1.7")
        && generators_match(&metadata.generator, &suite.generator)
        && oracle.suite_id.as_deref() == Some(suite.suite_id.as_str())
        && oracle.feature_set_id == suite.feature_set_id
        && oracle.host_profile_id.as_deref() == Some(profile.profile_id.as_str())
        && oracle.application == profile.application
        && oracle.os.as_deref() == Some(profile.os.as_str())
        && oracle.product_tier.as_deref() == Some(profile.product_tier.as_str())
        && oracle.locale.as_deref() == Some(profile.locale.as_str())
        && !oracle.version.is_empty()
        && oracle.application == raw_provenance.application
        && oracle.version == expected_version
        && oracle.saved_at == raw_provenance.modified_at
        && oracle.channel.is_none()
        && oracle.host_build.as_deref() == Some(expected_host_build.as_str());
    if !matches {
        return Err(format!(
            "{}: metadata host identity does not match suite profile {}",
            directory.display(),
            profile.profile_id
        ));
    }
    Ok(())
}

fn validate_profile_artifacts(
    profile: &HostProfile,
    directory: &Path,
    metadata: &Metadata,
) -> Result<(), String> {
    let Some(artifacts) = profile.artifacts.as_ref() else {
        return Ok(());
    };
    let checks = [
        (&artifacts.workbook, metadata.workbook.as_str()),
        (&artifacts.metadata, METADATA_FILE),
        (&artifacts.observations, OBSERVATIONS_FILE),
    ];
    for (artifact, expected_file) in checks {
        if artifact.file != expected_file {
            return Err(format!(
                "{}: profile artifact filename {} != {}",
                directory.display(),
                artifact.file,
                expected_file
            ));
        }
        let path = directory.join(&artifact.file);
        let actual_hash = sha256_file(&path)?;
        if actual_hash != artifact.sha256 {
            return Err(format!(
                "{}: profile artifact sha256 {} != {}",
                path.display(),
                actual_hash,
                artifact.sha256
            ));
        }
    }
    if artifacts.workbook.formula_cells != Some(metadata.formula_cells)
        || artifacts.workbook.selected_cases != metadata.selected_cases
        || artifacts.workbook.sha256 != expected_profile_workbook_sha256(profile)?
        || artifacts.metadata.formula_cells.is_some()
        || artifacts.metadata.selected_cases.is_some()
        || artifacts.observations.formula_cells.is_some()
        || artifacts.observations.selected_cases.is_some()
    {
        return Err(format!(
            "{}: profile artifact counts do not match metadata",
            directory.display()
        ));
    }
    Ok(())
}

fn validate_observation_header(
    observations: &Observations,
    suite: &OracleSuite,
    profile: &HostProfile,
    metadata: &Metadata,
    manifest_hash: &str,
    observations_path: &Path,
) -> Result<(), String> {
    if observations.schema != OBSERVATIONS_SCHEMA
        || observations.suite_id != suite.suite_id
        || observations.host_profile_id != profile.profile_id
        || observations.saved_at != metadata.oracle.saved_at
        || observations.workbook_sha256 != metadata.sha256
        || observations.source_workbook_sha256 != metadata.source_workbook_sha256
        || observations.case_manifest_sha256.as_deref() != Some(manifest_hash)
        || observations.harness_sha256 != metadata.generator.harness_sha256
        || observations.feature_set_id != suite.feature_set_id
        || observations.case_count != metadata.selected_cases
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
    let expected_status = if raw_cell.value.is_some() {
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
    let expected_result_range = raw_cell.array_result_ref.as_ref().map(|range| {
        let sheet = expected_address
            .rsplit_once('!')
            .map_or("", |(sheet, _)| sheet);
        format!("{sheet}!{range}")
    });
    match (
        expected_result_range.as_deref(),
        observation.result.as_ref(),
    ) {
        (None, None) => {}
        (Some(expected), Some(result)) if result.range == expected => {}
        (None, Some(_)) => {
            return Err(format!(
                "{context}: non-array formula unexpectedly declares an array result"
            ));
        }
        (Some(_), None) => {
            return Err(format!(
                "{context}: raw array formula is missing its observed result"
            ));
        }
        (Some(expected), Some(result)) => {
            return Err(format!(
                "{context}: array result range {} != raw XLSX {expected}",
                result.range
            ));
        }
    }
    Ok(())
}

fn validate_observed_result_cells(
    observation: &ObservedCase,
    raw_formula_cells: &BTreeMap<String, raw::RawFormulaCell>,
    raw_cells: &BTreeMap<String, raw::RawCachedCell>,
    observations_path: &Path,
) -> Result<(), String> {
    let Some(result) = observation.result.as_ref() else {
        return Ok(());
    };
    for observed in &result.cells {
        let context = format!(
            "{}: {} result {}",
            observations_path.display(),
            observation.case_key,
            observed.address
        );
        if observed.cache_status == CacheStatus::ImplicitBlank {
            if raw_formula_cells.contains_key(&observed.address) {
                return Err(format!(
                    "{context}: formula result cell cannot be an implicit blank"
                ));
            }
            if !raw_cell_matches_implicit_blank(raw_cells.get(&observed.address))
                || observed.cache_value.is_some()
                || observed.cache_type != "blank"
                || observed.rich_error.present
                || observed.rich_error.raw_error.is_some()
                || observed.rich_error.vm_index.is_some()
                || observed.rich_error.record_index.is_some()
                || observed.rich_error.structure_index.is_some()
                || observed.rich_error.error_type_code.is_some()
                || observed.rich_error.resolved_error.is_some()
                || observed.rich_error.fallback_error.is_some()
            {
                return Err(format!(
                    "{context}: implicit blank does not exact-match raw XLSX absence"
                ));
            }
            continue;
        }
        let raw = raw_cells
            .get(&observed.address)
            .ok_or_else(|| format!("{context}: saved result cell is absent from raw XLSX"))?;
        let expected_status = if raw.value.is_some() {
            CacheStatus::Semantic
        } else {
            CacheStatus::MissingSemanticCache
        };
        let raw_rich = raw.rich_error.as_ref();
        let expected_error = (raw.value_type == "e").then(|| raw.value.clone()).flatten();
        if observed.cache_status != expected_status
            || observed.cache_status == CacheStatus::Circular
            || observed.cache_status == CacheStatus::ImplicitBlank
            || observed.cache_value != raw.value
            || observed.cache_type != raw.value_type
            || observed.rich_error.present != raw.rich_error.is_some()
            || observed.rich_error.raw_error != expected_error
            || observed.rich_error.vm_index != raw.vm_index
            || observed.rich_error.record_index != raw_rich.map(|record| record.record_index)
            || observed.rich_error.structure_index != raw_rich.map(|record| record.structure_index)
            || observed.rich_error.error_type_code
                != raw_rich.and_then(|record| record.error_type_code)
            || observed.rich_error.resolved_error
                != raw_rich.and_then(|record| record.resolved_error.clone())
            || observed.rich_error.fallback_error
                != raw_rich.and_then(|record| record.fallback_error.clone())
        {
            return Err(format!(
                "{context}: observation does not exact-match raw XLSX cache metadata"
            ));
        }
    }
    Ok(())
}

fn raw_cell_matches_implicit_blank(raw: Option<&raw::RawCachedCell>) -> bool {
    raw.is_none_or(|cell| {
        cell.value.is_none() && cell.rich_error.is_none() && cell.vm_index.is_none()
    })
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

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn is_filename(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn generator_is_expected(generator: &GeneratorMetadata) -> bool {
    generator.name.as_deref() == Some(CELLRUNE_GENERATOR_NAME)
        && generator.revision.as_deref() == Some(CELLRUNE_GENERATOR_REVISION)
        && generator.harness_sha256.as_deref() == Some(CELLRUNE_HARNESS_SHA256)
}

fn valid_generator(generator: &Option<GeneratorMetadata>) -> bool {
    generator.as_ref().is_some_and(generator_is_expected)
}

fn generators_match(left: &GeneratorMetadata, right: &Option<GeneratorMetadata>) -> bool {
    right.as_ref().is_some_and(|right| {
        left.name == right.name
            && left.revision == right.revision
            && left.harness_sha256 == right.harness_sha256
            && generator_is_expected(left)
    })
}

fn valid_artifact(artifact: &ArtifactReference) -> bool {
    is_filename(&artifact.file) && is_sha256(&artifact.sha256)
}

fn expected_profile_workbook_sha256(profile: &HostProfile) -> Result<&'static str, String> {
    match profile.directory.as_str() {
        "online" => Ok(CELLRUNE_ONLINE_SHA256),
        "desktop-2021" => Ok(CELLRUNE_DESKTOP_SHA256),
        _ => Err(format!(
            "unexpected host profile directory {}",
            profile.directory
        )),
    }
}

fn profile_definition_is_expected(profile: &HostProfile) -> bool {
    let add_ins_are_expected =
        profile.add_ins.len() == 1 && profile.add_ins.get("euro_currency_tools") == Some(&false);
    match profile.directory.as_str() {
        "online" => {
            profile.profile_id == "excel-online-free-en-ui-ko-kr"
                && profile.application == "Microsoft Excel Online"
                && profile.os == "web"
                && profile.product_tier == "free"
                && profile.locale == "en-US UI; ko-KR regional format"
                && add_ins_are_expected
        }
        "desktop-2021" => {
            profile.profile_id == "excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools"
                && profile.application == "Microsoft Macintosh Excel"
                && profile.os == "macOS"
                && profile.product_tier == "Office Home & Student 2021"
                && profile.locale == "en-US UI; ko-KR regional format"
                && add_ins_are_expected
        }
        _ => false,
    }
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
        if let Some(observation) = loaded
            .observations
            .as_ref()
            .and_then(|observations| observations.get(key))
        {
            audit_observed_result(&context, loaded, id, observation, expectation, problems);
        }
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

fn audit_observed_result(
    context: &str,
    loaded: &LoadedOracle,
    anchor_id: CalculationCellId,
    observation: &ObservedCase,
    expectation: &Expectation,
    problems: &mut Vec<String>,
) {
    let Some(result) = observation.result.as_ref() else {
        return;
    };
    let Some((sheet_name, range_text)) = result.range.rsplit_once('!') else {
        problems.push(format!("{context}: array result has an invalid range"));
        return;
    };
    let (start_text, end_text) = range_text
        .split_once(':')
        .map_or((range_text, range_text), |(start, end)| (start, end));
    let Ok(start) = CellAddress::from_a1(start_text) else {
        problems.push(format!(
            "{context}: array result has an invalid start address"
        ));
        return;
    };
    let Ok(end) = CellAddress::from_a1(end_text) else {
        problems.push(format!(
            "{context}: array result has an invalid end address"
        ));
        return;
    };
    let Ok(range) = CellRange::new(start, end) else {
        problems.push(format!("{context}: array result range is not ordered"));
        return;
    };
    let Some(sheet) = loaded.workbook.sheet_by_name(sheet_name) else {
        problems.push(format!("{context}: array result names an unknown sheet"));
        return;
    };
    let expected_cells = u64::from(range.height()) * u64::from(range.width());
    if result.rows != range.height()
        || result.columns != range.width()
        || u64::try_from(result.cells.len()).ok() != Some(expected_cells)
    {
        problems.push(format!(
            "{context}: array result shape does not match its range"
        ));
        return;
    }
    let anchor = CalculationCellId::new(sheet.id(), start);
    if anchor != anchor_id {
        problems.push(format!(
            "{context}: array result range does not start at its formula anchor"
        ));
    }
    let mut addresses = BTreeSet::new();
    let mut mismatches = 0_usize;
    for cell in &result.cells {
        if !addresses.insert(cell.address.as_str()) {
            problems.push(format!("{context}: array result contains a duplicate cell"));
            continue;
        }
        let Some((cell_sheet, cell_address)) = cell.address.rsplit_once('!') else {
            problems.push(format!(
                "{context}: array result cell has an invalid address"
            ));
            continue;
        };
        if cell_sheet != sheet_name {
            problems.push(format!(
                "{context}: array result cell escapes its result sheet"
            ));
            continue;
        }
        let Ok(address) = CellAddress::from_a1(cell_address) else {
            problems.push(format!(
                "{context}: array result cell has an invalid address"
            ));
            continue;
        };
        if !range.contains(address) {
            problems.push(format!(
                "{context}: array result cell escapes its declared range"
            ));
            continue;
        }
        let id = CalculationCellId::new(sheet.id(), address);
        let actual = loaded
            .calculation
            .materialized_cell(id)
            .map(cellrune::MaterializedCalculationCell::result)
            .or_else(|| loaded.calculation.cell(id));
        let expected = observed_result_cell_value(cell);
        let actual = actual.and_then(|value| observed_result(Some(value)).ok().flatten());
        let matches = match (actual.as_ref(), expected.as_ref()) {
            (Some(actual), Some(expected)) => values_match(actual, expected, None).unwrap_or(false),
            (None, None) => true,
            _ => false,
        };
        if !matches {
            mismatches += 1;
        }
    }
    match expectation.classification {
        Classification::Match if mismatches > 0 => problems.push(format!(
            "{context}: {mismatches} array result cells differ from Excel"
        )),
        Classification::Divergent if mismatches == 0 => problems.push(format!(
            "{context}: divergent array result now matches Excel in every cell"
        )),
        _ => {}
    }
}

fn observed_result_cell_value(
    cell: &cellrune_integration_tests::oracle::ObservedResultCell,
) -> Option<ObservedValue> {
    if cell.cache_status != CacheStatus::Semantic {
        return None;
    }
    let value = cell
        .rich_error
        .resolved_error
        .as_ref()
        .or(cell.rich_error.fallback_error.as_ref())
        .or(cell.cache_value.as_ref())?
        .clone();
    let value_type = if cell.rich_error.present {
        "e"
    } else {
        &cell.cache_type
    };
    Some(ObservedValue {
        value,
        value_type: value_type.to_owned(),
    })
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
        CacheStatus::MissingSemanticCache | CacheStatus::ImplicitBlank => true,
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

    use cellrune_integration_tests::oracle::{OracleMetadata, OracleSuite};

    use super::{
        MESSAGE_ITERATIVE_CALCULATION, MESSAGE_SUITE_REQUIRED, audit_all, oracle_root, raw,
        raw_cell_matches_implicit_blank, read_json, resolve_suite_path, validate_suite_contract,
        verify_iterative_calculation,
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
    fn v2_suite_rejects_release_and_harness_provenance_substitution() {
        let suite_path = oracle_root().join("cellrune/suite.json");
        let mut suite: OracleSuite = read_json(&suite_path).expect("committed v2 suite");
        assert!(validate_suite_contract(&suite).is_ok());

        suite
            .planned_release_range
            .as_mut()
            .expect("v2 planned release range")
            .last = "0.1.20".to_owned();
        assert!(validate_suite_contract(&suite).is_err());

        let mut suite: OracleSuite = read_json(&suite_path).expect("committed v2 suite");
        suite
            .generator
            .as_mut()
            .expect("v2 generator")
            .harness_sha256 = Some("0".repeat(64));
        assert!(validate_suite_contract(&suite).is_err());
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
            feature_set_id: None,
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

    #[test]
    fn typed_value_absence_is_a_valid_implicit_blank() {
        let typed_blank = raw::RawCachedCell {
            value: None,
            value_type: "str".to_owned(),
            vm_index: None,
            rich_error: None,
        };
        assert!(raw_cell_matches_implicit_blank(Some(&typed_blank)));

        let typed_value = raw::RawCachedCell {
            value: Some(String::new()),
            value_type: "str".to_owned(),
            vm_index: None,
            rich_error: None,
        };
        assert!(!raw_cell_matches_implicit_blank(Some(&typed_value)));
    }
}
