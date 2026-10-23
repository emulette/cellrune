"use strict";

const { inputError } = require("./errors.js");
const { serializeCellValue } = require("./changes.js");

/** Validates the rows of a rectangle write; the native session checks that they are rectangular. */
function serializeRangeRows(rows) {
  if (!Array.isArray(rows)) {
    throw inputError("rows must be an array of rows");
  }
  return rows.map((row, rowIndex) => {
    if (!Array.isArray(row)) {
      throw inputError(`rows[${rowIndex}] must be an array of cell values`);
    }
    return row.map((value, columnIndex) =>
      serializeCellValue(value, `rows[${rowIndex}][${columnIndex}]`),
    );
  });
}

/** Converts native rectangle rows, sent as JSON, to cell values and unavailable results. */
function parseRangeValues(json) {
  return JSON.parse(json).map((row) =>
    row.map((value) =>
      value.kind === "unavailable"
        ? {
            kind: "unavailable",
            code: value.code,
            message: value.message,
            detail: value.detail ?? null,
            ...(value.limit == null ? {} : { limit: value.limit }),
          }
        : value,
    ),
  );
}

module.exports = { parseRangeValues, serializeRangeRows };
