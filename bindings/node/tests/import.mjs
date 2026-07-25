import assert from "node:assert/strict";
import { SCHEMA_VERSION, Workbook } from "../index.mjs";

assert.equal(SCHEMA_VERSION, 1);
const workbook = Workbook.create();
assert.equal(workbook.summary().sheets[0].name, "Sheet1");
