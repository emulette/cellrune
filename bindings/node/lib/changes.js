"use strict";

const { inputError } = require("./errors.js");
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
        value: serializeCellValue(change.value, index),
      };
    case "setFormula":
      if (
        change.dynamicRange !== undefined &&
        change.dynamicRange !== null
      ) {
        requireString(change.dynamicRange, `changes[${index}].dynamicRange`);
      }
      return {
        kind: "set_formula",
        sheet: requiredChangeString(change.sheet, index, "sheet"),
        address: requiredChangeString(change.address, index, "address"),
        formula: requiredChangeString(change.formula, index, "formula"),
        dynamic_range: change.dynamicRange ?? null,
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
      return {
        kind: "set_calculation_hints",
        mode: change.mode ?? null,
        calculation_id: change.calculationId ?? null,
        full_calculation_on_load: change.fullCalculationOnLoad ?? null,
        force_full_calculation: change.forceFullCalculation ?? null,
      };
    default:
      throw inputError(`changes[${index}].kind is not recognized`);
  }
}

function serializeCellValue(value, index) {
  requireOptions(value);
  requireString(value.kind, `changes[${index}].value.kind`);
  switch (value.kind) {
    case "blank":
      return { kind: "blank" };
    case "number":
      requireFinite(value.value, `changes[${index}].value.value`);
      return { kind: "number", value: value.value };
    case "text":
      requireString(value.value, `changes[${index}].value.value`);
      return { kind: "text", value: value.value };
    case "logical":
      if (typeof value.value !== "boolean") {
        throw inputError(`changes[${index}].value.value must be a boolean`);
      }
      return { kind: "logical", value: value.value };
    case "error":
      requireString(value.value, `changes[${index}].value.value`);
      return { kind: "error", value: value.value };
    default:
      throw inputError(`changes[${index}].value.kind is not writable`);
  }
}

function requiredChangeString(value, index, name) {
  requireString(value, `changes[${index}].${name}`);
  return value;
}

module.exports = { serializeWorkbookChange };
