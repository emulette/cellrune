"use strict";

const { inputError } = require("./errors.js");
const { requireNonNegativeInteger, requireString } = require("./validation.js");

/** Rule fields as `dataValidations` returns them, and their interop names. */
const RULE_KEYS = {
  ranges: "ranges",
  validationType: "validation_type",
  operator: "operator",
  formula1: "formula1",
  formula2: "formula2",
  allowBlank: "allow_blank",
  inCellDropdown: "in_cell_dropdown",
  showInputMessage: "show_input_message",
  showErrorMessage: "show_error_message",
  errorStyle: "error_style",
  errorTitle: "error_title",
  errorMessage: "error_message",
  promptTitle: "prompt_title",
  promptMessage: "prompt_message",
};
const FLAGS = new Set(["allowBlank", "inCellDropdown", "showInputMessage", "showErrorMessage"]);
const OPTIONAL_TEXT = new Set([
  "formula1",
  "formula2",
  "errorTitle",
  "errorMessage",
  "promptTitle",
  "promptMessage",
]);

function requireKeys(value, allowed, path) {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) {
      throw inputError(`${path}.${key} is not supported`);
    }
  }
}

function serializeRanges(ranges, path) {
  if (!Array.isArray(ranges)) {
    throw inputError(`${path} must be an array of ranges`);
  }
  return ranges.map((range, index) => {
    requireString(range, `${path}[${index}]`);
    return range;
  });
}

/** Serializes a rule in the shape `dataValidations` returns. */
function serializeRule(rule, path) {
  if (rule === null || typeof rule !== "object" || Array.isArray(rule)) {
    throw inputError(`${path} must be a data validation rule`);
  }
  requireKeys(rule, Object.keys(RULE_KEYS), path);
  const serialized = {};
  for (const [key, target] of Object.entries(RULE_KEYS)) {
    const value = rule[key];
    if (key === "ranges") {
      serialized[target] = serializeRanges(value, `${path}.ranges`);
    } else if (OPTIONAL_TEXT.has(key)) {
      if (value !== null && value !== undefined) {
        requireString(value, `${path}.${key}`);
      }
      serialized[target] = value ?? null;
    } else if (FLAGS.has(key)) {
      if (typeof value !== "boolean") {
        throw inputError(`${path}.${key} must be a boolean`);
      }
      serialized[target] = value;
    } else {
      requireString(value, `${path}.${key}`);
      serialized[target] = value;
    }
  }
  return serialized;
}

/**
 * Serializes a `setDataValidation`, `clearDataValidations`, `removeDataValidation`, or
 * `setDataValidations` change, or returns `undefined` for any other kind.
 */
function serializeDataValidationChange(change, index) {
  const path = `changes[${index}]`;
  switch (change.kind) {
    case "setDataValidation":
      requireKeys(change, ["kind", "sheet", "rule"], path);
      requireString(change.sheet, `${path}.sheet`);
      return {
        kind: "set_data_validation",
        sheet: change.sheet,
        rule: serializeRule(change.rule, `${path}.rule`),
      };
    case "clearDataValidations":
      requireKeys(change, ["kind", "sheet", "ranges"], path);
      requireString(change.sheet, `${path}.sheet`);
      return {
        kind: "clear_data_validations",
        sheet: change.sheet,
        ranges: serializeRanges(change.ranges, `${path}.ranges`),
      };
    case "removeDataValidation":
      requireKeys(change, ["kind", "sheet", "index"], path);
      requireString(change.sheet, `${path}.sheet`);
      requireNonNegativeInteger(change.index, `${path}.index`);
      return {
        kind: "remove_data_validation",
        sheet: change.sheet,
        index: change.index,
      };
    case "setDataValidations":
      requireKeys(change, ["kind", "sheet", "rules"], path);
      requireString(change.sheet, `${path}.sheet`);
      if (!Array.isArray(change.rules)) {
        throw inputError(`${path}.rules must be an array of rules`);
      }
      return {
        kind: "set_data_validations",
        sheet: change.sheet,
        rules: change.rules.map((rule, ruleIndex) =>
          serializeRule(rule, `${path}.rules[${ruleIndex}]`),
        ),
      };
    default:
      return undefined;
  }
}

module.exports = { serializeDataValidationChange };
