"use strict";

const { inputError } = require("./errors.js");
const {
  requireNonNegativeInteger,
  requireObject,
  requireOptions,
  requireString,
} = require("./validation.js");

const FONT_KEYS = {
  name: "name",
  size: "size",
  bold: "bold",
  italic: "italic",
  underline: "underline",
  strike: "strike",
  color: "color",
};
const FILL_KEYS = {
  pattern: "pattern",
  foreground: "foreground",
  background: "background",
};
const BORDER_KEYS = {
  left: "left",
  right: "right",
  top: "top",
  bottom: "bottom",
  diagonal: "diagonal",
  diagonalUp: "diagonal_up",
  diagonalDown: "diagonal_down",
};
const EDGE_KEYS = { style: "style", color: "color" };
const ALIGNMENT_KEYS = {
  horizontal: "horizontal",
  vertical: "vertical",
  wrapText: "wrap_text",
  shrinkToFit: "shrink_to_fit",
  indent: "indent",
  textRotation: "text_rotation",
};
const COLOR_KEYS = { kind: "kind", argb: "argb", index: "index", tint: "tint" };
const NESTED = {
  color: (value, path) => renameKeys(value, COLOR_KEYS, path),
  foreground: (value, path) => renameKeys(value, COLOR_KEYS, path),
  background: (value, path) => renameKeys(value, COLOR_KEYS, path),
};
const EDGES = new Set(["left", "right", "top", "bottom", "diagonal"]);

/** Renames the camel-case keys of one format object to the snake-case keys native code reads. */
function renameKeys(value, keys, path) {
  requireOptions(value);
  const renamed = {};
  for (const [key, entry] of Object.entries(value)) {
    const target = keys[key];
    if (target === undefined) {
      throw inputError(`${path}.${key} is not supported`);
    }
    if (entry !== null && NESTED[key] !== undefined) {
      renamed[target] = NESTED[key](entry, `${path}.${key}`);
    } else if (EDGES.has(key)) {
      renamed[target] = renameKeys(entry, EDGE_KEYS, `${path}.${key}`);
    } else {
      renamed[target] = entry;
    }
  }
  return renamed;
}

function optionalFormat(value, keys, path) {
  return value === undefined ? undefined : renameKeys(value, keys, path);
}

/**
 * Serializes a `setCellFormat`, `setColumnWidth`, or `setRowHeight` change, or returns
 * `undefined` for any other kind.
 */
function serializeFormatChange(change, index) {
  const path = `changes[${index}]`;
  switch (change.kind) {
    case "setCellFormat":
      requireKeys(change, ["kind", "sheet", "address", "font", "fill", "border", "alignment"], path);
      requireString(change.sheet, `${path}.sheet`);
      requireString(change.address, `${path}.address`);
      return {
        kind: "set_cell_format",
        sheet: change.sheet,
        address: change.address,
        font: optionalFormat(change.font, FONT_KEYS, `${path}.font`),
        fill: optionalFormat(change.fill, FILL_KEYS, `${path}.fill`),
        border: optionalFormat(change.border, BORDER_KEYS, `${path}.border`),
        alignment: optionalFormat(change.alignment, ALIGNMENT_KEYS, `${path}.alignment`),
      };
    case "setColumnWidth":
      requireKeys(change, ["kind", "sheet", "first", "last", "width"], path);
      requireString(change.sheet, `${path}.sheet`);
      requireNonNegativeInteger(change.first, `${path}.first`);
      requireNonNegativeInteger(change.last, `${path}.last`);
      return {
        kind: "set_column_width",
        sheet: change.sheet,
        first: change.first,
        last: change.last,
        width: change.width ?? null,
      };
    case "setRowHeight":
      requireKeys(change, ["kind", "sheet", "row", "height"], path);
      requireString(change.sheet, `${path}.sheet`);
      requireNonNegativeInteger(change.row, `${path}.row`);
      return {
        kind: "set_row_height",
        sheet: change.sheet,
        row: change.row,
        height: change.height ?? null,
      };
    default:
      return undefined;
  }
}

function requireKeys(value, allowed, path) {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) {
      throw inputError(`${path}.${key} is not supported`);
    }
  }
}

function normalizeColor(color) {
  if (color === null || color === undefined) {
    return null;
  }
  switch (color.kind) {
    case "argb":
      return { kind: "argb", argb: color.argb, tint: color.tint };
    case "theme":
    case "indexed":
      return { kind: color.kind, index: color.index, tint: color.tint };
    default:
      return { kind: color.kind, tint: color.tint };
  }
}

function normalizeEdge(edge) {
  return { style: edge.style, color: normalizeColor(edge.color) };
}

function normalizeCellFormat(format) {
  const { font, fill, border, alignment } = format;
  return {
    font: {
      name: font.name ?? null,
      size: font.size ?? null,
      bold: font.bold,
      italic: font.italic,
      underline: font.underline,
      strike: font.strike,
      color: normalizeColor(font.color),
    },
    fill:
      fill.kind === "pattern"
        ? {
            kind: "pattern",
            pattern: fill.pattern,
            foreground: normalizeColor(fill.foreground),
            background: normalizeColor(fill.background),
          }
        : { kind: fill.kind },
    border: {
      left: normalizeEdge(border.left),
      right: normalizeEdge(border.right),
      top: normalizeEdge(border.top),
      bottom: normalizeEdge(border.bottom),
      diagonal: normalizeEdge(border.diagonal),
      diagonalUp: border.diagonal_up,
      diagonalDown: border.diagonal_down,
    },
    alignment: {
      horizontal: alignment.horizontal,
      vertical: alignment.vertical,
      wrapText: alignment.wrap_text,
      shrinkToFit: alignment.shrink_to_fit,
      indent: alignment.indent,
      textRotation: alignment.text_rotation,
    },
  };
}

/** Converts a native format page, sent as JSON, to camel-case fields. */
function normalizeCellFormatPage(json) {
  const page = JSON.parse(json);
  requireObject(page, "cell format page");
  return {
    schemaVersion: page.schema_version,
    sheet: page.sheet,
    start: page.start,
    end: page.end,
    totalCells: page.total_cells,
    offset: page.offset,
    nextOffset: page.next_offset ?? null,
    cells: page.cells.map((cell) => ({
      address: cell.address,
      format: normalizeCellFormat(cell.format),
    })),
    columnWidths: page.column_widths.map((span) => ({
      first: span.first,
      last: span.last,
      width: span.width,
    })),
    rowHeights: page.row_heights.map((row) => ({
      row: row.row,
      height: row.height,
    })),
  };
}

module.exports = { normalizeCellFormatPage, serializeFormatChange };
