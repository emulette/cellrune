"use strict";

const assert = require("node:assert/strict");
const { createHash } = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { CellRuneError, Workbook, functionCatalog } = require("..");

const CATALOG_V0_1_10_REFERENCE_SHA256 =
  "d7b2743f3f9d612cafb8d4fa9797008f11001649726499efdde2d29b86e534ee";

function catalogDigest() {
  const catalog = functionCatalog();
  assert.equal(catalog.schemaVersion, 1);
  assert.equal(catalog.entries.length, 305);
  const digest = createHash("sha256");
  for (const entry of catalog.entries) {
    digest.update(
      [
        entry.name,
        entry.canonicalName,
        entry.alias ? "1" : "0",
        entry.returnsArray ? "1" : "0",
        entry.official ? "1" : "0",
      ].join("\0") + "\n",
    );
  }
  return digest.digest("hex");
}

async function main() {
  assert.equal(catalogDigest(), CATALOG_V0_1_10_REFERENCE_SHA256);
  const corpusPath = path.join(__dirname, "..", "..", "..", "binding-contract", "v1.json");
  const corpus = JSON.parse(fs.readFileSync(corpusPath, "utf8"));
  const definedNameCorpusPath = path.join(
    __dirname,
    "..",
    "..",
    "..",
    "binding-contract",
    "defined-name-v1.json",
  );
  const definedNameCorpus = JSON.parse(
    fs.readFileSync(definedNameCorpusPath, "utf8"),
  );
  const tableContractPath = path.join(
    __dirname,
    "..",
    "..",
    "..",
    "binding-contract",
    "table-authoring-v2.json",
  );
  const tableContract = JSON.parse(
    fs.readFileSync(tableContractPath, "utf8"),
  );
  assert.equal(definedNameCorpus.schema_version, 1);
  const workbook = Workbook.create();
  for (const operation of corpus.operations) {
    if (operation.kind === "set_number") {
      workbook.setNumber(operation.sheet, operation.address, operation.value);
    } else if (operation.kind === "set_formula") {
      workbook.setFormula(operation.sheet, operation.address, operation.formula);
    } else if (operation.kind === "set_dynamic_formula") {
      workbook.setFormula(operation.sheet, operation.address, operation.formula, {
        dynamicRange: operation.dynamic_range,
      });
    } else {
      assert.fail(`unknown corpus operation: ${operation.kind}`);
    }
  }
  const report = await workbook.calculate();
  assert.equal(report.unavailableCount, 0);
  const page = workbook.readRange("Sheet1", "A1", "F2", { limit: 100 });
  const values = new Map();
  for (const cell of page.cells) {
    const value =
      cell.calculated?.kind === "value"
        ? cell.calculated.value
        : cell.sourceValue;
    if (value.kind === "number") {
      values.set(cell.address, value.value);
    }
  }
  for (const expected of corpus.expected_numbers) {
    assert.equal(values.get(expected.address), expected.value);
  }

  workbook.setFormula("Sheet1", "A3", "=0.1+0.2-0.3");
  workbook.setFormula("Sheet1", "A4", "=IRR({-1,100000})");
  await workbook.recalculate({ mode: "full" });
  const defaults = workbook.readRange("Sheet1", "A3", "A4", { limit: 2 }).cells;
  assert.equal(defaults[0].calculated.value.value, 0);
  assert.equal(defaults[1].calculated.value.value, "#NUM!");
  await workbook.recalculate({
    mode: "full",
    arithmeticSemantics: "ieee_754",
    financialSolverSemantics: "extended_search",
  });
  const legacy = workbook.readRange("Sheet1", "A3", "A4", { limit: 2 }).cells;
  assert.notEqual(legacy[0].calculated.value.value, 0);
  assert.ok(Math.abs(legacy[1].calculated.value.value - 99999) < 1e-5);
  await assert.rejects(
    workbook.calculate({ arithmeticSemantics: "binary" }),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.calculation.arithmetic_semantics_invalid",
  );
  await assert.rejects(
    workbook.calculate({ financialSolverSemantics: "unbounded" }),
    (error) =>
      error instanceof CellRuneError &&
      error.code ===
        "interop.calculation.financial_solver_semantics_invalid",
  );

  for (const sheetName of definedNameCorpus.sheets) {
    workbook.addSheet(sheetName);
  }
  workbook.applyChanges(
    workbook.summary().semanticRevision,
    definedNameCorpus.defined_names.map((item) => ({
      kind: "setDefinedName",
      name: item.name,
      scopeSheet: item.scope_sheet,
      formula: item.formula,
      hidden: item.hidden,
    })),
  );
  assert.deepEqual(
    workbook.inspectDefinedName("WorkbookAlias", { currentSheet: "Sheet1" })
      .result,
    {
      kind: "rectangular",
      sheetId: 1,
      sheetName: "Sheet1",
      range: "A1:A1",
    },
  );
  assert.deepEqual(
    workbook.inspectDefinedName("LocalAlias", { currentSheet: "Sheet1" }).result,
    {
      kind: "rectangular",
      sheetId: 1,
      sheetName: "Sheet1",
      range: "B2:B2",
    },
  );
  assert.deepEqual(workbook.inspectDefinedName("QualifiedLocal").result, {
    kind: "rectangular",
    sheetId: 1,
    sheetName: "Sheet1",
    range: "B2:B2",
  });
  assert.deepEqual(workbook.inspectDefinedName("ExplicitSingleSpan").result, {
    kind: "threeDimensional",
    sheetSpan: {
      startSheetId: 2,
      startSheetName: "Middle",
      endSheetId: 2,
      endSheetName: "Middle",
    },
    range: "D4:D4",
  });
  assert.deepEqual(workbook.inspectDefinedName("Dynamic").result, {
    kind: "dynamicFormula",
    dynamicKind: "offset",
    formula: "=OFFSET(Sheet1!A1,1,0)",
  });
  assert.equal(
    workbook.inspectDefinedName("IndirectDynamic").result.dynamicKind,
    "indirect",
  );
  assert.equal(
    workbook.inspectDefinedName("SpillDynamic").result.dynamicKind,
    "spill",
  );
  assert.equal(
    workbook.inspectDefinedName("MixedDynamic").result.dynamicKind,
    "mixed",
  );
  const areas = workbook.inspectDefinedName("Areas").result;
  assert.equal(areas.kind, "nonRectangular");
  assert.deepEqual(
    areas.areas.map((area) => area.kind),
    ["rectangular", "rectangular", "threeDimensional", "rectangular"],
  );
  assert.deepEqual(areas.areas[2].sheetSpan, {
    startSheetId: 1,
    startSheetName: "Sheet1",
    endSheetId: 3,
    endSheetName: "Sheet3",
  });
  assert.equal(
    workbook.inspectDefinedName("ConstantValue").result.kind,
    "constant",
  );
  assert.deepEqual(workbook.inspectDefinedName("ExternalValue").result, {
    kind: "externalReference",
    locator: null,
    workbook: "Book.xlsx",
    sheet: "Data",
    sheetEnd: null,
    targetKind: "reference",
    targetText: "A1",
  });
  assert.equal(
    workbook.inspectDefinedName("InvalidValue").result.reason,
    "parse_error",
  );
  assert.equal(
    workbook.inspectDefinedName("CallableValue").result.reason,
    "non_reference_expression",
  );
  assert.deepEqual(workbook.inspectDefinedName("Missing").result, {
    kind: "notFound",
  });
  assert.throws(
    () =>
      workbook.inspectDefinedName("Areas", { currentSheet: "missing" }),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.sheet.not_found",
  );
  assert.throws(
    () => workbook.inspectDefinedName("Areas", { current_sheet: "Sheet1" }),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.input.invalid" &&
      error.details.detail === "options.current_sheet is not supported",
  );

  await workbook.recalculate({ mode: "full" });
  const bytes = await workbook.toBytes();
  const reopened = await Workbook.fromBytes(bytes);
  assert.equal(reopened.summary().documentKind, "xlsx");
  assert.equal(
    reopened.inspectDefinedName("Dynamic").result.kind,
    "dynamicFormula",
  );
  assert.deepEqual(
    reopened.inspectDefinedName("ExplicitSingleSpan").result,
    workbook.inspectDefinedName("ExplicitSingleSpan").result,
  );
  assert.deepEqual(
    reopened.inspectDefinedName("ExternalValue").result,
    workbook.inspectDefinedName("ExternalValue").result,
  );
  assert.throws(
    () => reopened.setNumber("Sheet1", corpus.invalid_address, 1),
    (error) =>
      error instanceof CellRuneError &&
      error.code === corpus.invalid_address_code &&
      error.kind === "validation",
  );

  const tableWorkbook = await Workbook.openPath(
    path.join(path.dirname(tableContractPath), tableContract.fixture),
  );
  assert.throws(
    () =>
      tableWorkbook.applyChanges(tableWorkbook.summary().semanticRevision, [
        {
          kind: "renameTable",
          tableId: tableContract.table_id,
          newDisplayName: tableContract.new_display_name,
        },
      ]),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.input.invalid",
  );
  assert.throws(
    () =>
      tableWorkbook.applyChangesV2(
        tableWorkbook.summary().semanticRevision,
        [
          {
            kind: "renameTable",
            tableId: tableContract.table_id,
            newDisplayName: tableContract.new_display_name,
            unexpected: true,
          },
        ],
      ),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.input.invalid" &&
      error.details.detail === "changes[0].unexpected is not supported",
  );
  assert.throws(
    () =>
      tableWorkbook.applyChangesV2(
        tableWorkbook.summary().semanticRevision,
        [
          {
            kind: "setValue",
            sheet: "Data",
            address: "F1",
            value: { kind: "number", value: 1, unexpected: true },
          },
        ],
      ),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.input.invalid" &&
      error.details.detail ===
        "changes[0].value.unexpected is not supported",
  );
  const tableReceipt = tableWorkbook.applyChangesV2(
    tableWorkbook.summary().semanticRevision,
    [
      {
        kind: "renameTable",
        tableId: tableContract.table_id,
        newDisplayName: tableContract.new_display_name,
      },
      {
        kind: "renameTableColumn",
        tableId: tableContract.table_id,
        columnId: tableContract.column_id,
        newName: tableContract.new_column_name,
      },
      {
        kind: "resizeTableRows",
        tableId: tableContract.table_id,
        firstDataRow: tableContract.first_data_row,
        lastDataRow: tableContract.last_data_row,
      },
    ],
  );
  assert.equal(tableReceipt.schemaVersion, tableContract.schema_version);
  assert.deepEqual(tableReceipt.changedTableIds, [tableContract.table_id]);
  assertTableAuthoringResult(tableWorkbook, tableContract);
  await tableWorkbook.recalculate({ mode: "full" });
  const reopenedTableWorkbook = await Workbook.fromBytes(
    await tableWorkbook.toBytes({ invalidateUnavailable: true }),
  );
  assertTableAuthoringResult(reopenedTableWorkbook, tableContract);
}

function assertTableAuthoringResult(workbook, contract) {
  const tables = workbook.summary().sheets[0].tables;
  const table = tables.find((candidate) => candidate.id === contract.table_id);
  assert.ok(table);
  assert.equal(table.id, contract.table_id);
  assert.equal(table.name, contract.new_display_name);
  assert.equal(table.displayName, contract.new_display_name);
  assert.equal(table.range, contract.expected_range);
  assert.equal(table.columns[1].id, contract.column_id);
  assert.equal(table.columns[1].name, contract.new_column_name);
  const header = workbook.readRange(
    "Data",
    contract.expected_header_address,
    contract.expected_header_address,
  ).cells[0];
  assert.equal(header.sourceValue.kind, "text");
  assert.equal(header.sourceValue.value, contract.new_column_name);
  const formula = workbook.readRange("Data", "E1", "E1").cells[0].formula;
  assert.equal(formula, "=SUM(Orders[Gross Amount])");
  const emptyTable = tables.find(
    (candidate) => candidate.id === contract.empty_table_id,
  );
  assert.ok(emptyTable);
  assert.equal(emptyTable.name, contract.empty_table_name);
  assert.equal(emptyTable.range, contract.empty_table_range);
  assert.equal(
    workbook.inspectDefinedName(contract.empty_defined_name).result.kind,
    "emptyReference",
  );
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
