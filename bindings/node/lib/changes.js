"use strict";

const { inputError } = require("./errors.js");
const { serializeFormatChange } = require("./formats.js");
const { serializePresentationChange } = require("./presentation.js");
const {
  requireFinite,
  requireNonNegativeInteger,
  requireOptionalBoolean,
  requireOptions,
  requireString,
} = require("./validation.js");

function serializeWorkbookChange(change, index) {
  requireOptions(change);
  requireString(change.kind, `changes[${index}].kind`);
  switch (change.kind) {
    case "setValue":
      return {
        kind: "set_value",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        address: requiredChangeString(change.address, index, "address"),
        value: serializeCellValue(change.value, `changes[${index}].value`),
      };
    case "setFormula":
      if (
        change.dynamicRange !== undefined &&
        change.dynamicRange !== null
      ) {
        requireString(change.dynamicRange, `changes[${index}].dynamicRange`);
      }
      requireOptionalBoolean(change.dynamicArray, `changes[${index}].dynamicArray`);
      return {
        kind: "set_formula",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        address: requiredChangeString(change.address, index, "address"),
        formula: requiredChangeString(change.formula, index, "formula"),
        dynamic_range: change.dynamicRange ?? null,
        ...(change.dynamicArray === true ? { dynamic_array: true } : {}),
      };
    case "clearCell":
      return {
        kind: "clear_cell",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        address: requiredChangeString(change.address, index, "address"),
      };
    case "setNumberFormat":
      requireNonNegativeInteger(change.id, `changes[${index}].id`);
      if (change.code !== undefined && change.code !== null) {
        requireString(change.code, `changes[${index}].code`);
      }
      return {
        kind: "set_number_format",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        address: requiredChangeString(change.address, index, "address"),
        id: change.id,
        code: change.code ?? null,
        format_kind: requiredChangeString(
          change.formatKind,
          index,
          "formatKind",
        ),
      };
    case "addSheet":
      return {
        kind: "add_sheet",
        name: requiredChangeString(change.name, index, "name"),
      };
    case "renameSheet":
      return {
        kind: "rename_sheet",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        new_name: requiredChangeString(change.newName, index, "newName"),
      };
    case "deleteSheet":
      return {
        kind: "delete_sheet",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
      };
    case "moveSheet":
      requireNonNegativeInteger(change.position, `changes[${index}].position`);
      return {
        kind: "move_sheet",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        position: change.position,
      };
    case "copySheet":
      requireNonNegativeInteger(change.position, `changes[${index}].position`);
      return {
        kind: "copy_sheet",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        position: change.position,
      };
    case "setSheetVisibility":
      return {
        kind: "set_sheet_visibility",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        visibility: requiredChangeString(
          change.visibility,
          index,
          "visibility",
        ),
      };
    case "setSheetAutoFilter":
      return {
        kind: "set_sheet_auto_filter",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        range: optionalChangeString(change.range, index, "range"),
      };
    case "setDefinedName":
      if (change.scopeSheet !== undefined && change.scopeSheet !== null) {
        requireString(change.scopeSheet, `changes[${index}].scopeSheet`);
      }
      if (typeof change.hidden !== "boolean") {
        throw inputError(`changes[${index}].hidden must be a boolean`);
      }
      return {
        kind: "set_defined_name",
        name: requiredChangeString(change.name, index, "name"),
        scope_sheet: change.scopeSheet ?? null,
        formula: requiredChangeString(change.formula, index, "formula"),
        hidden: change.hidden,
      };
    case "removeDefinedName":
      if (change.scopeSheet !== undefined && change.scopeSheet !== null) {
        requireString(change.scopeSheet, `changes[${index}].scopeSheet`);
      }
      return {
        kind: "remove_defined_name",
        name: requiredChangeString(change.name, index, "name"),
        scope_sheet: change.scopeSheet ?? null,
      };
    case "renameDefinedName":
      return {
        kind: "rename_defined_name",
        name: requiredChangeString(change.name, index, "name"),
        scope_sheet: optionalChangeString(
          change.scopeSheet,
          index,
          "scopeSheet",
        ),
        new_name: requiredChangeString(change.newName, index, "newName"),
      };
    case "insertRows":
      requireNonNegativeInteger(change.row, `changes[${index}].row`);
      requireNonNegativeInteger(change.count, `changes[${index}].count`);
      return {
        kind: "insert_rows",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        row: change.row,
        count: change.count,
      };
    case "deleteRows":
    case "deleteColumns":
      requireNonNegativeInteger(change.first, `changes[${index}].first`);
      requireNonNegativeInteger(change.last, `changes[${index}].last`);
      return {
        kind: change.kind === "deleteRows" ? "delete_rows" : "delete_columns",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        first: change.first,
        last: change.last,
      };
    case "insertColumns":
      requireNonNegativeInteger(change.column, `changes[${index}].column`);
      requireNonNegativeInteger(change.count, `changes[${index}].count`);
      return {
        kind: "insert_columns",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        column: change.column,
        count: change.count,
      };
    case "moveRows":
    case "moveColumns":
      requireNonNegativeInteger(change.first, `changes[${index}].first`);
      requireNonNegativeInteger(change.last, `changes[${index}].last`);
      requireNonNegativeInteger(
        change.destination,
        `changes[${index}].destination`,
      );
      return {
        kind: change.kind === "moveRows" ? "move_rows" : "move_columns",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        first: change.first,
        last: change.last,
        destination: change.destination,
      };
    case "moveRange":
      return {
        kind: "move_range",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        source: requiredChangeString(change.source, index, "source"),
        destination: requiredChangeString(
          change.destination,
          index,
          "destination",
        ),
      };
    case "copyRange":
      return {
        kind: "copy_range",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        source: requiredChangeString(change.source, index, "source"),
        destination_sheet: optionalChangeString(
          change.destinationSheet,
          index,
          "destinationSheet",
        ),
        destination: requiredChangeString(
          change.destination,
          index,
          "destination",
        ),
      };
    case "setDateSystem":
      return {
        kind: "set_date_system",
        date_system: requiredChangeString(
          change.dateSystem,
          index,
          "dateSystem",
        ),
      };
    case "setCalculationHints":
      if (change.mode !== undefined && change.mode !== null) {
        requireString(change.mode, `changes[${index}].mode`);
      }
      if (
        change.calculationId !== undefined &&
        change.calculationId !== null
      ) {
        requireNonNegativeInteger(
          change.calculationId,
          `changes[${index}].calculationId`,
        );
      }
      requireOptionalBoolean(
        change.fullCalculationOnLoad,
        `changes[${index}].fullCalculationOnLoad`,
      );
      requireOptionalBoolean(
        change.forceFullCalculation,
        `changes[${index}].forceFullCalculation`,
      );
      requireOptionalBoolean(
        change.iterativeCalculation,
        `changes[${index}].iterativeCalculation`,
      );
      return {
        kind: "set_calculation_hints",
        mode: change.mode ?? null,
        calculation_id: change.calculationId ?? null,
        full_calculation_on_load: change.fullCalculationOnLoad ?? null,
        force_full_calculation: change.forceFullCalculation ?? null,
        iterative_calculation: change.iterativeCalculation ?? null,
      };
    default:
      throw inputError(`changes[${index}].kind is not recognized`);
  }
}

function serializeWorkbookChangeV2(change, index) {
  requireOptions(change);
  requireString(change.kind, `changes[${index}].kind`);
  const format = serializeFormatChange(change, index);
  if (format !== undefined) {
    return format;
  }
  const presentation = serializePresentationChange(change, index);
  if (presentation !== undefined) {
    return presentation;
  }
  validateV2ChangeKeys(change, index);
  switch (change.kind) {
    case "renameTable":
      requireNonNegativeInteger(change.tableId, `changes[${index}].tableId`);
      return {
        kind: "rename_table",
        table_id: change.tableId,
        new_display_name: requiredChangeString(
          change.newDisplayName,
          index,
          "newDisplayName",
        ),
      };
    case "renameTableColumn":
      requireNonNegativeInteger(change.tableId, `changes[${index}].tableId`);
      requireNonNegativeInteger(change.columnId, `changes[${index}].columnId`);
      return {
        kind: "rename_table_column",
        table_id: change.tableId,
        column_id: change.columnId,
        new_name: requiredChangeString(change.newName, index, "newName"),
      };
    case "resizeTableRows":
      requireNonNegativeInteger(change.tableId, `changes[${index}].tableId`);
      requireNonNegativeInteger(
        change.firstDataRow,
        `changes[${index}].firstDataRow`,
      );
      requireNonNegativeInteger(
        change.lastDataRow,
        `changes[${index}].lastDataRow`,
      );
      return {
        kind: "resize_table_rows",
        table_id: change.tableId,
        first_data_row: change.firstDataRow,
        last_data_row: change.lastDataRow,
      };
    case "createTable":
      return {
        kind: "create_table",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        range: requiredChangeString(change.range, index, "range"),
        name: optionalChangeString(change.name, index, "name"),
      };
    case "setTableRange":
      requireNonNegativeInteger(change.tableId, `changes[${index}].tableId`);
      return {
        kind: "set_table_range",
        table_id: change.tableId,
        range: requiredChangeString(change.range, index, "range"),
      };
    case "convertTableToRange":
      requireNonNegativeInteger(change.tableId, `changes[${index}].tableId`);
      return {
        kind: "convert_table_to_range",
        table_id: change.tableId,
      };
    case "widenTableColumns":
      requireNonNegativeInteger(change.tableId, `changes[${index}].tableId`);
      return {
        kind: "widen_table_columns",
        table_id: change.tableId,
        range: requiredChangeString(change.range, index, "range"),
      };
    case "widenSheetAutoFilter":
      return {
        kind: "widen_sheet_auto_filter",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        range: requiredChangeString(change.range, index, "range"),
      };
    default:
      return serializeWorkbookChange(change, index);
  }
}

const V2_CHANGE_KEYS = new Map([
  ["setValue", ["kind", "sheet", "address", "value"]],
  ["setFormula", ["kind", "sheet", "address", "formula", "dynamicRange", "dynamicArray"]],
  ["clearCell", ["kind", "sheet", "address"]],
  [
    "setNumberFormat",
    ["kind", "sheet", "address", "id", "code", "formatKind"],
  ],
  ["addSheet", ["kind", "name"]],
  ["renameSheet", ["kind", "sheet", "newName"]],
  ["moveSheet", ["kind", "sheet", "position"]],
  ["copySheet", ["kind", "sheet", "position"]],
  ["setSheetVisibility", ["kind", "sheet", "visibility"]],
  ["setSheetAutoFilter", ["kind", "sheet", "range"]],
  [
    "setDefinedName",
    ["kind", "name", "scopeSheet", "formula", "hidden"],
  ],
  ["removeDefinedName", ["kind", "name", "scopeSheet"]],
  ["renameDefinedName", ["kind", "name", "scopeSheet", "newName"]],
  ["insertRows", ["kind", "sheet", "row", "count"]],
  ["deleteRows", ["kind", "sheet", "first", "last"]],
  ["insertColumns", ["kind", "sheet", "column", "count"]],
  ["deleteColumns", ["kind", "sheet", "first", "last"]],
  ["moveRows", ["kind", "sheet", "first", "last", "destination"]],
  ["moveColumns", ["kind", "sheet", "first", "last", "destination"]],
  ["moveRange", ["kind", "sheet", "source", "destination"]],
  [
    "copyRange",
    ["kind", "sheet", "source", "destinationSheet", "destination"],
  ],
  ["setDateSystem", ["kind", "dateSystem"]],
  [
    "setCalculationHints",
    [
      "kind",
      "mode",
      "calculationId",
      "fullCalculationOnLoad",
      "forceFullCalculation",
      "iterativeCalculation",
    ],
  ],
  ["renameTable", ["kind", "tableId", "newDisplayName"]],
  ["renameTableColumn", ["kind", "tableId", "columnId", "newName"]],
  [
    "resizeTableRows",
    ["kind", "tableId", "firstDataRow", "lastDataRow"],
  ],
  ["createTable", ["kind", "sheet", "range", "name"]],
  ["setTableRange", ["kind", "tableId", "range"]],
  ["convertTableToRange", ["kind", "tableId"]],
  ["widenTableColumns", ["kind", "tableId", "range"]],
  ["widenSheetAutoFilter", ["kind", "sheet", "range"]],
]);

function validateV2ChangeKeys(change, index) {
  const allowed = V2_CHANGE_KEYS.get(change.kind);
  if (allowed === undefined) {
    return;
  }
  for (const key of Object.keys(change)) {
    if (!allowed.includes(key)) {
      throw inputError(`changes[${index}].${key} is not supported`);
    }
  }
  if (change.kind === "setValue") {
    validateV2CellValueKeys(change.value, index);
  }
}

function validateV2CellValueKeys(value, index) {
  requireOptions(value);
  requireString(value.kind, `changes[${index}].value.kind`);
  const allowed =
    value.kind === "blank" ? ["kind"] : ["kind", "value"];
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) {
      throw inputError(`changes[${index}].value.${key} is not supported`);
    }
  }
}

/** Validates one writable cell value, naming it `path` in errors. */
function serializeCellValue(value, path) {
  requireOptions(value);
  requireString(value.kind, `${path}.kind`);
  switch (value.kind) {
    case "blank":
      return { kind: "blank" };
    case "number":
      requireFinite(value.value, `${path}.value`);
      return { kind: "number", value: value.value };
    case "text":
      requireString(value.value, `${path}.value`);
      return { kind: "text", value: value.value };
    case "logical":
      if (typeof value.value !== "boolean") {
        throw inputError(`${path}.value must be a boolean`);
      }
      return { kind: "logical", value: value.value };
    case "error":
      requireString(value.value, `${path}.value`);
      return { kind: "error", value: value.value };
    default:
      throw inputError(`${path}.kind is not writable`);
  }
}

function requiredChangeString(value, index, name) {
  requireString(value, `changes[${index}].${name}`);
  return value;
}

/** Returns an optional string field, or `null` when it is omitted. */
function optionalChangeString(value, index, name) {
  return value === undefined || value === null
    ? null
    : requiredChangeString(value, index, name);
}

module.exports = { serializeCellValue, serializeWorkbookChange, serializeWorkbookChangeV2 };
