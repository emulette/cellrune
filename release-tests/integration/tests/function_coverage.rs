use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellValue, DateSystem, ExcelError, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName,
    SheetVisibility, WorkbookSnapshot, WorkbookSource, calculate_workbook,
    scan_formula_capabilities,
};

#[derive(Debug, Clone, Copy)]
enum Expected {
    Number { value: f64, tolerance: f64 },
    Text(&'static str),
    Logical(bool),
    Error(ExcelError),
}

#[derive(Debug, Clone, Copy)]
struct FormulaCase {
    formula: &'static str,
    expected: Expected,
}

#[test]
fn math_trigonometry_and_combinatorics_match_documented_examples() {
    let pi = std::f64::consts::PI;
    let cases = [
        number("ACOS(-1)", pi, 1e-14),
        number("ACOSH(1)", 0.0, 0.0),
        number("ACOT(1)", pi / 4.0, 1e-14),
        number("ACOTH(2)", 0.549_306_144_334_054_9, 1e-14),
        number("ASIN(1)", pi / 2.0, 1e-14),
        number("ASINH(1)", 0.881_373_587_019_543, 1e-14),
        number("ATAN(1)", pi / 4.0, 1e-14),
        number("ATAN2(-1,-1)", -3.0 * pi / 4.0, 1e-14),
        number("ATANH(0.5)", 0.549_306_144_334_054_9, 1e-14),
        number("COS(0)", 1.0, 0.0),
        number("COSH(0)", 1.0, 0.0),
        number("COT(PI()/4)", 1.0, 1e-14),
        number("COTH(1)", 1.313_035_285_499_331_2, 1e-14),
        number("CSC(PI()/2)", 1.0, 1e-14),
        number("CSCH(1)", 0.850_918_128_239_321_6, 1e-14),
        number("DEGREES(PI())", 180.0, 1e-14),
        number("RADIANS(180)", pi, 1e-14),
        number("SEC(0)", 1.0, 0.0),
        number("SECH(0)", 1.0, 0.0),
        number("SIN(PI()/2)", 1.0, 1e-14),
        number("SINH(0)", 0.0, 0.0),
        number("TAN(PI()/4)", 1.0, 1e-14),
        number("TANH(0)", 0.0, 0.0),
        number("EVEN(-1.1)", -2.0, 0.0),
        number("ODD(0)", 1.0, 0.0),
        number("LOG(8,2)", 3.0, 1e-14),
        number("LOG10(100)", 2.0, 1e-14),
        number("MROUND(10,3)", 9.0, 0.0),
        number("PI()", pi, 0.0),
        number("QUOTIENT(-7,3)", -2.0, 0.0),
        number("SQRTPI(1)", pi.sqrt(), 1e-14),
        number("FACT(5)", 120.0, 0.0),
        number("FACTDOUBLE(10)", 3_840.0, 0.0),
        number("GCD(24,36)", 12.0, 0.0),
        number("LCM(4,6)", 12.0, 0.0),
        number("COMBIN(8,2)", 28.0, 0.0),
        number("COMBINA(4,3)", 20.0, 0.0),
        number("PERMUT(8,2)", 56.0, 0.0),
        number("PERMUTATIONA(3,2)", 9.0, 0.0),
        number("MULTINOMIAL(2,3,4)", 1_260.0, 0.0),
        number("SUMSQ(3,4)", 25.0, 0.0),
        number("SUMX2MY2({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})", -55.0, 0.0),
        number("SUMX2PY2({2,3},{4,5})", 54.0, 0.0),
        number("SUMXMY2({2,3},{4,5})", 8.0, 0.0),
    ];
    assert_eq!(cases.len(), 44);
    let workbook = workbook_with_formula_cases(&cases);

    let capabilities = scan_formula_capabilities(&workbook);
    assert!(
        capabilities.is_supported(),
        "new function names must be supported by capability analysis: {capabilities:?}"
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn new_functions_propagate_excel_errors_at_documented_boundaries() {
    let cases = [
        error("ACOS(2)", ExcelError::Number),
        error("ACOTH(1)", ExcelError::Number),
        error("ATAN2(0,0)", ExcelError::DivisionByZero),
        error("COT(0)", ExcelError::DivisionByZero),
        error("COT(134217728)", ExcelError::Number),
        error("LOG(-1)", ExcelError::Number),
        error("MROUND(10,-3)", ExcelError::Number),
        error("QUOTIENT(1,0)", ExcelError::DivisionByZero),
        error("FACT(-1)", ExcelError::Number),
        error("COMBIN(2,3)", ExcelError::Number),
        error("PERMUTATIONA(0,1)", ExcelError::Number),
        error("SUMXMY2({1,2},{1})", ExcelError::NotAvailable),
        error("SUMSQ(\"not numeric\")", ExcelError::Value),
        error("PI(1)", ExcelError::Value),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn v0_1_10_text_and_pcre2_functions_match_documented_semantics() {
    let cases = [
        text(
            "ARRAYTOTEXT({0.1,\"K001\",\"A\";0.2,\"K002\",\"B\";-0.3,\"K003\",\"C\"},1)",
            "{0.1,\"K001\",\"A\";0.2,\"K002\",\"B\";-0.3,\"K003\",\"C\"}",
        ),
        text("ARRAYTOTEXT({\"a\",\"b\"},0)", "a, b"),
        text("REGEXEXTRACT(\"CR-2026-0727\",\"[0-9]{4}\")", "2026"),
        text(
            "REGEXEXTRACT(\"prefix-cell-cell\",\"(?<=prefix-)([a-z]+)-\\1\")",
            "cell-cell",
        ),
        text(
            "REGEXREPLACE(\"CR-2026-0727\",\"[0-9]\",\"#\")",
            "CR-####-####",
        ),
        text(
            "REGEXREPLACE(\"2026-0727\",\"([0-9]{4})-([0-9]{4})\",\"$2/$1\")",
            "0727/2026",
        ),
        text(
            "REGEXREPLACE(\"cellrune\",\"(?<word>[a-z]+)\",\"$<word>-$&\")",
            "cellrune-cellrune",
        ),
        text("REGEXREPLACE(\"a\",\"(?:|a)\",\"X\")", "XXX"),
        text(
            "REGEXREPLACE(\"b\",\"(?J)(?<part>a)|(?<part>b)\",\"${part}\")",
            "b",
        ),
        text(
            "REGEXREPLACE(\"a1 b22 c333\",\"[0-9]+\",\"#\",-2)",
            "a1 b# c333",
        ),
        logical("REGEXTEST(\"CellRune\",\"^cellrune$\",1)", true),
        logical("REGEXTEST(\"CellRune\",\"^cellrune$\")", false),
        logical("REGEXTEST(\"a\",\"(*NO_AUTO_POSSESS)a\")", true),
        logical("REGEXTEST(\"a\",\"(*UTF)a\")", true),
        logical("REGEXTEST(\"abc\",\"\\Qabc\")", true),
        text(
            "REGEXREPLACE(\"a\",\"(*NO_AUTO_POSSESS)(?:|a)\",\"X\")",
            "XXX",
        ),
        text("REGEXREPLACE(\"a\",\"(?x)# (?R)\n(?:|a)\",\"X\")", "XXX"),
        text("REGEXREPLACE(\"a\",\"(*MARK:(?R)\",\"X\")", "XaX"),
        text("REGEXREPLACE(\"a\",\"(?C\"\"(?R)\"\")\",\"X\")", "XaX"),
        text(
            "REGEXREPLACE(\"a\",\"(?x)# comment\"&CHAR(13)&\"|a(?R)\",\"X\")",
            "XaX",
        ),
        text("REGEXREPLACE(\"a\",\"[](?R)]|\",\"X\")", "XaX"),
        text("REGEXREPLACE(\"a\",\"\\Q\",\"X\")", "XaX"),
        text("TEXTSPLIT(\"alpha|beta\",\"|\")", "alpha"),
        error("REGEXEXTRACT(\"abc\",\"[0-9]+\")", ExcelError::NotAvailable),
        error("REGEXTEST(\"abc\",\"(\")", ExcelError::Value),
        error(
            "REGEXTEST(\"a\",\"(*LIMIT_MATCH=999999999999999)a\")",
            ExcelError::Value,
        ),
        error("REGEXREPLACE(\"a\",\"|a(?0)\",\"X\")", ExcelError::Value),
        error("REGEXREPLACE(\"a\",\"|a(?000)\",\"X\")", ExcelError::Value),
        error("REGEXREPLACE(\"a\",\"|a\\g<00>\",\"X\")", ExcelError::Value),
        error(
            "REGEXREPLACE(\"a\",\"(?#\\)|a(?R)\",\"X\")",
            ExcelError::Value,
        ),
        error(
            "REGEXREPLACE(\"a\",\"(*CR)(?x)# comment\"&CHAR(13)&\"|a(?R)\",\"X\")",
            ExcelError::Value,
        ),
        error("REGEXREPLACE(\"b\",\"(a)?b\",\"$1\")", ExcelError::Value),
        error("TEXTSPLIT(\"abc\",\"\")", ExcelError::Value),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn collection_arguments_follow_excel_direct_and_range_coercion_rules() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    insert_literal(&mut sheet, 1, 1, CellValue::number(2.0).expect("finite"));
    insert_literal(&mut sheet, 2, 1, CellValue::Text("3".to_owned()));
    insert_literal(&mut sheet, 3, 1, CellValue::Logical(true));
    insert_literal(&mut sheet, 5, 1, CellValue::Error(ExcelError::NotAvailable));

    for (row, formula) in [
        (1, "SUMSQ(A1:A4)"),
        (2, "SUMSQ(A1:A5)"),
        (3, "GCD(A1:A4,6)"),
        (4, "SUMSQ(\"2\",TRUE,{3,\"4\",TRUE})"),
        (5, "GCD(\"12\",TRUE,{18,\"24\",FALSE})"),
    ] {
        insert_formula(&mut sheet, row, 2, formula);
    }
    let workbook = workbook_with_sheet(sheet);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_cell_number(&calculation, 1, 2, 4.0, 0.0);
    assert_cell_error(&calculation, 2, 2, ExcelError::NotAvailable);
    assert_cell_number(&calculation, 3, 2, 2.0, 0.0);
    assert_cell_number(&calculation, 4, 2, 14.0, 0.0);
    assert_cell_number(&calculation, 5, 2, 1.0, 0.0);
}

#[test]
fn second_expansion_wave_matches_documented_examples_and_aliases() {
    let normal_cumulative = 0.5 * libm::erfc(-1.0 / std::f64::consts::SQRT_2);
    let normal_density = (-0.5_f64).exp() / (2.0 * std::f64::consts::PI).sqrt();
    let effective = (1.0_f64 + 0.0525 / 4.0).powi(4) - 1.0;
    let mirr = 1.324_32_f64.powf(1.0 / 3.0) - 1.0;
    let cases = [
        number("ERROR.TYPE(NA())", 7.0, 0.0),
        logical("ISERR(1/0)", true),
        logical("ISNONTEXT(1)", true),
        logical("ISREF($A$1048576)", true),
        logical("XOR(TRUE,FALSE,TRUE)", false),
        text("CLEAN(\"abc\")", "abc"),
        text("CONCATENATE(\"a\",1,TRUE)", "a1TRUE"),
        text("UNICHAR(9731)", "☃"),
        number("UNICODE(\"☃\")", 9_731.0, 0.0),
        text("TEXTBEFORE(\"red-blue\",\"-\")", "red"),
        text("TEXTAFTER(\"red-blue\",\"-\")", "blue"),
        text("VALUETOTEXT(\"abc\",1)", "\"abc\""),
        number("DAYS(DATE(2021,3,15),DATE(2021,2,1))", 42.0, 0.0),
        number("DAYS360(DATE(2011,1,1),DATE(2011,12,31))", 360.0, 0.0),
        number("HOUR(0.75)", 18.0, 0.0),
        number("MINUTE(TIME(12,34,56))", 34.0, 0.0),
        number("SECOND(TIME(12,34,56))", 56.0, 0.0),
        number("TIME(12,34,56)", 45_296.0 / 86_400.0, 1e-14),
        number("ISOWEEKNUM(DATE(2012,3,9))", 10.0, 0.0),
        number("WEEKNUM(DATE(2012,3,9),2)", 11.0, 0.0),
        number("CEILING.MATH(-4.3,2)", -4.0, 0.0),
        number("CEILING.PRECISE(-4.3,2)", -4.0, 0.0),
        number("FLOOR.MATH(-4.3,2)", -6.0, 0.0),
        number("FLOOR.PRECISE(-4.3,2)", -6.0, 0.0),
        number("ISO.CEILING(-4.3,2)", -4.0, 0.0),
        text("BASE(15,2,8)", "00001111"),
        number("DECIMAL(\"FF\",16)", 255.0, 0.0),
        number("SERIESSUM(1,0,1,{1,2,3})", 6.0, 0.0),
        number("BIN2DEC(1100100)", 100.0, 0.0),
        text("BIN2HEX(11111011,4)", "00FB"),
        text("BIN2OCT(1001,3)", "011"),
        number("BITAND(13,25)", 9.0, 0.0),
        number("BITLSHIFT(4,2)", 16.0, 0.0),
        number("BITOR(23,10)", 31.0, 0.0),
        number("BITRSHIFT(13,2)", 3.0, 0.0),
        number("BITXOR(5,3)", 6.0, 0.0),
        text("DEC2BIN(9,4)", "1001"),
        text("DEC2HEX(100,4)", "0064"),
        text("DEC2OCT(58,3)", "072"),
        number("DELTA(5,4)", 0.0, 0.0),
        number("ERF(0)", 0.0, 0.0),
        number("ERF.PRECISE(1)", libm::erf(1.0), 1e-14),
        number("ERFC(0)", 1.0, 0.0),
        number("ERFC.PRECISE(1)", libm::erfc(1.0), 1e-14),
        number("GESTEP(5,4)", 1.0, 0.0),
        text("HEX2BIN(\"F\",8)", "00001111"),
        number("HEX2DEC(\"A5\")", 165.0, 0.0),
        text("HEX2OCT(\"F\",3)", "017"),
        text("OCT2BIN(7,4)", "0111"),
        number("OCT2DEC(54)", 44.0, 0.0),
        text("OCT2HEX(100,4)", "0040"),
        number("DOLLARDE(1.02,16)", 1.125, 1e-14),
        number("DOLLARFR(1.125,16)", 1.02, 1e-14),
        number("EFFECT(0.0525,4)", effective, 1e-14),
        number("FVSCHEDULE(1,{0.1,0.2})", 1.32, 1e-14),
        number("ISPMT(0.1,1,10,1000)", -90.0, 1e-14),
        number("MIRR({-100,30,40,50},0.1,0.12)", mirr, 1e-14),
        number("NOMINAL(0.053542667,4)", 0.0525, 1e-9),
        number(
            "PDURATION(0.1,1000,2000)",
            2.0_f64.ln() / 1.1_f64.ln(),
            1e-14,
        ),
        number("RRI(10,1000,2000)", 2.0_f64.powf(0.1) - 1.0, 1e-14),
        number("AVEDEV({2,4,6})", 4.0 / 3.0, 1e-14),
        number("AVERAGEA({2,TRUE,\"x\"})", 1.0, 1e-14),
        number("COVARIANCE.P({1,2,3},{2,4,6})", 4.0 / 3.0, 1e-14),
        number("DEVSQ({2,4,6})", 8.0, 1e-14),
        number("EXPON.DIST(1,2,TRUE)", 1.0 - (-2.0_f64).exp(), 1e-14),
        number("GAUSS(1)", normal_cumulative - 0.5, 1e-14),
        number("GEOMEAN(1,4)", 2.0, 1e-14),
        number("HARMEAN(1,2)", 4.0 / 3.0, 1e-14),
        number("INTERCEPT({2,4,6},{1,2,3})", 0.0, 1e-14),
        number("MAXA({-2,FALSE,\"x\"})", 0.0, 0.0),
        number("MINA({2,TRUE,\"x\"})", 0.0, 0.0),
        number("NORM.DIST(1,0,1,TRUE)", normal_cumulative, 1e-14),
        number("PEARSON({1,2,3},{2,4,6})", 1.0, 1e-14),
        number("PHI(0)", 1.0 / (2.0 * std::f64::consts::PI).sqrt(), 1e-14),
        number(
            "POISSON.DIST(2,3,FALSE)",
            (-3.0_f64).exp() * 9.0 / 2.0,
            1e-14,
        ),
        number("RSQ({1,2,3},{2,4,6})", 1.0, 1e-14),
        number("STANDARDIZE(42,40,1.5)", 4.0 / 3.0, 1e-14),
        number("STDEV.P({2,4})", 1.0, 1e-14),
        number("VAR.P({2,4})", 1.0, 1e-14),
        number("COVAR({1,2,3},{2,4,6})", 4.0 / 3.0, 1e-14),
        number("EXPONDIST(1,2,FALSE)", 2.0 * (-2.0_f64).exp(), 1e-14),
        number("MODE(1,2,2)", 2.0, 0.0),
        number("NORMDIST(1,0,1,FALSE)", normal_density, 1e-14),
        number("POISSON(2,3,FALSE)", (-3.0_f64).exp() * 9.0 / 2.0, 1e-14),
        number("QUARTILE({1,2,3,4},2)", 2.5, 1e-14),
        number("RANK(3,{1,2,3})", 1.0, 0.0),
        number("STDEVP({2,4})", 1.0, 1e-14),
        number("VAR(2,4)", 2.0, 1e-14),
        number("VARP({2,4})", 1.0, 1e-14),
    ];
    assert_eq!(cases.len(), 89);
    let workbook = workbook_with_formula_cases(&cases);

    let capabilities = scan_formula_capabilities(&workbook);
    assert!(
        capabilities.is_supported(),
        "all 89 second-wave names must pass capability analysis: {capabilities:?}"
    );
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn second_expansion_wave_rejects_invalid_domains_and_widths() {
    let mirr = 1.324_32_f64.powf(1.0 / 3.0) - 1.0;
    let cases = [
        error("UNICHAR(0)", ExcelError::Value),
        error("TEXTBEFORE(\"abc\",\"-\")", ExcelError::NotAvailable),
        text("TEXTBEFORE(\"abc\",\"x\",-1,0,1)", ""),
        text("TEXTAFTER(\"abc\",\"x\",-1,0,1)", "abc"),
        error("HEX2DEC(\" A5 \")", ExcelError::Number),
        error("TIME(-1,0,0)", ExcelError::Number),
        error("BASE(-1,2)", ExcelError::Number),
        error("DECIMAL(\"2\",2)", ExcelError::Number),
        error("BITAND(-1,1)", ExcelError::Number),
        error("DEC2BIN(512)", ExcelError::Number),
        error("EFFECT(-0.1,4)", ExcelError::Number),
        error("FVSCHEDULE(1,{0.1,\"x\"})", ExcelError::Value),
        error("MIRR({-100,-50,200},-1,0.1)", ExcelError::DivisionByZero),
        error("MIRR({-100,30,40,50},0.1,-1)", ExcelError::DivisionByZero),
        number("MIRR({-100,30,40,50},-1,0.12)", mirr, 1e-14),
        error("RRI(-1,1000,2000)", ExcelError::Number),
        error("GEOMEAN(0,1)", ExcelError::Number),
        error(
            "HARMEAN($XFD$1048575:$XFD$1048576)",
            ExcelError::NotAvailable,
        ),
        error("NORM.DIST(1,0,0,TRUE)", ExcelError::Number),
        error("POISSON.DIST(1,0,TRUE)", ExcelError::Number),
        text("VALUETOTEXT(1/0,1)", "#DIV/0!"),
        logical("XOR({TRUE,FALSE,TRUE})", false),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn gamma_distribution_family_matches_documented_examples() {
    let cases = [
        number("GAMMA(2.5)", 1.329_340_388_179_137, 1e-12),
        number("GAMMA(-3.75)", 0.267_866_128_861_416_6, 1e-12),
        number(
            "GAMMA.DIST(10.00001131,9,2,FALSE)",
            0.032_639_130_418_294,
            1e-12,
        ),
        number(
            "GAMMA.DIST(10.00001131,9,2,TRUE)",
            0.068_094_003_869_787_33,
            1e-12,
        ),
        number("GAMMA.INV(0.068094,9,2)", 10.000_011_191_437_178, 1e-8),
        number("GAMMALN(4)", 1.791_759_469_228_055, 1e-12),
        number("GAMMALN.PRECISE(4)", 1.791_759_469_228_055, 1e-12),
        number(
            "GAMMADIST(10.00001131,9,2,TRUE)",
            0.068_094_003_869_787_33,
            1e-12,
        ),
        number("GAMMAINV(0.068094,9,2)", 10.000_011_191_437_178, 1e-8),
        error("GAMMA(0)", ExcelError::Number),
        error("GAMMA(-2)", ExcelError::Number),
        error("GAMMA.DIST(-1,9,2,TRUE)", ExcelError::Number),
        error("GAMMA.INV(1,9,2)", ExcelError::Number),
        error("GAMMALN(0)", ExcelError::Number),
        error("GAMMALN.PRECISE(-1)", ExcelError::Number),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn binomial_distribution_family_matches_documented_examples() {
    let cases = [
        number("BINOM.DIST(6,10,0.5,FALSE)", 0.205_078_125, 1e-12),
        number("BINOM.DIST(6,10,0.5,TRUE)", 0.828_125, 1e-12),
        number("BINOMDIST(6,10,0.5,FALSE)", 0.205_078_125, 1e-12),
        number(
            "BINOM.DIST.RANGE(60,0.75,48)",
            0.083_974_967_429_047_5,
            1e-12,
        ),
        number(
            "BINOM.DIST.RANGE(60,0.75,45,50)",
            0.523_629_793_471_887_2,
            1e-12,
        ),
        number("BINOM.INV(6,0.5,0.75)", 4.0, 0.0),
        number("CRITBINOM(6,0.5,0.75)", 4.0, 0.0),
        number(
            "NEGBINOM.DIST(10,5,0.25,TRUE)",
            0.313_514_058_478_176_6,
            1e-12,
        ),
        number(
            "NEGBINOM.DIST(10,5,0.25,FALSE)",
            0.055_048_660_375_177_86,
            1e-12,
        ),
        number("NEGBINOMDIST(10,5,0.25)", 0.055_048_660_375_177_86, 1e-12),
        error("BINOM.DIST(-1,10,0.4,FALSE)", ExcelError::Number),
        error("BINOM.DIST.RANGE(10,0.4,3,11)", ExcelError::Number),
        error("BINOM.INV(10,0.4,2)", ExcelError::Number),
        error("NEGBINOM.DIST(-1,4,0.4,TRUE)", ExcelError::Number),
        error("NEGBINOMDIST(6,0,0.4)", ExcelError::Number),
    ];
    let workbook = workbook_with_formula_cases(&cases);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn beta_distribution_family_matches_documented_examples() {
    // Microsoft's documented example uses x = 2, alpha = 8, beta = 10 on the
    // interval [1, 3]; the docs round the results to 0.6854706 and 1.4837646.
    let cases = [
        number("BETA.DIST(2,8,10,TRUE,1,3)", 0.685_470_581_054_687_5, 1e-12),
        number("BETA.DIST(2,8,10,FALSE,1,3)", 1.483_764_648_437_5, 1e-12),
        number("BETA.INV(0.685470581,8,10,1,3)", 2.0, 1e-9),
        number("BETADIST(2,8,10,1,3)", 0.685_470_581_054_687_5, 1e-12),
        number("BETAINV(0.685470581,8,10,1,3)", 2.0, 1e-9),
        number("BETA.DIST(0.6,8,10,TRUE)", 0.908_100_745_828_761_5, 1e-12),
        number("BETA.INV(1,2,3)", 1.0, 0.0),
        error("BETA.DIST(2,8,10,TRUE,1,1)", ExcelError::Number),
        error("BETA.DIST(0.5,0,3,TRUE)", ExcelError::Number),
        error("BETA.DIST(0.5,2,-1,TRUE)", ExcelError::Number),
        error("BETA.DIST(-1,8,10,TRUE)", ExcelError::Number),
        error("BETA.INV(0,2,3)", ExcelError::Number),
        error("BETA.INV(1.5,2,3)", ExcelError::Number),
        error("BETADIST(-1,2,3)", ExcelError::Number),
        error("BETAINV(0,2,3)", ExcelError::Number),
        error("BETA.DIST(\"abc\",8,10,TRUE)", ExcelError::Value),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn hypergeometric_distribution_family_matches_documented_examples() {
    // Microsoft documents 1 success drawn from a sample of 4, taken from a
    // population of 20 holding 8 successes: 0.4654 cumulative, 0.3633 mass.
    let cases = [
        number(
            "HYPGEOM.DIST(1,4,8,20,TRUE)",
            0.465_428_276_573_787_44,
            1e-12,
        ),
        number(
            "HYPGEOM.DIST(1,4,8,20,FALSE)",
            0.363_261_093_911_248_7,
            1e-12,
        ),
        number("HYPGEOMDIST(1,4,8,20)", 0.363_261_093_911_248_7, 1e-12),
        error("HYPGEOM.DIST(-1,4,8,20,TRUE)", ExcelError::Number),
        error("HYPGEOM.DIST(5,4,8,20,TRUE)", ExcelError::Number),
        error("HYPGEOM.DIST(1,4,8,0,TRUE)", ExcelError::Number),
        error("HYPGEOMDIST(-1,4,8,20)", ExcelError::Number),
        error("HYPGEOMDIST(1,21,8,20)", ExcelError::Number),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn corpus_driven_functions_match_excel_scalar_contracts() {
    let cases = [
        text("ADDRESS(2,3)", "$C$2"),
        text("ADDRESS(2,3,2)", "C$2"),
        text("ADDRESS(2,3,3,TRUE,\"[Book1]Sheet1\")", "[Book1]Sheet1!$C2"),
        text("ADDRESS(2,3,4,FALSE,\"My Sheet\")", "'My Sheet'!R[2]C[3]"),
        text("CHAR(65)", "A"),
        text("CHAR(128)", "€"),
        number("COLUMN()", 7.0, 0.0),
        number("COLUMN(C7)", 3.0, 0.0),
        number("COLUMN(2:3)", 1.0, 0.0),
        number("COLUMN(B7:B11)", 2.0, 0.0),
        number("COLUMN(B11:D11)", 2.0, 0.0),
        number("COLUMN(E15:G19)", 5.0, 0.0),
        text("DOLLAR(1234.567,2)", "$1,234.57"),
        text("DOLLAR(-1234.5,0)", "($1,235)"),
        text("DOLLAR(1234.567,-2)", "$1,200"),
        text("DOLLAR(M1)", "$1,234.57"),
        text("HYPERLINK(\"https://example.com\")", "https://example.com"),
        text("HYPERLINK(\"https://example.com\",\"Example\")", "Example"),
        text("HYPERLINK(123)", "123"),
        text("LOOKUP(4,{1,2,4,8},{\"a\",\"b\",\"d\",\"h\"})", "d"),
        text("LOOKUP(5,{1,2,4,8},{\"a\",\"b\",\"d\",\"h\"})", "d"),
        text("LOOKUP(4,{8,1,4,2},{\"h\",\"a\",\"d\",\"b\"})", "d"),
        number("LOOKUP(5,{1,2,4,8})", 4.0, 0.0),
        number("LOOKUP(2,{1,10;2,20;3,30})", 20.0, 0.0),
        number("VALUE(\"0.33\")", 0.33, 0.0),
        number("VALUE(\"$1,000\")", 1_000.0, 0.0),
        number("VALUE(\"12.5%\")", 0.125, 0.0),
        number("VALUE(\"(₩1,234)\")", -1_234.0, 0.0),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

#[test]
fn corpus_driven_functions_reject_invalid_boundaries() {
    let cases = [
        error("ADDRESS(0,1)", ExcelError::Value),
        error("ADDRESS(1,16385)", ExcelError::Value),
        error("ADDRESS(1,1,5)", ExcelError::Value),
        error("CHAR(0)", ExcelError::Value),
        error("CHAR(129)", ExcelError::Value),
        error("DOLLAR(\"not numeric\")", ExcelError::Value),
        error("HYPERLINK()", ExcelError::Value),
        error("LOOKUP(0,{1,2,4},{10,20,40})", ExcelError::NotAvailable),
        error(
            "LOOKUP(1,{1,2;3,4},{10,20;30,40})",
            ExcelError::NotAvailable,
        ),
        error("VALUE(\"1,00\")", ExcelError::Value),
        error("VALUE(\"NaN\")", ExcelError::Value),
    ];
    let workbook = workbook_with_formula_cases(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (offset, case) in cases.iter().enumerate() {
        assert_expected(&calculation, offset as u32 + 1, case);
    }
}

const fn number(formula: &'static str, value: f64, tolerance: f64) -> FormulaCase {
    FormulaCase {
        formula,
        expected: Expected::Number { value, tolerance },
    }
}

const fn error(formula: &'static str, expected: ExcelError) -> FormulaCase {
    FormulaCase {
        formula,
        expected: Expected::Error(expected),
    }
}

const fn text(formula: &'static str, expected: &'static str) -> FormulaCase {
    FormulaCase {
        formula,
        expected: Expected::Text(expected),
    }
}

const fn logical(formula: &'static str, expected: bool) -> FormulaCase {
    FormulaCase {
        formula,
        expected: Expected::Logical(expected),
    }
}

fn workbook_with_formula_cases(cases: &[FormulaCase]) -> WorkbookSnapshot {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (offset, case) in cases.iter().enumerate() {
        insert_formula(&mut sheet, 1, offset as u32 + 1, case.formula);
    }
    workbook_with_sheet(sheet)
}

fn workbook_with_sheet(sheet: Sheet) -> WorkbookSnapshot {
    WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("function-coverage-test", "1").expect("valid provider identity"),
            None,
        ),
    )
    .expect("valid workbook")
}

fn insert_formula(sheet: &mut Sheet, row: u32, column: u32, formula: &str) {
    let address = CellAddress::from_indices(row, column).expect("valid formula address");
    sheet
        .insert_cell(
            address,
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx(formula).expect("valid formula text"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
        )
        .expect("unique formula address");
}

fn insert_literal(sheet: &mut Sheet, row: u32, column: u32, value: CellValue) {
    sheet
        .insert_cell(
            CellAddress::from_indices(row, column).expect("valid literal address"),
            CellContent::Literal(value),
        )
        .expect("unique literal address");
}

fn assert_expected(calculation: &cellrune::CalculationSnapshot, column: u32, case: &FormulaCase) {
    match case.expected {
        Expected::Number { value, tolerance } => {
            assert_cell_number(calculation, 1, column, value, tolerance);
        }
        Expected::Text(text) => assert_cell_text(calculation, 1, column, text),
        Expected::Logical(value) => assert_cell_logical(calculation, 1, column, value),
        Expected::Error(error) => assert_cell_error(calculation, 1, column, error),
    }
}

fn assert_cell_number(
    calculation: &cellrune::CalculationSnapshot,
    row: u32,
    column: u32,
    expected: f64,
    tolerance: f64,
) {
    let cell = calculation_cell_id(row, column);
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) = calculation.cell(cell)
    else {
        panic!(
            "expected numeric result at {row},{column}, got {:?}",
            calculation.cell(cell)
        );
    };
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected result at {row},{column}: formula expected {expected}, got {}",
        actual.get()
    );
}

fn assert_cell_error(
    calculation: &cellrune::CalculationSnapshot,
    row: u32,
    column: u32,
    expected: ExcelError,
) {
    let cell = calculation_cell_id(row, column);
    assert_eq!(
        calculation.cell(cell),
        Some(&CalculationCellResult::Value(CellValue::Error(expected))),
        "unexpected error result at {row},{column}"
    );
}

fn assert_cell_text(
    calculation: &cellrune::CalculationSnapshot,
    row: u32,
    column: u32,
    expected: &str,
) {
    let cell = calculation_cell_id(row, column);
    assert_eq!(
        calculation.cell(cell),
        Some(&CalculationCellResult::Value(CellValue::Text(
            expected.to_owned()
        ))),
        "unexpected text result at {row},{column}"
    );
}

fn assert_cell_logical(
    calculation: &cellrune::CalculationSnapshot,
    row: u32,
    column: u32,
    expected: bool,
) {
    let cell = calculation_cell_id(row, column);
    assert_eq!(
        calculation.cell(cell),
        Some(&CalculationCellResult::Value(CellValue::Logical(expected))),
        "unexpected logical result at {row},{column}"
    );
}

fn calculation_cell_id(row: u32, column: u32) -> CalculationCellId {
    CalculationCellId::new(
        SheetId::new(1).expect("valid sheet ID"),
        CellAddress::from_indices(row, column).expect("valid cell address"),
    )
}
