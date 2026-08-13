/// Discount, maturity-interest, and T-bill-adjacent closed-form adapters.
///
/// `DISC`, `INTRATE`, `PRICEDISC`, `RECEIVED`, and `YIELDDISC` share the discount-security day
/// count. `PRICEMAT` and `YIELDMAT` add the issue→settlement accrued fraction and are therefore
/// not aliases of the discount pair.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::day_count::{annual_denominator, days_between};
use super::{coerce_basis, coerce_date, date_from_serial_arg, finite_number};

pub(super) fn disc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    discount_security(engine, context, args, DiscountMeasure::Disc)
}

pub(super) fn int_rate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    discount_security(engine, context, args, DiscountMeasure::IntRate)
}

pub(super) fn price_disc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    discount_security(engine, context, args, DiscountMeasure::PriceDisc)
}

pub(super) fn received(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    discount_security(engine, context, args, DiscountMeasure::Received)
}

pub(super) fn yield_disc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    discount_security(engine, context, args, DiscountMeasure::YieldDisc)
}

pub(super) fn price_mat(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    maturity_security(engine, context, args, MaturityMeasure::Price)
}

pub(super) fn yield_mat(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    maturity_security(engine, context, args, MaturityMeasure::Yield)
}

enum DiscountMeasure {
    Disc,
    IntRate,
    PriceDisc,
    Received,
    YieldDisc,
}

enum MaturityMeasure {
    Price,
    Yield,
}

fn discount_security(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    measure: DiscountMeasure,
) -> Value {
    if args.len() < 4 || args.len() > 5 {
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
    let primary = match required_number(engine, context, &args[2]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(4)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };
    if settlement >= maturity {
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
    let dsm = days_between(settlement_date, maturity_date, basis);
    let denominator = annual_denominator(basis, settlement_date, maturity_date);
    if dsm <= 0.0 || denominator <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let result = match measure {
        DiscountMeasure::Disc => {
            if primary <= 0.0 || redemption <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            (redemption - primary) / redemption * denominator / dsm
        }
        DiscountMeasure::IntRate => {
            if primary <= 0.0 || redemption <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            (redemption - primary) / primary * denominator / dsm
        }
        DiscountMeasure::PriceDisc => {
            if primary <= 0.0 || redemption <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            redemption * (1.0 - primary * dsm / denominator)
        }
        DiscountMeasure::Received => {
            if primary <= 0.0 || redemption <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            let factor = 1.0 - redemption * dsm / denominator;
            if factor <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            primary / factor
        }
        DiscountMeasure::YieldDisc => {
            if primary <= 0.0 || redemption <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            (redemption - primary) / primary * denominator / dsm
        }
    };
    finite_number(result)
}

fn maturity_security_date_order(issue: f64, settlement: f64, maturity: f64) -> bool {
    issue < settlement && settlement < maturity
}

fn maturity_security(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    measure: MaturityMeasure,
) -> Value {
    if args.len() < 5 || args.len() > 6 {
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
    let issue = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let price_or_yield = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(5)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };
    if !maturity_security_date_order(issue, settlement, maturity) {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 {
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
    let issue_date = match date_from_serial_arg(issue, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let dim = days_between(issue_date, maturity_date, basis);
    let dsm = days_between(settlement_date, maturity_date, basis);
    let accrued = days_between(issue_date, settlement_date, basis);
    let denominator = annual_denominator(basis, issue_date, maturity_date);
    if dsm <= 0.0 || denominator <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let redemption_value = 100.0 + 100.0 * rate * dim / denominator;
    let accrued_value = 100.0 * rate * accrued / denominator;
    let result = match measure {
        MaturityMeasure::Price => {
            if price_or_yield < 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            redemption_value / (1.0 + price_or_yield * dsm / denominator) - accrued_value
        }
        MaturityMeasure::Yield => {
            if price_or_yield <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            (redemption_value / (price_or_yield + accrued_value) - 1.0) * denominator / dsm
        }
    };
    finite_number(result)
}

#[cfg(test)]
mod tests {
    use super::maturity_security_date_order;
    use crate::{
        CalculationCellId, CalculationCellResult, CalculationHints, CellAddress, CellContent,
        CellValue, DateSystem, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
        Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName, SheetVisibility,
        WorkbookSnapshot, WorkbookSource,
    };

    #[test]
    fn maturity_security_requires_issue_before_settlement() {
        assert!(maturity_security_date_order(10.0, 20.0, 30.0));
        assert!(!maturity_security_date_order(20.0, 20.0, 30.0));
        assert!(!maturity_security_date_order(25.0, 20.0, 30.0));
    }

    #[test]
    fn worksheet_disc_matches_multi_year_actual_actual_rationals() {
        let sheet_id = SheetId::new(1).unwrap();
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new("ActualActual").unwrap(),
            SheetVisibility::Visible,
        );
        for (address, formula) in [
            ("A1", "DISC(DATE(2019,7,1),DATE(2021,7,1),95,100,1)"),
            ("A2", "DISC(DATE(2019,12,31),DATE(2022,1,1),97,100,1)"),
        ] {
            sheet
                .insert_cell(
                    CellAddress::from_a1(address).unwrap(),
                    CellContent::Formula(FormulaCell::new(
                        FormulaDialect::ExcelA1,
                        FormulaText::from_xlsx(formula).unwrap(),
                        SavedResult::Missing,
                        FormulaMetadata::Normal,
                    )),
                )
                .unwrap();
        }
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
        for (address, expected) in [("A1", 274.0 / 10_965.0), ("A2", 1_461.0 / 97_600.0)] {
            let Some(CalculationCellResult::Value(CellValue::Number(value))) = calculation.cell(
                CalculationCellId::new(sheet_id, CellAddress::from_a1(address).unwrap()),
            ) else {
                panic!("expected numeric multi-year DISC result at {address}");
            };
            assert!((value.get() - expected).abs() < 1e-15);
        }
    }
}
