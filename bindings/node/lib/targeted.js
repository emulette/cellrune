"use strict";

const { inputError, INPUT_DETAIL } = require("./errors.js");
const { requireOptions, requireOptionKeys, requireString, requireOptionalString,
  requireOptionalFinite, requireNonNegativeInteger } = require("./validation.js");

function targetRequest(targets, options) {
  if (!Array.isArray(targets)) throw inputError(INPUT_DETAIL.TARGETS_ARRAY);
  requireOptions(options);
  requireOptionKeys(options, ["todaySerial", "nowSerial", "arithmeticSemantics", "financialSolverSemantics", "limits"]);
  requireOptionalFinite(options.todaySerial, "todaySerial");
  requireOptionalFinite(options.nowSerial, "nowSerial");
  requireOptionalString(options.arithmeticSemantics, "arithmeticSemantics");
  requireOptionalString(options.financialSolverSemantics, "financialSolverSemantics");
  const limits = {};
  if (options.limits !== undefined) {
    requireOptions(options.limits);
    requireOptionKeys(options.limits, ["maxTargets", "maxResultCells", "maxEvaluatedCells"]);
    for (const [key, wire] of [["maxTargets", "max_targets"], ["maxResultCells", "max_result_cells"], ["maxEvaluatedCells", "max_evaluated_cells"]]) {
      if (options.limits[key] !== undefined) {
        requireNonNegativeInteger(options.limits[key], key);
        limits[wire] = options.limits[key];
      }
    }
  }
  return JSON.stringify({
    targets: targets.map(target => {
      requireOptions(target);
      requireOptionKeys(target, ["sheet", "start", "end"]);
      requireString(target.sheet, "sheet");
      requireString(target.start, "start");
      requireOptionalString(target.end, "end");
      return { sheet: target.sheet, start: target.start, end: target.end };
    }),
    options: { today_serial: options.todaySerial, now_serial: options.nowSerial,
      arithmetic_semantics: options.arithmeticSemantics, financial_solver_semantics: options.financialSolverSemantics },
    limits,
  });
}

module.exports = { targetRequest };
