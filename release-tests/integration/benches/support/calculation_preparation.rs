use std::hint::black_box;
use std::time::{Duration, Instant};

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellValue, DateSystem, DefinedName, DefinedNameScope, FormulaCell, FormulaDialect,
    FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId,
    SheetName, SheetVisibility, WorkbookSnapshot, WorkbookSource, calculate_workbook,
};

pub(super) fn run(rows: u32, iterations: u32, arguments: &[String]) {
    let named = arguments.iter().any(|value| value == "--names");
    let rank = arguments.iter().any(|value| value == "--rank");
    assert!(!(named && rank), "choose one calculation scenario");
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("data").expect("sheet name"),
        SheetVisibility::Visible,
    );
    let mut values = (0..rows).collect::<Vec<_>>();
    let mut seed = 42_u64;
    for index in (1..values.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        values.swap(index, (seed % (index as u64 + 1)) as usize);
    }
    for row in 1..=rows {
        let value = if rank { values[(row - 1) as usize] } else { 1 };
        sheet
            .insert_cell(
                address(row, 1),
                CellContent::Literal(CellValue::number(value as f64).expect("finite input")),
            )
            .expect("literal cell");
    }
    let formulas = if rank { 100.min(rows) } else { rows };
    for row in 1..=formulas {
        let formula = if rank {
            format!("RANK({},A1:A{rows})", row - 1)
        } else if named {
            format!("_xlfn.SUM(data!A{row},base_rate)+ABS(-1)")
        } else {
            format!("SUM(A{row},2)+ABS(-1)")
        };
        sheet
            .insert_cell(
                address(row, 2),
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    FormulaText::from_xlsx(formula).expect("formula"),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .expect("formula cell");
    }
    let names = if named {
        vec![
            DefinedName::new(
                "base_rate",
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("2").expect("name formula"),
                false,
            )
            .expect("name"),
        ]
    } else {
        Vec::new()
    };
    let workbook = WorkbookSnapshot::new_with_metadata(
        vec![sheet],
        names,
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("benchmark", "1").expect("provider"),
            None,
        ),
    )
    .expect("benchmark workbook");
    let mut elapsed = Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        let result = calculate_workbook(black_box(&workbook), CalculationOptions::default());
        elapsed += started.elapsed();
        assert_eq!(result.cells().count(), formulas as usize);
        for row in 1..=formulas {
            let expected = if rank { f64::from(rows - row + 1) } else { 4.0 };
            assert_eq!(
                result.cell(CalculationCellId::new(sheet_id, address(row, 2))),
                Some(&CalculationCellResult::Value(
                    CellValue::number(expected).expect("result")
                ))
            );
        }
    }
    println!("cellrune_calculation_preparation_benchmark_v1");
    println!(
        "scenario\t{}",
        if rank {
            "rank"
        } else if named {
            "names"
        } else {
            "arithmetic"
        }
    );
    println!("rows\t{rows}");
    println!("formulas\t{formulas}");
    println!("iterations\t{iterations}");
    println!(
        "calculate_mean_ns\t{}",
        elapsed.as_nanos() / u128::from(iterations)
    );
}

fn address(row: u32, column: u32) -> CellAddress {
    CellAddress::from_indices(row, column).expect("benchmark address")
}
