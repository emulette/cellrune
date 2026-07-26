"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { CellRuneError, Workbook } = require("..");

async function main() {
  const corpusPath = path.join(__dirname, "..", "..", "..", "binding-contract", "v1.json");
  const corpus = JSON.parse(fs.readFileSync(corpusPath, "utf8"));
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

  workbook.setFormula("Sheet1", "A3", "=100.1-100-0.1");
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

  const bytes = await workbook.toBytes();
  const reopened = await Workbook.fromBytes(bytes);
  assert.equal(reopened.summary().documentKind, "xlsx");
  assert.throws(
    () => reopened.setNumber("Sheet1", corpus.invalid_address, 1),
    (error) =>
      error instanceof CellRuneError &&
      error.code === corpus.invalid_address_code &&
      error.kind === "validation",
  );
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
