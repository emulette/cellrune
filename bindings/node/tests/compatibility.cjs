"use strict";

const assert = require("node:assert/strict");
const corpus = require("../../../binding-contract/compatibility-v021.json");
const { Workbook } = require("..");

module.exports = async function assertCompatibility() {
  const workbook = Workbook.create();
  try {
    workbook.setNumber("Sheet1", "A1", corpus.initial_input);
    for (const item of corpus.cases) workbook.setFormula("Sheet1", item.address, item.formula);
    const targets = await workbook.calculateTargets([{ sheet: "Sheet1", start: "B1", end: "B9" }]);
    assert.equal(targets.evaluatedCount, corpus.cases.length);
    assert.deepEqual(targets.cells.map(cell => cell.result),
      corpus.cases.map(item => ({ kind: "value", value: item.initial })));
    await workbook.recalculate({ mode: "full" });
    const initial = workbook.readRange("Sheet1", "B1", "B9");
    assert.deepEqual(initial.cells.map(cell => cell.calculated),
      corpus.cases.map(item => ({ kind: "value", value: item.initial })));
    const summary = workbook.summary();
    const history = workbook.changesSince();
    const preview = await workbook.previewChanges(summary.semanticRevision, [{
      kind: "setValue", sheet: "Sheet1", address: "A1", value: { kind: "number", value: corpus.edited_input },
    }]);
    const details = workbook.previewChangesPage(preview.previewId, { section: "preview_results" });
    for (const item of corpus.cases) {
      if (item.initial.kind !== item.edited.kind || item.initial.value !== item.edited.value) {
        assert.deepEqual(details.items.find(result => result.cell.address === item.address).result,
          { kind: "value", value: item.edited });
      }
    }
    assert.deepEqual(workbook.summary(), summary);
    assert.deepEqual(workbook.changesSince(), history);
    assert.deepEqual(workbook.readRange("Sheet1", "B1", "B9"), initial);
    workbook.discardPreview(preview.previewId);
    workbook.setNumber("Sheet1", "A1", corpus.edited_input);
    await workbook.recalculate({ mode: "auto" });
    assert.deepEqual(workbook.readRange("Sheet1", "B1", "B9").cells.map(cell => cell.calculated),
      corpus.cases.map(item => ({ kind: "value", value: item.edited })));
    const reopened = await Workbook.fromBytes(await workbook.toBytes());
    try {
      assert.deepEqual(reopened.readRange("Sheet1", "B1", "B9").cells.map(cell => cell.sourceValue),
        corpus.cases.map(item => item.edited));
    } finally {
      reopened.close();
    }
  } finally {
    workbook.close();
  }
};
