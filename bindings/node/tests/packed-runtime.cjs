"use strict";

const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const { createRequire } = require("node:module");
const path = require("node:path");

const PACKAGE_NAME = "@cellrune/node";
const PACKAGE_VERSION = "0.1.0";

async function main() {
  const consumerDirectory = process.argv[2];
  const platformPackageName = process.argv[3];
  assert.ok(consumerDirectory, "consumer directory argument is required");
  assert.ok(platformPackageName, "platform package argument is required");

  const requireFromConsumer = createRequire(
    path.join(consumerDirectory, "consumer.cjs"),
  );
  const installedEntry = requireFromConsumer.resolve(PACKAGE_NAME);
  const installedManifest = JSON.parse(
    fs.readFileSync(
      path.join(path.dirname(installedEntry), "package.json"),
      "utf8",
    ),
  );
  assert.equal(installedManifest.version, PACKAGE_VERSION);
  assert.equal(
    installedManifest.optionalDependencies[platformPackageName],
    PACKAGE_VERSION,
  );
  for (const dependencyVersion of Object.values(
    installedManifest.optionalDependencies,
  )) {
    assert.equal(dependencyVersion, PACKAGE_VERSION);
    assert.ok(!dependencyVersion.startsWith("workspace:"));
  }

  const { Workbook } = requireFromConsumer(PACKAGE_NAME);
  const workbook = Workbook.create();
  workbook.setNumber("Sheet1", "A1", 41);
  workbook.setFormula("Sheet1", "B1", "=A1+1");
  const report = await workbook.calculate();
  assert.equal(report.unavailableCount, 0);

  const outputPath = path.join(consumerDirectory, "output.xlsx");
  await workbook.save(outputPath);
  const reopened = await Workbook.openPath(outputPath);
  const page = reopened.readRange("Sheet1", "B1", "B1");
  assert.deepEqual(page.cells[0].sourceValue, {
    kind: "number",
    value: 42,
  });

  const esm = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "--eval",
      `import { Workbook } from "${PACKAGE_NAME}"; if (Workbook.create().summary().sheets.length !== 1) process.exit(2);`,
    ],
    {
      cwd: consumerDirectory,
      encoding: "utf8",
    },
  );
  assert.equal(esm.status, 0, esm.stderr || esm.stdout);
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
