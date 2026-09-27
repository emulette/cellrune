//! Manual static-demand benchmark. Run one scenario per process with the bench profile.
//! Pass --hold to keep the process alive after its JSON sample for external peak-RSS collection.

use std::hint::black_box;
use std::io::{self, Write};
use std::time::Instant;

use cellrune::{
    CellAddress, DefinedName, DefinedNameScope, EditBatch, FormulaText, SheetName, WorkbookChange,
    WorkbookDraft, scan_function_usage,
};
use serde::Serialize;

#[derive(Serialize)]
struct Sample {
    scenario: String,
    first_ns: u128,
    cache_hit_ns: u128,
    edited_ns: u128,
    formula_count: usize,
    sum_calls: u64,
}

fn name(name: &str, formula: &str, scope: DefinedNameScope) -> WorkbookChange {
    WorkbookChange::set_defined_name(
        DefinedName::new(name, scope, FormulaText::from_xlsx(formula).unwrap(), false).unwrap(),
    )
}

fn workload(scenario: &str) -> (WorkbookDraft, String, u64) {
    let (depth, rows, plain, scoped, chain, cycle) = match scenario {
        "dag8" => (8, 1, false, false, false, false),
        "dag12" => (12, 1, false, false, false, false),
        "dag16" => (16, 1, false, false, false, false),
        "dag18" => (18, 1, false, false, false, false),
        "shared" => (12, 100, false, false, false, false),
        "plain1000" => (0, 1_000, true, false, false, false),
        "plain10000" => (0, 10_000, true, false, false, false),
        "scoped" => (8, 20, false, true, false, false),
        "chain" => (256, 1, false, false, true, false),
        "cycle" => (0, 1, false, false, false, true),
        _ => panic!("choose dag8/dag12/dag16/dag18/shared/plain1000/plain10000/scoped/chain/cycle"),
    };
    let mut draft = WorkbookDraft::new();
    let mut sheets = vec![draft.workbook().sheets()[0].id()];
    if scoped {
        sheets.push(draft.add_sheet(SheetName::new("Second").unwrap()).unwrap());
    }
    let mut changes = Vec::new();
    if !plain && !cycle {
        let seed = if scoped {
            "SUM(1)+LocalSeed"
        } else {
            "SUM(1)+MIN(1)+MAX(1)+ABS(-1)+AVERAGE(1)+COUNT(1)"
        };
        changes.push(name("Usage_0", seed, DefinedNameScope::Workbook));
        for level in 1..=depth {
            let formula = if chain {
                format!("Usage_{}", level - 1)
            } else {
                format!("Usage_{}+Usage_{}", level - 1, level - 1)
            };
            changes.push(name(
                &format!("Usage_{level}"),
                &formula,
                DefinedNameScope::Workbook,
            ));
        }
    }
    if cycle {
        changes.push(name(
            "Cycle_A",
            "SUM(1)+Cycle_B",
            DefinedNameScope::Workbook,
        ));
        changes.push(name(
            "Cycle_B",
            "MIN(1)+Cycle_A",
            DefinedNameScope::Workbook,
        ));
    }
    let formula = if plain {
        "SUM(1)+ABS(-1)".to_owned()
    } else if cycle {
        "Cycle_A+Cycle_B".to_owned()
    } else {
        format!("Usage_{depth}")
    };
    for (index, &sheet) in sheets.iter().enumerate() {
        if scoped {
            changes.push(name(
                "LocalSeed",
                if index == 0 { "MIN(1)" } else { "MAX(1)" },
                DefinedNameScope::Sheet(sheet),
            ));
        }
        for row in 1..=rows {
            changes.push(WorkbookChange::set_cell_formula(
                sheet,
                CellAddress::from_indices(row, 1).unwrap(),
                FormulaText::from_xlsx(&formula).unwrap(),
            ));
        }
    }
    draft.apply_changes(EditBatch::new(changes)).unwrap();
    let per_cell = if cycle {
        2
    } else if plain || chain {
        1
    } else {
        1_u64 << depth
    };
    let expected = per_cell * u64::from(rows) * sheets.len() as u64;
    (draft, formula, expected)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scenario = args.first().map(String::as_str).unwrap_or("dag18");
    let (mut draft, formula, expected) = workload(scenario);
    // Construction and edits stay outside the timers; initial parsing belongs to the scan.
    let started = Instant::now();
    let first = black_box(scan_function_usage(draft.workbook()));
    let first_ns = started.elapsed().as_nanos();
    let sum_calls = first
        .entries()
        .iter()
        .find(|entry| entry.name() == "SUM")
        .unwrap()
        .call_count();
    assert_eq!(sum_calls, expected);
    let started = Instant::now();
    for _ in 0..1_000 {
        black_box(scan_function_usage(draft.workbook()));
    }
    let cache_hit_ns = started.elapsed().as_nanos() / 1_000;
    assert_eq!(scan_function_usage(draft.workbook()), first);
    let sheet = draft.workbook().sheets()[0].id();
    draft
        .set_cell_formula(
            sheet,
            CellAddress::from_a1("A1").unwrap(),
            FormulaText::from_xlsx(format!("{formula}+0")).unwrap(),
        )
        .unwrap();
    let started = Instant::now();
    let edited = black_box(scan_function_usage(draft.workbook()));
    let edited_ns = started.elapsed().as_nanos();
    assert_eq!(edited, first);
    println!(
        "{}",
        serde_json::to_string(&Sample {
            scenario: scenario.to_owned(),
            first_ns,
            cache_hit_ns,
            edited_ns,
            formula_count: first.formula_count(),
            sum_calls,
        })
        .unwrap()
    );
    if args.iter().any(|arg| arg == "--hold") {
        io::stdout().flush().unwrap();
        let mut release = String::new();
        io::stdin().read_line(&mut release).unwrap();
    }
}
