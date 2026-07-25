"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { CellRuneError, Workbook } = require("..");

async function main() {
  const corpusPath = path.join(
    __dirname,
    "..",
    "..",
    "..",
    "conformance",
    "interactive-v1.json",
  );
  const corpus = JSON.parse(fs.readFileSync(corpusPath, "utf8"));
  const workbook = Workbook.create();
  const initial = workbook.applyChanges(
    0n,
    corpus.initial_changes.map(fromCorpusChange),
  );
  assert.equal(initial.resultRevision, BigInt(corpus.expected.initial_revision));
  assert.equal(initial.appliedChangeCount, corpus.initial_changes.length);
  assert.equal(
    initial.calculationChangedCells.length,
    corpus.initial_changes.length,
  );
  assert.equal(initial.calculationMetadataChanged, false);

  const first = await workbook.recalculate();
  assert.equal(first.mode, "full");
  assert.equal(first.resultRevision, initial.resultRevision);

  const receipt = workbook.applyChanges(
    initial.resultRevision,
    corpus.incremental_changes.map(fromCorpusChange),
  );
  assert.equal(receipt.calculationChangedCells.length, 1);
  const delta = await workbook.recalculate();
  assert.equal(delta.mode, corpus.expected.incremental_mode);
  assert.equal(
    delta.evaluatedCount,
    corpus.expected.incremental_evaluated_count,
  );
  assert.equal(delta.parsedFormulaCount, 0);
  assert.equal(delta.resultRevision, receipt.resultRevision);

  const page = workbook.readRange("Sheet1", "B1", "C1");
  assert.equal(page.cells[0].calculated.value.value, corpus.expected.b1);
  assert.equal(page.cells[1].calculated.value.value, corpus.expected.c1);

  const history = workbook.changesSince(0n, { limit: 1 });
  assert.equal(history.deltas.length, 1);
  assert.notEqual(history.nextCursor, null);
  const next = workbook.changesSince(history.nextCursor, { limit: 1 });
  assert.equal(next.deltas.length, 1);
  assert.equal(next.nextCursor, null);

  assert.throws(
    () =>
      workbook.applyChanges(0n, [
        {
          kind: "setValue",
          sheet: "Sheet1",
          address: "A1",
          value: { kind: "number", value: 99 },
        },
      ]),
    (error) =>
      error instanceof CellRuneError &&
      error.code === corpus.expected.revision_error,
  );

  await assertConcurrentEditAndCancellation();
}

async function assertConcurrentEditAndCancellation() {
  const workbook = Workbook.create();
  const formulaChanges = [
    {
      kind: "setValue",
      sheet: "Sheet1",
      address: "A1",
      value: { kind: "number", value: 1 },
    },
  ];
  for (let row = 1; row <= 30_000; row += 1) {
    formulaChanges.push({
      kind: "setFormula",
      sheet: "Sheet1",
      address: `B${row}`,
      formula: `=A1+${row}`,
    });
  }
  const receipt = workbook.applyChanges(0n, formulaChanges);

  const staleTask = workbook.recalculate({ mode: "full" });
  await waitUntilActive(workbook, staleTask);
  workbook.applyChanges(receipt.resultRevision, [
    {
      kind: "setValue",
      sheet: "Sheet1",
      address: "A1",
      value: { kind: "number", value: 2 },
    },
  ]);
  await assert.rejects(
    staleTask,
    (error) =>
      error instanceof CellRuneError && error.code === "session.stale_result",
  );

  const cancelledTask = workbook.recalculate({ mode: "full" });
  await waitUntilActive(workbook, cancelledTask);
  assert.equal(workbook.cancelCalculation(), true);
  await assert.rejects(
    cancelledTask,
    (error) =>
      error instanceof CellRuneError && error.code === "session.cancelled",
  );

  const closedTask = workbook.recalculate({ mode: "full" });
  await waitUntilActive(workbook, closedTask);
  workbook.close();
  workbook.close();
  assert.equal(workbook.closed, true);
  await assert.rejects(
    closedTask,
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.session.closed" &&
      error.kind === "state",
  );
  assert.throws(
    () => workbook.summary(),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.session.closed" &&
      error.kind === "state",
  );
  await assert.rejects(
    workbook.recalculate(),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.session.closed" &&
      error.kind === "state",
  );
}

async function waitUntilActive(workbook, calculation) {
  let settled = false;
  void calculation.then(
    () => {
      settled = true;
    },
    () => {
      settled = true;
    },
  );
  for (let attempt = 0; attempt < 10_000; attempt += 1) {
    try {
      if (workbook.calculationActive()) {
        return;
      }
    } catch (error) {
      if (
        !(error instanceof CellRuneError) ||
        error.code !== "interop.session.unavailable"
      ) {
        throw error;
      }
    }
    if (settled) {
      assert.fail("calculation completed before its active state was observable");
    }
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.fail("calculation did not become active");
}

function fromCorpusChange(change) {
  switch (change.kind) {
    case "set_value":
      return {
        kind: "setValue",
        sheet: change.sheet,
        address: change.address,
        value: change.value,
      };
    case "set_formula":
      return {
        kind: "setFormula",
        sheet: change.sheet,
        address: change.address,
        formula: change.formula,
        dynamicRange: change.dynamic_range,
      };
    default:
      assert.fail(`unknown interactive corpus change: ${change.kind}`);
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
