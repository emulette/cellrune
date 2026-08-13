/// `YIELD` — direct closed form for the single-coupon branch, safeguarded inverse otherwise.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::regular_bond::{BondTerms, bond_measurements, direct_yield_n1};
use super::schedule::estimated_periods;
use super::solver::{EXCEL_YIELD_POLICY, EXTENDED_YIELD_POLICY, solve};
use super::{
    cash_flow_reduction_with_poll, charge_work, check_cancellation, coerce_basis, coerce_date,
    coerce_frequency, date_from_serial_arg, finite_number,
};
use crate::FinancialSolverSemantics;

pub(super) fn call(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 6 || args.len() > 7 {
        return Value::Error(ErrorKind::Value);
    }
    let settlement = match coerce_date(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let maturity = match coerce_date(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[2]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let price = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[5]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(6)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if settlement >= maturity {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || price <= 0.0 || redemption <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let settlement_date = match date_from_serial_arg(settlement, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let maturity_date = match date_from_serial_arg(maturity, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(work) = estimated_periods(settlement_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work.saturating_mul(2)) {
        return Value::Error(kind);
    }
    let mut poll = || check_cancellation(context);
    let measurements = match bond_measurements(
        BondTerms {
            settlement: settlement_date,
            maturity: maturity_date,
            rate,
            redemption,
            frequency,
            basis,
            date_system: engine.date_system(),
        },
        &mut poll,
    ) {
        Ok(Some(measurements)) => measurements,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };

    if measurements.coupon_count == 1 {
        return finite_number(direct_yield_n1(&measurements, price));
    }

    let frequency_value = measurements.frequency;
    let lower_bound = -frequency_value;
    let policy = match engine.financial_solver_semantics() {
        FinancialSolverSemantics::ExcelIterationBudget => EXCEL_YIELD_POLICY,
        FinancialSolverSemantics::ExtendedSearch => EXTENDED_YIELD_POLICY,
    };
    let residual = |yield_: f64| -> Result<(f64, f64), ErrorKind> {
        let (value, time_weighted) = cash_flow_reduction_with_poll(
            &measurements.flows,
            frequency_value,
            yield_,
            &mut || check_cancellation(context),
        )?;
        let q = 1.0 + yield_ / frequency_value;
        let clean = value - measurements.accrued_interest;
        let derivative = -time_weighted / (frequency_value * q);
        Ok((clean - price, derivative))
    };
    match solve(
        engine,
        context,
        lower_bound,
        measurements.flows.len(),
        policy,
        &residual,
    ) {
        Ok(yield_) => finite_number(yield_),
        Err(kind) => Value::Error(kind),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CalculationCellId, CalculationCellResult, CalculationHints, CellAddress, CellContent,
        CellValue, DateSystem, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
        Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName, SheetVisibility,
        WorkbookSnapshot, WorkbookSource,
    };

    #[test]
    fn worksheet_yield_recovers_independent_near_frequency_root_literal() {
        let sheet_id = SheetId::new(1).unwrap();
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new("YieldBoundary").unwrap(),
            SheetVisibility::Visible,
        );
        sheet
            .insert_cell(
                CellAddress::from_a1("A1").unwrap(),
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    FormulaText::from_xlsx(
                        "YIELD(DATE(2025,1,1),DATE(2027,1,1),0.05,16421050,100,2,0)",
                    )
                    .unwrap(),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .unwrap();
        let workbook = WorkbookSnapshot::new(
            vec![sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("fixed-income-test", "1").unwrap(),
                None,
            ),
        )
        .unwrap();
        let calculation =
            crate::calculation::calculate_workbook(&workbook, crate::CalculationOptions::default());
        let Some(CalculationCellResult::Value(CellValue::Number(value))) = calculation.cell(
            CalculationCellId::new(sheet_id, CellAddress::from_a1("A1").unwrap()),
        ) else {
            panic!("expected numeric near-boundary YIELD result");
        };
        assert!((value.get() + 1.9).abs() < 1e-10);
    }
}
