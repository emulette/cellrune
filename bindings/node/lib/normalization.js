"use strict";

const { protocolError } = require("./errors.js");
const {
  requireObject,
  requireProtocolFinite,
  requireProtocolString,
} = require("./validation.js");

function normalizeValue(value) {
  requireObject(value, "cell value");
  switch (value.kind) {
    case "blank":
    case "unsupported":
      return { kind: value.kind };
    case "number":
      requireProtocolFinite(value.numberValue, "native number value");
      return { kind: "number", value: value.numberValue };
    case "text":
      requireProtocolString(value.textValue, "native text value");
      return { kind: "text", value: value.textValue };
    case "logical":
      if (typeof value.logicalValue !== "boolean") {
        throw protocolError("native logical value is missing");
      }
      return { kind: "logical", value: value.logicalValue };
    case "error":
      requireProtocolString(value.errorValue, "native error value");
      return { kind: "error", value: value.errorValue };
    default:
      throw protocolError("native cell value kind is unknown");
  }
}

function normalizeResult(result) {
  if (result === null || result === undefined) {
    return null;
  }
  requireObject(result, "calculation result");
  if (result.kind === "value" && result.value) {
    return { kind: "value", value: normalizeValue(result.value) };
  }
  if (
    result.kind === "unavailable" &&
    typeof result.code === "string" &&
    typeof result.message === "string"
  ) {
    return {
      kind: "unavailable",
      code: result.code,
      message: result.message,
      detail: result.detail ?? null,
    };
  }
  throw protocolError("native calculation result is malformed");
}

function normalizeRangePage(page) {
  requireObject(page, "range page");
  return {
    schemaVersion: page.schemaVersion,
    sheet: page.sheet,
    start: page.start,
    end: page.end,
    totalCells: page.totalCells,
    offset: page.offset,
    nextOffset: page.nextOffset ?? null,
    cells: page.cells.map((cell) => ({
      address: cell.address,
      formula: cell.formula ?? null,
      sourceValue: normalizeValue(cell.sourceValue),
      sourceValueState: cell.sourceValueState,
      calculated: normalizeResult(cell.calculated),
    })),
  };
}

function normalizeSummary(summary) {
  requireObject(summary, "workbook summary");
  return {
    schemaVersion: summary.schemaVersion,
    semanticRevision: BigInt(summary.semanticRevision),
    documentBacked: summary.documentBacked,
    documentKind: summary.documentKind,
    dateSystem: summary.dateSystem,
    diagnosticCount: summary.diagnosticCount,
    sheets: summary.sheets.map((sheet) => ({
      id: sheet.id,
      name: sheet.name,
      visibility: sheet.visibility,
      cellCount: sheet.cellCount,
      usedRange: sheet.usedRange ?? null,
    })),
  };
}

function normalizeCalculationReport(report) {
  return {
    schemaVersion: report.schemaVersion,
    semanticRevision: BigInt(report.semanticRevision),
    formulaCount: report.formulaCount,
    valueCount: report.valueCount,
    unavailableCount: report.unavailableCount,
    materializedCellCount: report.materializedCellCount,
  };
}

function normalizeCalculationDelta(delta) {
  requireObject(delta, "calculation delta");
  return {
    schemaVersion: delta.schemaVersion,
    cursor: BigInt(delta.cursor),
    baseRevision: BigInt(delta.baseRevision),
    resultRevision: BigInt(delta.resultRevision),
    mode: delta.mode,
    reason: delta.reason,
    dirtyCount: delta.dirtyCount,
    evaluatedCount: delta.evaluatedCount,
    parsedFormulaCount: delta.parsedFormulaCount,
    changedCells: delta.changedCells.map((change) => ({
      cell: normalizeCellReference(change.cell),
      origin: change.origin,
      anchor:
        change.anchor === null || change.anchor === undefined
          ? null
          : normalizeCellReference(change.anchor),
      range: change.range ?? null,
      result: normalizeResult(change.result),
    })),
    removedMaterializedCells: delta.removedMaterializedCells.map(
      normalizeCellReference,
    ),
  };
}

function normalizeCalculationDeltaPage(page) {
  requireObject(page, "calculation delta page");
  return {
    schemaVersion: page.schemaVersion,
    requestedCursor: BigInt(page.requestedCursor),
    nextCursor:
      page.nextCursor === null || page.nextCursor === undefined
        ? null
        : BigInt(page.nextCursor),
    deltas: page.deltas.map(normalizeCalculationDelta),
  };
}

function normalizeEditReceipt(receipt) {
  requireObject(receipt, "edit receipt");
  return {
    schemaVersion: receipt.schemaVersion,
    baseRevision: BigInt(receipt.baseRevision),
    resultRevision: BigInt(receipt.resultRevision),
    appliedChangeCount: receipt.appliedChangeCount,
    changedCells: receipt.changedCells.map(normalizeCellReference),
    calculationChangedCells: receipt.calculationChangedCells.map(
      normalizeCellReference,
    ),
    createdSheetIds: receipt.createdSheetIds,
    topologyChanged: receipt.topologyChanged,
    calculationMetadataChanged: receipt.calculationMetadataChanged,
  };
}

function normalizeFunctionUsage(report) {
  return {
    schemaVersion: report.schemaVersion,
    formulaCount: report.formulaCount,
    parsedFormulaCount: report.parsedFormulaCount,
    unparsedFormulaCount: report.unparsedFormulaCount,
    entries: report.entries.map((entry) => ({
      name: entry.name,
      supported: entry.supported,
      callCount: entry.callCount,
      formulaCount: entry.formulaCount,
      sampleCells: entry.sampleCells.map(normalizeCellReference),
    })),
  };
}

function normalizeWriteReport(report) {
  return {
    schemaVersion: report.schemaVersion,
    complete: report.complete,
    policy: report.policy,
    materializedCount: report.materializedCount,
    invalidatedCells: report.invalidatedCells.map(normalizeCellReference),
    changedParts: report.changedParts,
    removedParts: report.removedParts,
    diagnosticCount: report.diagnosticCount,
  };
}

function normalizeCellReference(cell) {
  return {
    sheetId: cell.sheetId,
    sheetName: cell.sheetName,
    address: cell.address,
  };
}

module.exports = {
  normalizeCalculationDelta,
  normalizeCalculationDeltaPage,
  normalizeCalculationReport,
  normalizeEditReceipt,
  normalizeFunctionUsage,
  normalizeRangePage,
  normalizeSummary,
  normalizeWriteReport,
};
