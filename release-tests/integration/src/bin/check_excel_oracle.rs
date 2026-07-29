//! Explicit local audit for committed Excel-saved workbook oracles.

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
    Expectations, HostProfile, METADATA_SCHEMA, ManifestCase, Metadata, OBSERVATIONS_SCHEMA,
    Observations, ObservedCase, ObservedValue, OracleExclusion, OracleSuite, SUITE_SCHEMA,
    values_match,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

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

const USAGE: &str = "usage: check_excel_oracle [--require-cellrune-suite | --report <oracle-directory> [output.json]]";
const METADATA_FILE: &str = "metadata.json";
const EXPECTATIONS_FILE: &str = "expectations.json";
const SUITE_FILE: &str = "suite.json";
const OBSERVATIONS_FILE: &str = "observations.json";
const CELLRUNE_SUITE_ID: &str = "cellrune-excel-host-matrix-v1";
const REQUIRED_CELLRUNE_PROFILES: [(&str, &str); 2] = [
    ("excel-online-free-en-ui-ko-kr", "online"),
    (
        "excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools",
        "desktop-2021",
    ),
];
const EXPECTED_FUNCTION_EXCLUSIONS: [&str; 28] = [
    "CALL",
    "CELL",
    "COPILOT",
    "CUBEKPIMEMBER",
    "CUBEMEMBER",
    "CUBEMEMBERPROPERTY",
    "CUBERANKEDMEMBER",
    "CUBESET",
    "CUBESETCOUNT",
    "CUBEVALUE",
    "DETECTLANGUAGE",
    "ENCODEURL",
    "EUROCONVERT",
    "FILTERXML",
    "GETPIVOTDATA",
    "IMAGE",
    "INFO",
    "JIS",
    "NOW",
    "RAND",
    "RANDARRAY",
    "RANDBETWEEN",
    "REGISTER.ID",
    "RTD",
    "STOCKHISTORY",
    "TODAY",
    "TRANSLATE",
    "WEBSERVICE",
];
const EXPECTED_HOST_MATRIX_EXCLUSIONS: [&str; 4] = ["ENCODEURL", "EUROCONVERT", "FILTERXML", "JIS"];
const MESSAGE_NO_ORACLES: &str = "no oracle metadata files found";
const MESSAGE_EXPECTATION_KEYS: &str =
    "expectation keys must exactly equal the selected workbook cases";
const MESSAGE_UNCLASSIFIED: &str = "unclassified oracle case";
const MESSAGE_NOTE_REQUIRED: &str = "reviewed non-match classification requires a note";
const MESSAGE_WORKBOOK_FILENAME: &str = "workbook must be a filename within the oracle directory";
const MESSAGE_SHA_FORMAT: &str = "SHA-256 must contain exactly 64 hexadecimal digits";
const MESSAGE_ITERATIVE_CALCULATION: &str =
    "workbook iterative-calculation setting does not match metadata";
const MESSAGE_CELLRUNE_SUITE_REQUIRED: &str =
    "CellRune 0.1.7 requires conformance/cellrune/suite.json with both host profiles";

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [] => audit_all(&oracle_root(), false),
        [flag] if flag == "--require-cellrune-suite" => audit_all(&oracle_root(), true),
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

fn audit_all(root: &Path, require_cellrune_suite: bool) -> Result<(), Vec<String>> {
    let mut suite_files = Vec::new();
    collect_named_files(root, SUITE_FILE, &mut suite_files)?;
    suite_files.sort();
    let mut managed_metadata = BTreeSet::new();
    let mut problems = Vec::new();
    let required_cellrune_suite = root.join("cellrune").join(SUITE_FILE);
    if require_cellrune_suite && !suite_files.contains(&required_cellrune_suite) {
        problems.push(format!(
            "{}: {MESSAGE_CELLRUNE_SUITE_REQUIRED}",
            required_cellrune_suite.display()
        ));
    }
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

    let mut expectations_by_profile = BTreeMap::new();
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
                expectations_by_profile.insert(profile.profile_id.clone(), loaded.expectations);
            }
            Err(error) => problems.push(error),
        }
    }
    for oracle_case in manifest.cases.iter().filter(|case| case.active) {
        let classifications = suite
            .profiles
            .iter()
            .filter_map(|profile| {
                expectations_by_profile
                    .get(&profile.profile_id)?
                    .get(&oracle_case.case_key)
                    .map(|expectation| expectation.classification)
            })
            .collect::<Vec<_>>();
        if classifications.len() == suite.profiles.len()
            && !classifications.iter().any(|classification| {
                matches!(
                    classification,
                    Classification::Match
                        | Classification::Divergent
                        | Classification::NotImplemented
                )
            })
        {
            problems.push(format!(
                "{}: active case {} has no semantic classification in any required profile",
                suite_path.display(),
                oracle_case.case_key
            ));
        }
    }
}

fn validate_suite_contract(suite: &OracleSuite) -> Result<(), String> {
    if suite.schema != SUITE_SCHEMA || suite.suite_id != CELLRUNE_SUITE_ID {
        return Err(format!("unsupported suite schema {}", suite.schema));
    }
    if suite.case_manifest != "case-manifest.json" {
        return Err("CellRune suite must use case-manifest.json".to_owned());
    }
    verify_sha256_format(&suite.source_workbook_sha256)?;
    let mut profile_ids = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for profile in &suite.profiles {
        if !profile_ids.insert(profile.profile_id.as_str()) {
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
            || !profile.add_ins.contains_key("euro_currency_tools")
        {
            return Err(format!("incomplete host profile {}", profile.profile_id));
        }
    }
    let required = REQUIRED_CELLRUNE_PROFILES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let actual = suite
        .profiles
        .iter()
        .map(|profile| (profile.profile_id.as_str(), profile.directory.as_str()))
        .collect::<BTreeSet<_>>();
    if actual != required {
        return Err("CellRune suite does not contain the exact required host matrix".to_owned());
    }
    for profile in &suite.profiles {
        let euro_tools = profile.add_ins.get("euro_currency_tools") == Some(&false)
            && profile.add_ins.len() == 1;
        let exact = match profile.profile_id.as_str() {
            "excel-online-free-en-ui-ko-kr" => {
                profile.directory == "online"
                    && profile.application == "Microsoft Excel Online"
                    && profile.os == "web"
                    && profile.product_tier == "free"
                    && profile.locale == "en-US UI; ko-KR regional format"
                    && euro_tools
            }
            "excel-mac-2021-home-student-en-ui-ko-kr-no-euro-tools" => {
                profile.directory == "desktop-2021"
                    && profile.application == "Microsoft Macintosh Excel"
                    && profile.os == "macOS"
                    && profile.product_tier == "Office Home & Student 2021"
                    && profile.locale == "en-US UI; ko-KR regional format"
                    && euro_tools
            }
            _ => false,
        };
        if !exact {
            return Err(format!(
                "host profile does not exact-match the reviewed CellRune matrix: {}",
                profile.profile_id
            ));
        }
    }
    Ok(())
}

fn validate_manifest_contract(suite: &OracleSuite, manifest: &CaseManifest) -> Result<(), String> {
    if manifest.schema != CASE_MANIFEST_SCHEMA || manifest.suite_id != suite.suite_id {
        return Err("case manifest identity does not match suite".to_owned());
    }
    if manifest.generator.name.as_deref().is_none_or(str::is_empty)
        || manifest
            .generator
            .revision
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err("case manifest generator provenance is incomplete".to_owned());
    }
    let profile_ids = suite
        .profiles
        .iter()
        .map(|profile| profile.profile_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut keys = BTreeSet::new();
    let mut catalog_addresses = BTreeSet::new();
    let mut formula_addresses = BTreeSet::new();
    let mut excluded_functions = BTreeSet::new();
    let mut host_matrix_exclusions = BTreeSet::new();
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
        verify_text_sha256(
            &oracle_case.authored_formula,
            &oracle_case.authored_formula_fingerprint,
        )?;
        verify_text_sha256(
            &oracle_case.storage_formula,
            &oracle_case.storage_formula_fingerprint,
        )?;
        let semantic_fingerprint = manifest_semantic_fingerprint(oracle_case)?;
        if semantic_fingerprint != oracle_case.semantic_fingerprint {
            return Err(format!(
                "semantic fingerprint does not match {}",
                oracle_case.case_key
            ));
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
        if let Some(exclusion) = &oracle_case.exclusion {
            let (reason, evidence) = exclusion.rationale();
            if reason.is_empty() || evidence.is_empty() {
                return Err(format!(
                    "exclusion rationale is incomplete for {}",
                    oracle_case.case_key
                ));
            }
            if oracle_case.case_key.starts_with("function:") {
                excluded_functions.insert(oracle_case.function.as_str());
            }
        }
        if let Some(OracleExclusion::HostMatrixUnavailable {
            unsupported_profile_ids,
            ..
        }) = &oracle_case.exclusion
        {
            let unsupported = unsupported_profile_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if unsupported.len() != unsupported_profile_ids.len()
                || unsupported != profile_ids
                || oracle_case.active
            {
                return Err(format!(
                    "host-matrix exclusion does not cover the suite for {}",
                    oracle_case.case_key
                ));
            }
            host_matrix_exclusions.insert(oracle_case.function.as_str());
        }
        if oracle_case.exclusion.is_some() && oracle_case.active {
            return Err(format!(
                "excluded case is unexpectedly active: {}",
                oracle_case.case_key
            ));
        }
    }
    let expected_functions = EXPECTED_FUNCTION_EXCLUSIONS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if excluded_functions != expected_functions {
        return Err(format!(
            "function exclusion inventory changed: expected={expected_functions:?} actual={excluded_functions:?}"
        ));
    }
    let expected_host_matrix = EXPECTED_HOST_MATRIX_EXCLUSIONS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if host_matrix_exclusions != expected_host_matrix {
        return Err(format!(
            "host-matrix exclusion inventory changed: expected={expected_host_matrix:?} actual={host_matrix_exclusions:?}"
        ));
    }
    Ok(())
}

fn manifest_semantic_fingerprint(oracle_case: &ManifestCase) -> Result<String, String> {
    #[derive(Serialize)]
    struct SemanticRecord<'a> {
        function: &'a str,
        category: &'a str,
        scenario: &'a str,
        authored_formula: &'a str,
        storage_formula: &'a str,
        allowed_host_rewrites: &'a [cellrune_integration_tests::oracle::HostFormulaRewrite],
        active: bool,
        seed_classification: &'a str,
        inactive_reason: &'a Option<String>,
        exclusion: &'a Option<OracleExclusion>,
    }
    let serialized = serde_json::to_string(&SemanticRecord {
        function: &oracle_case.function,
        category: &oracle_case.category,
        scenario: &oracle_case.scenario,
        authored_formula: &oracle_case.authored_formula,
        storage_formula: &oracle_case.storage_formula,
        allowed_host_rewrites: &oracle_case.allowed_host_rewrites,
        active: oracle_case.active,
        seed_classification: &oracle_case.seed_classification,
        inactive_reason: &oracle_case.inactive_reason,
        exclusion: &oracle_case.exclusion,
    })
    .map_err(|error| format!("cannot serialize manifest semantics: {error}"))?;
    let digest = Sha256::digest(serialized.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
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
    verify_sha256(&workbook_path, &metadata.sha256)?;
    if let Some(source_workbook_sha256) = &metadata.source_workbook_sha256 {
        verify_sha256_format(source_workbook_sha256)
            .map_err(|error| format!("{}: {error}", metadata_path.display()))?;
    }
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
    let suite_path = directory
        .parent()
        .map(|parent| parent.join(SUITE_FILE))
        .filter(|path| path.is_file());
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
    validate_observation_header(
        &observations,
        metadata,
        &suite,
        profile,
        workbook_path,
        &manifest_path,
        &observations_path,
    )?;
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
        && oracle.host_profile_id.as_deref() == Some(profile.profile_id.as_str())
        && oracle.application == profile.application
        && oracle.os.as_deref() == Some(profile.os.as_str())
        && oracle.locale.as_deref() == Some(profile.locale.as_str())
        && oracle.product_tier.as_deref() == Some(profile.product_tier.as_str())
        && metadata.source_workbook_sha256.as_deref()
            == Some(suite.source_workbook_sha256.as_str());
    if !matches {
        return Err(format!(
            "{}: metadata host identity does not match suite profile {}",
            directory.display(),
            profile.profile_id
        ));
    }
    if oracle.version.is_empty()
        || oracle.saved_at.is_empty()
        || oracle.host_build.as_deref().is_none_or(str::is_empty)
        || oracle
            .product_tier_evidence
            .as_deref()
            .is_none_or(str::is_empty)
        || oracle.host_note.as_deref().is_none_or(str::is_empty)
    {
        return Err(format!(
            "{}: suite metadata requires host version/build, saved_at, product-tier evidence, and host_note",
            directory.display()
        ));
    }
    Ok(())
}

fn validate_observation_header(
    observations: &Observations,
    metadata: &Metadata,
    suite: &OracleSuite,
    profile: &HostProfile,
    workbook_path: &Path,
    manifest_path: &Path,
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
    if observations.workbook_sha256 != metadata.sha256 {
        return Err(format!(
            "{}: observation workbook SHA-256 does not match metadata",
            observations_path.display()
        ));
    }
    if observations.source_workbook_sha256 != suite.source_workbook_sha256
        || metadata.source_workbook_sha256.as_deref()
            != Some(observations.source_workbook_sha256.as_str())
    {
        return Err(format!(
            "{}: common source workbook SHA-256 does not match suite/metadata",
            observations_path.display()
        ));
    }
    verify_sha256(workbook_path, &observations.workbook_sha256)?;
    verify_sha256(manifest_path, &observations.case_manifest_sha256)?;
    if observations.saved_at != metadata.oracle.saved_at {
        return Err(format!(
            "{}: observation saved_at does not match metadata",
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
    if observation.address != expected_address
        || observation.authored_formula_fingerprint != oracle_case.authored_formula_fingerprint
    {
        return Err(format!(
            "{context}: observation does not join the active manifest record"
        ));
    }
    verify_text_sha256(
        &observation.saved_formula,
        &observation.saved_formula_fingerprint,
    )
    .map_err(|error| format!("{context}: {error}"))?;
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

fn verify_text_sha256(value: &str, expected: &str) -> Result<(), String> {
    let digest = expected
        .strip_prefix("sha256:")
        .ok_or_else(|| "text fingerprint must use the sha256: prefix".to_owned())?;
    verify_sha256_bytes(value.as_bytes(), digest, "text")
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    verify_sha256_bytes(&bytes, expected, &path.display().to_string())
}

fn verify_sha256_format(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(MESSAGE_SHA_FORMAT.to_owned())
    }
}

fn verify_sha256_bytes(bytes: &[u8], expected: &str, context: &str) -> Result<(), String> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{context}: {MESSAGE_SHA_FORMAT}"));
    }
    let actual = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "{context}: SHA-256 {actual} != expected {expected}"
        ))
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
                require_note(&context, expectation, problems);
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
                require_note(&context, expectation, problems);
                if matches!(result, Some(CalculationCellResult::Unavailable(_))) {
                    counts.not_implemented += 1;
                } else {
                    problems.push(format!("{context}: not-implemented case now calculates"));
                }
            }
            Classification::HostUnsupported => {
                require_note(&context, expectation, problems);
                counts.host_unsupported += 1;
            }
            Classification::Excluded | Classification::Unreadable => {
                require_note(&context, expectation, problems);
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

fn require_note(context: &str, expectation: &Expectation, problems: &mut Vec<String>) {
    if expectation.note.as_deref().is_none_or(str::is_empty) {
        problems.push(format!("{context}: {MESSAGE_NOTE_REQUIRED}"));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{MESSAGE_ITERATIVE_CALCULATION, audit_all, verify_iterative_calculation};

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
    fn release_gate_requires_the_cellrune_host_matrix_suite() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cellrune-oracle-release-gate-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("temporary conformance root");

        let errors = audit_all(&root, true).expect_err("missing suite must fail release gate");

        assert!(
            errors
                .iter()
                .any(|error| error.contains("CellRune 0.1.7 requires"))
        );
        assert!(
            audit_all(&root, false)
                .expect_err("empty ordinary audit still has no metadata")
                .iter()
                .all(|error| !error.contains("CellRune 0.1.7 requires"))
        );
        fs::remove_dir(&root).expect("remove temporary conformance root");
    }
}
