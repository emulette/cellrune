"use strict";

const assert = require("node:assert/strict");
const { CellRuneError, Workbook } = require("..");

function assertBusyError(error) {
  assert.ok(error instanceof CellRuneError);
  assert.equal(error.code, "interop.session.unavailable");
  assert.equal(error.kind, "state");
}

async function main() {
  const workbook = Workbook.create();
  const changes = [
    {
      kind: "setValue",
      sheet: "Sheet1",
      address: "A1",
      value: { kind: "number", value: 1 },
    },
  ];
  for (let row = 2; row <= 25_000; row += 1) {
    changes.push({
      kind: "setFormula",
      sheet: "Sheet1",
      address: `A${row}`,
      formula: `=A${row - 1}+1`,
    });
  }
  workbook.applyChanges(0n, changes);

  let ticks = 0;
  const timer = setInterval(() => {
    ticks += 1;
  }, 1);
  try {
    const calculation = workbook.calculate();

    try {
      const summary = workbook.summary();
      assert.equal(summary.sheets.length, 1);
    } catch (error) {
      assertBusyError(error);
    }

    const report = await calculation;
    assert.equal(report.unavailableCount, 0);
    assert.ok(ticks > 0, "native calculation blocked the JavaScript event loop");

    ticks = 0;
    const previewTask = workbook.previewChanges(workbook.summary().semanticRevision, [
      {
        kind: "setValue",
        sheet: "Sheet1",
        address: "A1",
        value: { kind: "number", value: 2 },
      },
    ]);
    const preview = await previewTask;
    assert.ok(ticks > 0, "native preview blocked the JavaScript event loop");
    workbook.discardPreview(preview.previewId);
  } finally {
    clearInterval(timer);
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
