use super::super::ast::Expr;
use super::super::coerce::compare_text_case_insensitive;
use super::super::eval::{Engine, EvalContext};
use super::super::formula_rebase::FormulaCriteria;
use super::super::runtime::Rect;
use super::super::value::{ErrorKind, Value};
use super::criteria_runtime::CriteriaRuntime;
use super::database_criteria::CompiledDatabaseCriteria;
use super::kernel::DatabaseFunction;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: DatabaseFunction,
    args: &[Expr],
) -> Value {
    let [database_expr, field_expr, criteria_expr] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let database = match resolve_database(engine, context, database_expr) {
        Ok(database) => database,
        Err(kind) => return Value::Error(kind),
    };
    let criteria_rect = match resolve_criteria_range(engine, context, criteria_expr) {
        Ok(criteria) => criteria,
        Err(kind) => return Value::Error(kind),
    };
    let mut runtime = CriteriaRuntime::new(engine, context);
    let field = match resolve_field(
        engine,
        context,
        &mut runtime,
        &database,
        field_expr,
        matches!(function, DatabaseFunction::Count | DatabaseFunction::CountA),
    ) {
        Ok(field) => field,
        Err(kind) => return Value::Error(kind),
    };
    let criteria =
        match compile_criteria_table(engine, context, &mut runtime, &database, criteria_rect) {
            Ok(criteria) => criteria,
            Err(kind) => return Value::Error(kind),
        };
    let selected = match select_records(engine, context, &mut runtime, &database, &criteria) {
        Ok(selected) => selected,
        Err(kind) => return Value::Error(kind),
    };
    aggregate_selected(
        engine,
        context,
        &mut runtime,
        function,
        database.rect.sheet,
        field,
        &selected,
    )
}

#[derive(Debug, Clone)]
struct Database {
    rect: Rect,
    headers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseField {
    Omitted,
    Column(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderMatch {
    Missing,
    Unique(u32),
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CriteriaHeader {
    Field(u32),
    Formula,
}

#[derive(Debug, Clone)]
enum CriteriaCondition {
    Field {
        column: u32,
        criterion: CompiledDatabaseCriteria,
    },
    Formula(FormulaCriteria),
}

#[derive(Debug, Clone, Default)]
struct CriteriaRow {
    conditions: Vec<CriteriaCondition>,
}

#[derive(Debug, Clone, Default)]
struct SelectedRecords {
    rows: Vec<u32>,
}

impl SelectedRecords {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn as_slice(&self) -> &[u32] {
        &self.rows
    }

    fn iter(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.rows.iter().copied()
    }
}

fn resolve_database(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<Database, ErrorKind> {
    let rect = engine.resolve_rect_expr(context, expr)?;
    if rect.height() < 2 {
        return Err(ErrorKind::Value);
    }
    engine.ensure_array_cells(rect.width())?;
    let mut headers = Vec::with_capacity(rect.width() as usize);
    for column in rect.col_start..=rect.col_end {
        let value = engine.read_reference_cell(context, (rect.sheet, rect.row_start, column))?;
        let Value::Text(header) = value else {
            return Err(ErrorKind::Value);
        };
        if header.is_empty() {
            return Err(ErrorKind::Value);
        }
        headers.push(header);
    }
    Ok(Database { rect, headers })
}

fn resolve_criteria_range(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<Rect, ErrorKind> {
    let rect = engine.resolve_rect_expr(context, expr)?;
    if rect.height() < 2 {
        return Err(ErrorKind::Value);
    }
    let cells = rect.height() * rect.width();
    engine.ensure_array_cells(cells)?;
    Ok(rect)
}

fn resolve_field(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
    database: &Database,
    expr: &Expr,
    omission_allowed: bool,
) -> Result<DatabaseField, ErrorKind> {
    if matches!(expr, Expr::Missing) {
        return if omission_allowed {
            Ok(DatabaseField::Omitted)
        } else {
            Err(ErrorKind::Value)
        };
    }
    match engine.eval_scalar(context, expr) {
        Value::Number(index)
            if index.is_finite()
                && index.fract() == 0.0
                && index >= 1.0
                && index <= database.rect.width() as f64 =>
        {
            Ok(DatabaseField::Column(
                database.rect.col_start + index as u32 - 1,
            ))
        }
        Value::Text(name) if !name.is_empty() => {
            match find_header_column(runtime, database, &name)? {
                HeaderMatch::Unique(column) => Ok(DatabaseField::Column(column)),
                HeaderMatch::Missing | HeaderMatch::Ambiguous => Err(ErrorKind::Value),
            }
        }
        Value::Error(kind) if kind.is_engine_issue() => Err(kind),
        Value::Blank | Value::Number(_) | Value::Text(_) | Value::Logical(_) | Value::Error(_) => {
            Err(ErrorKind::Value)
        }
    }
}

fn find_header_column(
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
    database: &Database,
    name: &str,
) -> Result<HeaderMatch, ErrorKind> {
    let mut found = None;
    for (offset, header) in database.headers.iter().enumerate() {
        let work = u64::try_from(header.len())
            .ok()
            .and_then(|length| length.checked_add(name.len() as u64))
            .ok_or(ErrorKind::Value)?;
        runtime.charge_work(work)?;
        if compare_text_case_insensitive(header, name).is_eq() {
            if found.is_some() {
                return Ok(HeaderMatch::Ambiguous);
            }
            found = Some(database.rect.col_start + offset as u32);
        }
    }
    Ok(found.map_or(HeaderMatch::Missing, HeaderMatch::Unique))
}

fn compile_criteria_table(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
    database: &Database,
    criteria: Rect,
) -> Result<Vec<CriteriaRow>, ErrorKind> {
    let mut headers = Vec::with_capacity(criteria.width() as usize);
    for column in criteria.col_start..=criteria.col_end {
        let value =
            engine.read_reference_cell(context, (criteria.sheet, criteria.row_start, column))?;
        let header = match value {
            Value::Text(name) if !name.is_empty() => {
                match find_header_column(runtime, database, &name)? {
                    HeaderMatch::Unique(column) => CriteriaHeader::Field(column),
                    HeaderMatch::Missing => CriteriaHeader::Formula,
                    HeaderMatch::Ambiguous => return Err(ErrorKind::Value),
                }
            }
            Value::Error(kind) if kind.is_engine_issue() => return Err(kind),
            Value::Blank
            | Value::Number(_)
            | Value::Text(_)
            | Value::Logical(_)
            | Value::Error(_) => CriteriaHeader::Formula,
        };
        headers.push(header);
    }

    let mut rows = Vec::with_capacity(criteria.height().saturating_sub(1) as usize);
    for row in criteria.row_start + 1..=criteria.row_end {
        let mut compiled = CriteriaRow::default();
        for (offset, header) in headers.iter().enumerate() {
            let cell = (criteria.sheet, row, criteria.col_start + offset as u32);
            let value = engine.read_reference_cell(context, cell)?;
            let condition = match header {
                CriteriaHeader::Field(column) => {
                    if value.is_blank_like() {
                        continue;
                    }
                    CriteriaCondition::Field {
                        column: *column,
                        criterion: runtime.compile_database_criteria(&value)?,
                    }
                }
                CriteriaHeader::Formula => {
                    if !engine.cell_has_formula(cell) {
                        if value.is_blank_like() {
                            continue;
                        }
                        return Err(match value {
                            Value::Error(kind) if kind.is_engine_issue() => kind,
                            _ => ErrorKind::Value,
                        });
                    }
                    if let Value::Error(kind) = value
                        && kind.is_engine_issue()
                    {
                        return Err(kind);
                    }
                    let root = engine.parsed_expr(cell).ok_or(ErrorKind::Value)?;
                    CriteriaCondition::Formula(FormulaCriteria::prepare(
                        engine,
                        context,
                        cell,
                        root,
                        database.rect,
                    )?)
                }
            };
            compiled.conditions.push(condition);
        }
        rows.push(compiled);
    }
    Ok(rows)
}

fn select_records(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
    database: &Database,
    criteria: &[CriteriaRow],
) -> Result<SelectedRecords, ErrorKind> {
    let record_count = database.rect.height() - 1;
    engine.ensure_array_cells(record_count)?;
    let mut selected = Vec::with_capacity(record_count as usize);
    for row in database.rect.row_start + 1..=database.rect.row_end {
        runtime.charge_work(1)?;
        let row_delta = row - database.rect.row_start - 1;
        let mut matches_any = false;
        for criteria_row in criteria {
            let mut matches_all = true;
            for condition in &criteria_row.conditions {
                let matched = match condition {
                    CriteriaCondition::Field { column, criterion } => {
                        let value = engine
                            .read_reference_cell(context, (database.rect.sheet, row, *column))?;
                        runtime.matches_database(criterion, &value)?
                    }
                    CriteriaCondition::Formula(formula) => {
                        formula.evaluate(engine, context, row_delta)?
                    }
                };
                if !matched {
                    matches_all = false;
                    break;
                }
            }
            if matches_all {
                matches_any = true;
                break;
            }
        }
        if matches_any {
            selected.push(row);
        }
    }
    Ok(SelectedRecords { rows: selected })
}

fn aggregate_selected(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
    function: DatabaseFunction,
    database_sheet: usize,
    field: DatabaseField,
    selected: &SelectedRecords,
) -> Value {
    let DatabaseField::Column(column) = field else {
        return Value::Number(selected.len() as f64);
    };
    if function == DatabaseFunction::Get {
        return match selected.as_slice() {
            [] => Value::Error(ErrorKind::Value),
            [row] => engine
                .read_reference_cell(context, (database_sheet, *row, column))
                .unwrap_or_else(Value::Error),
            [_, _, ..] => Value::Error(ErrorKind::Num),
        };
    }
    let mut numbers = Vec::with_capacity(selected.len());
    let mut nonblank_count = 0_u64;
    for row in selected.iter() {
        if let Err(kind) = runtime.charge_work(1) {
            return Value::Error(kind);
        }
        let value = match engine.read_reference_cell(context, (database_sheet, row, column)) {
            Ok(value) => value,
            Err(kind) => return Value::Error(kind),
        };
        match value {
            Value::Number(number) => {
                numbers.push(number);
                nonblank_count += 1;
            }
            Value::Blank => {}
            Value::Text(_) | Value::Logical(_) => {
                if function == DatabaseFunction::CountA {
                    nonblank_count += 1;
                }
            }
            Value::Error(kind) if kind.is_engine_issue() => return Value::Error(kind),
            Value::Error(kind) => match function {
                DatabaseFunction::Count => {}
                DatabaseFunction::CountA => nonblank_count += 1,
                DatabaseFunction::Average
                | DatabaseFunction::Get
                | DatabaseFunction::Max
                | DatabaseFunction::Min
                | DatabaseFunction::Product
                | DatabaseFunction::StDev
                | DatabaseFunction::StDevP
                | DatabaseFunction::Sum
                | DatabaseFunction::Var
                | DatabaseFunction::VarP => return Value::Error(kind),
            },
        }
    }
    let result = match function {
        DatabaseFunction::Count => return Value::Number(numbers.len() as f64),
        DatabaseFunction::CountA => return Value::Number(nonblank_count as f64),
        DatabaseFunction::Sum => numbers.iter().sum(),
        DatabaseFunction::Average if numbers.is_empty() => return Value::Error(ErrorKind::Div0),
        DatabaseFunction::Average => numbers.iter().sum::<f64>() / numbers.len() as f64,
        DatabaseFunction::Max => numbers.into_iter().reduce(f64::max).unwrap_or(0.0),
        DatabaseFunction::Min => numbers.into_iter().reduce(f64::min).unwrap_or(0.0),
        DatabaseFunction::Product => numbers.into_iter().reduce(|a, b| a * b).unwrap_or(0.0),
        DatabaseFunction::StDev | DatabaseFunction::Var if numbers.len() < 2 => {
            return Value::Error(ErrorKind::Div0);
        }
        DatabaseFunction::StDevP | DatabaseFunction::VarP if numbers.is_empty() => {
            return Value::Error(ErrorKind::Div0);
        }
        DatabaseFunction::StDev
        | DatabaseFunction::StDevP
        | DatabaseFunction::Var
        | DatabaseFunction::VarP => {
            let mean = numbers.iter().sum::<f64>() / numbers.len() as f64;
            let divisor = if matches!(function, DatabaseFunction::StDev | DatabaseFunction::Var) {
                numbers.len() - 1
            } else {
                numbers.len()
            };
            let variance = numbers
                .iter()
                .map(|number| (number - mean) * (number - mean))
                .sum::<f64>()
                / divisor as f64;
            if matches!(function, DatabaseFunction::StDev | DatabaseFunction::StDevP) {
                variance.sqrt()
            } else {
                variance
            }
        }
        DatabaseFunction::Get => {
            unreachable!("database special cases returned before numeric aggregation")
        }
    };
    if result.is_finite() {
        Value::Number(result)
    } else {
        Value::Error(ErrorKind::Num)
    }
}
