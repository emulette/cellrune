/// Odd-first and odd-last price/yield kernels and adapters.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::model::CouponFrequency;
use super::odd_schedule::{OddFirstMeasures, odd_first_measures, odd_last_measures};
use super::schedule::estimated_periods;
use super::{
    cash_flow_reduction_with_poll, charge_work, check_cancellation, coerce_basis, coerce_date,
    coerce_frequency, coupon_amount, date_from_serial_arg, finite_number, poll_loop_cancellation,
};

pub(super) fn odd_f_price(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 8 || args.len() > 9 {
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
    let first_coupon = match coerce_date(engine, context, &args[3]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let yield_ = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[6]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[7]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(8)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > first_coupon && first_coupon > settlement && settlement > issue) {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || yield_ < 0.0 || redemption <= 0.0 {
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
    let first_coupon_date = match date_from_serial_arg(first_coupon, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(work) = estimated_periods(issue_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work.saturating_mul(3)) {
        return Value::Error(kind);
    }

    let mut poll = || check_cancellation(context);
    let measures = match odd_first_measures(
        issue_date,
        first_coupon_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
        &mut poll,
    ) {
        Ok(Some(measures)) => measures,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let flows = match odd_first_flows(&measures, rate, redemption, frequency, &mut poll) {
        Ok(flows) => flows,
        Err(kind) => return Value::Error(kind),
    };
    let accrued = flows.accrued_interest;
    let (value, _) =
        match cash_flow_reduction_with_poll(&flows.flows, frequency.as_f64(), yield_, &mut poll) {
            Ok(reduction) => reduction,
            Err(kind) => return Value::Error(kind),
        };
    finite_number(value - accrued)
}

pub(super) fn odd_l_price(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 7 || args.len() > 8 {
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
    let last_interest = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let yield_ = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[6]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(7)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > settlement && settlement > last_interest) {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || yield_ < 0.0 || redemption <= 0.0 {
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
    let last_interest_date = match date_from_serial_arg(last_interest, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(work) = estimated_periods(last_interest_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work) {
        return Value::Error(kind);
    }
    let mut poll = || check_cancellation(context);
    let measures = match odd_last_measures(
        last_interest_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
        &mut poll,
    ) {
        Ok(Some(measures)) => measures,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let coupon = coupon_amount(rate, frequency);
    let result = (redemption + coupon * measures.coupon_days_fraction)
        / (1.0 + (yield_ / frequency.as_f64()) * measures.to_maturity_fraction)
        - coupon * measures.accrued_fraction;
    finite_number(result)
}

pub(super) fn odd_l_yield(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 7 || args.len() > 8 {
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
    let last_interest = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let price = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[6]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(7)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > settlement && settlement > last_interest) {
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
    let last_interest_date = match date_from_serial_arg(last_interest, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(work) = estimated_periods(last_interest_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work) {
        return Value::Error(kind);
    }
    let mut poll = || check_cancellation(context);
    let measures = match odd_last_measures(
        last_interest_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
        &mut poll,
    ) {
        Ok(Some(measures)) => measures,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let coupon = coupon_amount(rate, frequency);
    let result = ((redemption + coupon * measures.coupon_days_fraction)
        / (price + coupon * measures.accrued_fraction)
        - 1.0)
        * frequency.as_f64()
        / measures.to_maturity_fraction;
    finite_number(result)
}

pub(super) struct OddFirstFlows {
    pub(super) flows: Vec<(f64, f64)>,
    pub(super) accrued_interest: f64,
}

pub(super) fn odd_first_flows(
    measures: &OddFirstMeasures,
    rate: f64,
    redemption: f64,
    frequency: CouponFrequency,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<OddFirstFlows, ErrorKind> {
    if measures.is_long_first {
        odd_first_long_flows(measures, rate, redemption, frequency, poll)
    } else {
        odd_first_short_flows(measures, rate, redemption, frequency, poll)
    }
}

fn odd_first_short_flows(
    measures: &OddFirstMeasures,
    rate: f64,
    redemption: f64,
    frequency: CouponFrequency,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<OddFirstFlows, ErrorKind> {
    let coupon = coupon_amount(rate, frequency);
    let first_time = measures.settlement_to_first_fraction;
    let first_cash_flow = coupon * measures.issue_to_first_fraction;

    let mut flows = Vec::with_capacity(measures.coupon_count.max(0) as usize + 1);
    flows.push((first_time, first_cash_flow));
    for k in 2..=measures.coupon_count {
        poll_loop_cancellation(k as usize, poll)?;
        flows.push(((k - 1) as f64 + first_time, coupon));
    }
    flows.push(((measures.coupon_count - 1) as f64 + first_time, redemption));

    Ok(OddFirstFlows {
        flows,
        accrued_interest: coupon * measures.accrued_fraction,
    })
}

fn odd_first_long_flows(
    measures: &OddFirstMeasures,
    rate: f64,
    redemption: f64,
    frequency: CouponFrequency,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<OddFirstFlows, ErrorKind> {
    let coupon = coupon_amount(rate, frequency);
    let nq = measures.settlement_to_first_fraction.floor();
    let dsc_over_e = measures.settlement_to_first_fraction - nq;
    let first_time = nq + dsc_over_e;
    let n = measures.coupon_count - 1;

    let mut flows = Vec::with_capacity(measures.coupon_count.max(0) as usize + 1);
    // Microsoft long-first notation: `Σ DCᵢ/NLᵢ` is paid at `Nq + DSC/E`.
    flows.push((first_time, coupon * measures.issue_to_first_fraction));
    // The regular coupon sum is `k=1..N` at `k + Nq + DSC/E`.
    for k in 1..=n {
        poll_loop_cancellation(k as usize, poll)?;
        flows.push((k as f64 + first_time, coupon));
    }
    // Redemption is paid at `N + Nq + DSC/E`.
    flows.push((n as f64 + first_time, redemption));

    Ok(OddFirstFlows {
        flows,
        // Microsoft long-first notation: clean price subtracts `C × Σ Aᵢ/NLᵢ`.
        accrued_interest: coupon * measures.accrued_fraction,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::super::cash_flow_reduction;
    use super::super::model::DayCountBasis;
    use super::*;
    use crate::{
        CalculationCellId, CalculationCellResult, CalculationHints, CellAddress, CellContent,
        CellValue, DateSystem, ExcelError, FormulaCell, FormulaDialect, FormulaMetadata,
        FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName,
        SheetVisibility, WorkbookSnapshot, WorkbookSource,
    };

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn odd_first_short_price_matches_frozen_reference() {
        let measures = odd_first_measures(
            date(2024, 11, 15),
            date(2025, 3, 1),
            date(2025, 2, 1),
            date(2030, 3, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        let flows = odd_first_flows(
            &measures,
            0.05,
            100.0,
            CouponFrequency::Semiannual,
            &mut || Ok(()),
        )
        .unwrap();
        let (value, _) = cash_flow_reduction(&flows.flows, 2.0, 0.06);
        assert!((value - flows.accrued_interest - 95.673_855_249_014_57).abs() < 1e-12);
    }

    #[test]
    fn microsoft_odd_first_example_rounds_to_documented_price() {
        let measures = odd_first_measures(
            date(2008, 10, 15),
            date(2009, 3, 1),
            date(2008, 11, 11),
            date(2021, 3, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::ActualActual,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        let flows = odd_first_flows(
            &measures,
            0.0785,
            100.0,
            CouponFrequency::Semiannual,
            &mut || Ok(()),
        )
        .unwrap();
        let (dirty, _) = cash_flow_reduction(&flows.flows, 2.0, 0.0625);
        let price = dirty - flows.accrued_interest;
        assert!((price - 113.60).abs() < 0.005);
    }

    #[test]
    fn odd_first_long_price_uses_quasi_coupon_amount_and_time() {
        let measures = odd_first_measures(
            date(2023, 9, 1),
            date(2025, 3, 1),
            date(2025, 2, 1),
            date(2030, 3, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        let flows = odd_first_flows(
            &measures,
            0.05,
            100.0,
            CouponFrequency::Semiannual,
            &mut || Ok(()),
        )
        .unwrap();
        assert!((flows.flows[0].0 - 1.0 / 6.0).abs() < 1e-12);
        assert_eq!(flows.flows[0].1, 7.5);
        assert!((flows.accrued_interest - 2.5 * 17.0 / 6.0).abs() < 1e-12);
        let (dirty, _) = cash_flow_reduction(&flows.flows, 2.0, 0.06);
        assert!((dirty - flows.accrued_interest - 95.644_232_627_811_8).abs() < 1e-12);

        let supplied_price = dirty - flows.accrued_interest;
        let residual = |yield_: f64| {
            let (value, weighted) = cash_flow_reduction(&flows.flows, 2.0, yield_);
            let q = 1.0 + yield_ / 2.0;
            Ok((
                value - flows.accrued_interest - supplied_price,
                -weighted / (2.0 * q),
            ))
        };
        let recovered = super::super::solver::solve_with_charge(
            -2.0,
            flows.flows.len(),
            super::super::solver::EXTENDED_YIELD_POLICY,
            &residual,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert!((recovered - 0.06).abs() < 1e-12);
    }

    #[test]
    fn odd_last_price_and_yield_match_frozen_literals() {
        let measures = odd_last_measures(
            date(2024, 10, 15),
            date(2025, 2, 1),
            date(2025, 6, 15),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        let coupon = coupon_amount(0.05, CouponFrequency::Semiannual);
        let price = (100.0 + coupon * measures.coupon_days_fraction)
            / (1.0 + (0.06 / 2.0) * measures.to_maturity_fraction)
            - coupon * measures.accrued_fraction;
        assert!((price - 99.603_747_781_038_28).abs() < 1e-12);

        let yield_ = ((100.0 + coupon * measures.coupon_days_fraction)
            / (99.0 + coupon * measures.accrued_fraction)
            - 1.0)
            * 2.0
            / measures.to_maturity_fraction;
        assert!((yield_ - 0.076_504_400_859_952_39).abs() < 1e-12);
    }

    #[test]
    fn all_odd_adapters_reject_zero_and_negative_redemption() {
        let cases = [
            (
                "A1",
                "ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,0,2,0)",
            ),
            (
                "A2",
                "ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2023,9,1),DATE(2025,3,1),0.05,0.06,-1,2,0)",
            ),
            (
                "A3",
                "ODDFYIELD(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,95,0,2,0)",
            ),
            (
                "A4",
                "ODDFYIELD(DATE(2025,2,1),DATE(2030,3,1),DATE(2023,9,1),DATE(2025,3,1),0.05,95,-1,2,0)",
            ),
            (
                "A5",
                "ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,0.06,0,2,0)",
            ),
            (
                "A6",
                "ODDLPRICE(DATE(2024,2,1),DATE(2025,6,15),DATE(2024,1,1),0.05,0.06,-1,2,0)",
            ),
            (
                "A7",
                "ODDLYIELD(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,99,0,2,0)",
            ),
            (
                "A8",
                "ODDLYIELD(DATE(2024,2,1),DATE(2025,6,15),DATE(2024,1,1),0.05,99,-1,2,0)",
            ),
        ];
        let sheet_id = SheetId::new(1).unwrap();
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new("OddRedemption").unwrap(),
            SheetVisibility::Visible,
        );
        for (address, formula) in cases {
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
        for (address, _) in cases {
            assert_eq!(
                calculation.cell(CalculationCellId::new(
                    sheet_id,
                    CellAddress::from_a1(address).unwrap(),
                )),
                Some(&CalculationCellResult::Value(CellValue::Error(
                    ExcelError::Number
                ))),
                "unexpected redemption boundary result at {address}"
            );
        }
    }
}
