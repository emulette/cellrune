"use strict";
const assert = require("node:assert/strict");
const { Workbook, CellRuneError } = require("..");

async function main() {
  const workbook = Workbook.create();
  workbook.setNumber("Sheet1", "A1", 2);
  workbook.setFormula("Sheet1", "B1", "=A1+1");
  workbook.setFormula("Sheet1", "C1", "=B1*2");
  workbook.setFormula("Sheet1", "Z1", "=UNKNOWN_FUNCTION(1)");
  const revision = workbook.summary().semanticRevision;
  const targets = [{ sheet: "Sheet1", start: "B1", end: "D1" }, { sheet: "Sheet1", start: "C1" }];
  const result = await workbook.calculateTargets(targets);
  assert.equal(result.scope, "targets");
  assert.equal(result.semanticRevision, revision);
  assert.equal(result.evaluatedCount, 2);
  assert.equal(result.parsedFormulaCount, 2);
  assert.deepEqual(result.cells.map(item => item.result), [
    { kind: "value", value: { kind: "number", value: 3 } },
    { kind: "value", value: { kind: "number", value: 6 } },
    { kind: "value", value: { kind: "blank" } },
  ]);
  assert.equal(result.sourceFingerprint.digestHex.length, 64);
  assert.equal(workbook.readRange("Sheet1", "C1", "C1").cells[0].calculated, null);
  assert.equal(workbook.changesSince().deltas.length, 0);
  await assert.rejects(workbook.toBytes(), error => error.code === "interop.calculation.required");
  await workbook.recalculate();
  const cached = await workbook.calculateTargets(targets);
  assert.equal(cached.reusedCount, 2);
  assert.equal(cached.evaluatedCount, 0);
  workbook.setNumber("Sheet1", "A1", 5);
  const edited = await workbook.calculateTargets([{ sheet: "Sheet1", start: "C1" }]);
  assert.equal(edited.cells[0].result.value.value, 12);
  assert.equal(edited.reusedCount, 0);
  for (const [input, options] of [[[], {}], [[{sheet:"Sheet1",start:"A1",end:"XFD1048576"}], {}],
    [targets, {limits:{maxResultCells:1}}], [targets, {todaySerial:NaN}], [targets, {limits:{maxTargets:true}}]]) {
    await assert.rejects(workbook.calculateTargets(input, options), CellRuneError);
  }
  // A created workbook holds no saved result, so asking for them changes nothing.
  const saved = await workbook.calculateTargets(targets, { uncalculablePrecedents: "saved_result" });
  assert.deepEqual(saved.cells, (await workbook.calculateTargets(targets)).cells);
  assert.ok(saved.cells.every(item => !("savedPrecedents" in item)));
  await assert.rejects(
    workbook.calculateTargets(targets, { uncalculablePrecedents: "always" }),
    error => error instanceof CellRuneError && error.code === "interop.calculation.target.invalid_payload",
  );
  workbook.close();
  await assert.rejects(workbook.calculateTargets(targets), error => error.code === "interop.session.closed");

  // A minimal save of a workbook read from bytes takes the saved-result mark option.
  const source = Workbook.create();
  source.setNumber("Sheet1", "A1", 2);
  await source.recalculate();
  const opened = await Workbook.fromBytes(await source.toBytes());
  opened.setFormula("Sheet1", "B1", "=A1+1");
  const marked = await opened.editsToBytes({ savedResultMark: true });
  const unmarked = await opened.editsToBytes();
  assert.ok(!marked.equals(unmarked), "the mark changes the workbook part");
  assert.ok((await (await Workbook.fromBytes(marked)).summary()).sheets.length === 1);
  await assert.rejects(opened.editsToBytes({ savedResultMark: 1 }), CellRuneError);
  await assert.rejects(opened.editsToBytes({ mark: true }), CellRuneError);
  opened.close();
  source.close();
}

main().catch(error => { process.stderr.write(`${error.stack ?? error}\n`); process.exitCode = 1; });
