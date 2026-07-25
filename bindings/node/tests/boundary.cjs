"use strict";

const assert = require("node:assert/strict");
const { CellRuneError, Workbook } = require("..");
const native = require("../native.js");

function isInvalidInput(error) {
  return (
    error instanceof CellRuneError &&
    error.code === "interop.input.invalid" &&
    error.kind === "input"
  );
}

function isNativeClosed(error) {
  return (
    error instanceof Error &&
    error.message.includes('"code":"interop.session.closed"')
  );
}

async function main() {
  const workbook = Workbook.create();
  for (const options of [null, "options", 1, [], new Date(), new Map()]) {
    assert.throws(
      () => workbook.readRange("Sheet1", "A1", "A1", options),
      isInvalidInput,
    );
    assert.throws(
      () => workbook.setFormula("Sheet1", "A1", "=1", options),
      isInvalidInput,
    );
    await assert.rejects(workbook.calculate(options), isInvalidInput);
    await assert.rejects(workbook.toBytes(options), isInvalidInput);
    await assert.rejects(workbook.save("ignored.xlsx", options), isInvalidInput);
  }

  for (const limit of [2 ** 32, 2 ** 32 + 1, Number.MAX_SAFE_INTEGER]) {
    assert.throws(
      () => workbook.readRange("Sheet1", "A1", "A1", { limit }),
      (error) =>
        error instanceof CellRuneError &&
        error.code === "interop.page.limit_invalid" &&
        error.kind === "input",
    );
  }
  assert.throws(
    () => workbook.readRange("Sheet1", "A1", "A1", { limit: 10_001 }),
    (error) =>
      error instanceof CellRuneError &&
      error.code === "interop.page.limit_exceeded" &&
      error.kind === "input",
  );

  const nativeWorkbook = native.createWorkbook();
  assert.equal(nativeWorkbook.closed, false);
  nativeWorkbook.close();
  nativeWorkbook.close();
  assert.equal(nativeWorkbook.closed, true);
  assert.throws(() => nativeWorkbook.summary(), isNativeClosed);
  await assert.rejects(nativeWorkbook.calculate(null, null), isNativeClosed);
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
