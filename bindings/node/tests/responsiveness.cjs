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
  workbook.setNumber("Sheet1", "A1", 1);
  for (let row = 2; row <= 25_000; row += 1) {
    workbook.setFormula("Sheet1", `A${row}`, `=A${row - 1}+1`);
  }

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
  } finally {
    clearInterval(timer);
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
