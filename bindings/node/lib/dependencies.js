"use strict";

const { requireObject } = require("./validation.js");

function cellReference(cell) {
  return { sheetId: cell.sheet_id, sheetName: cell.sheet_name, address: cell.address };
}

/** Converts a native precedent page, sent as JSON, to camel-case fields. */
function normalizePrecedentPage(json) {
  const page = JSON.parse(json);
  requireObject(page, "precedent page");
  return {
    schemaVersion: page.schema_version,
    cell: cellReference(page.cell),
    dynamic: page.dynamic,
    total: page.total,
    offset: page.offset,
    nextOffset: page.next_offset ?? null,
    entries: page.entries.map((entry) => ({
      sheetId: entry.sheet_id,
      sheetName: entry.sheet_name,
      range: entry.range,
      kind: entry.kind,
    })),
  };
}

/** Converts a native dependent page, sent as JSON, to camel-case fields. */
function normalizeDependentPage(json) {
  const page = JSON.parse(json);
  requireObject(page, "dependent page");
  return {
    schemaVersion: page.schema_version,
    cell: cellReference(page.cell),
    unlistedDynamicFormulas: page.unlisted_dynamic_formulas,
    total: page.total,
    offset: page.offset,
    nextOffset: page.next_offset ?? null,
    entries: page.entries.map((entry) => ({
      cell: cellReference(entry.cell),
      dynamic: entry.dynamic,
    })),
  };
}

module.exports = { normalizeDependentPage, normalizePrecedentPage };
