"use strict";

const assert = require("node:assert/strict");
const corpus = require("../../../binding-contract/function-usage-v2.json");
const { Workbook } = require("..");

module.exports = function assertFunctionUsage() {
  const workbook = Workbook.create();
  try {
    workbook.applyChanges(0n, Array.from({ length: corpus.max_depth + 1 }, (_, depth) => ({
      kind: "setDefinedName",
      name: `Usage_${depth}`,
      formula: depth === 0 ? "=SUM(1)" : `=Usage_${depth - 1}+Usage_${depth - 1}`,
      hidden: false,
    })));
    for (const item of corpus.cases) {
      workbook.setFormula("Sheet1", "A1", item.formula);
      const summary = workbook.summary();
      const history = workbook.changesSince(0n);
      const usage = workbook.functionUsage();
      assert.equal(usage.schemaVersion, corpus.schema_version);
      assert.equal(usage.formulaCount, 1);
      assert.equal(usage.parsedFormulaCount, 1);
      assert.equal(usage.unparsedFormulaCount, 0);
      assert.deepEqual(usage.entries, [{
        name: "SUM", supported: true, callCount: BigInt(item.call_count), formulaCount: 1,
        sampleCells: [{ sheetId: 1, sheetName: "Sheet1", address: "A1" }],
      }]);
      assert.deepEqual(workbook.functionUsage(), usage);
      assert.deepEqual(workbook.summary(), summary);
      assert.deepEqual(workbook.changesSince(0n), history);
    }
  } finally {
    workbook.close();
  }
};
