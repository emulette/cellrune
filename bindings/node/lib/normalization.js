"use strict";

const { protocolError } = require("./errors.js");
const {
  requireObject,
  requireProtocolFinite,
  requireProtocolString,
} = require("./validation.js");

const INTEROP_SCHEMA_VERSION = 1;
const INTEROP_EDIT_SCHEMA_V2 = 2;
const DYNAMIC_KINDS = new Set(["offset", "indirect", "spill", "mixed"]);
const EXTERNAL_TARGET_KINDS = new Set([
  "reference",
  "defined_name",
  "structured_reference",
]);
const INVALID_REASONS = new Set([
  "parse_error",
  "circular_reference",
  "unresolved_name",
  "invalid_reference",
]);
const UNSUPPORTED_REASONS = new Set([
  "non_reference_expression",
  "context_dependent",
  "unsupported_expression",
]);

function requireProtocolVersion(value, name) {
  if (value !== INTEROP_SCHEMA_VERSION) {
    throw protocolError(`${name} schema version is unsupported`);
  }
}

function requireOptionalProtocolString(value, name) {
  if (value !== undefined && value !== null) {
    requireProtocolString(value, name);
  }
}

function requireProtocolEnum(value, values, name) {
  requireProtocolString(value, name);
  if (!values.has(value)) {
    throw protocolError(`${name} is unknown`);
  }
}

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

function normalizeDefinedNameInspection(report) {
  requireObject(report, "defined name inspection");
  requireProtocolVersion(report.schemaVersion, "defined name inspection");
  return {
    schemaVersion: report.schemaVersion,
    result: normalizeDefinedNameResult(report.result),
  };
}

function normalizeDefinedNameResult(result) {
  requireObject(result, "defined name inspection result");
  switch (result.kind) {
    case "rectangular":
      requireProtocolFinite(result.sheetId, "defined name sheet ID");
      requireProtocolString(result.sheetName, "defined name sheet name");
      requireProtocolString(result.range, "defined name range");
      return {
        kind: "rectangular",
        sheetId: result.sheetId,
        sheetName: result.sheetName,
        range: result.range,
      };
    case "three_dimensional":
      requireProtocolString(result.range, "defined name range");
      return {
        kind: "threeDimensional",
        sheetSpan: normalizeDefinedNameSheetSpan(result.sheetSpan),
        range: result.range,
      };
    case "non_rectangular":
      if (!Array.isArray(result.areas)) {
        throw protocolError("native defined name areas are missing");
      }
      return {
        kind: "nonRectangular",
        areas: result.areas.map(normalizeDefinedNameArea),
      };
    case "empty_reference":
      return { kind: "emptyReference" };
    case "dynamic_formula":
      requireProtocolEnum(
        result.dynamicKind,
        DYNAMIC_KINDS,
        "defined name dynamic kind",
      );
      requireProtocolString(result.formula, "defined name formula");
      return {
        kind: "dynamicFormula",
        dynamicKind: result.dynamicKind,
        formula: result.formula,
      };
    case "constant":
      requireProtocolString(result.formula, "defined name formula");
      return { kind: "constant", formula: result.formula };
    case "external_reference":
      requireOptionalProtocolString(result.locator, "external locator");
      requireProtocolString(result.workbook, "external workbook");
      requireOptionalProtocolString(result.sheet, "external sheet");
      requireOptionalProtocolString(result.sheetEnd, "external final sheet");
      requireProtocolEnum(
        result.targetKind,
        EXTERNAL_TARGET_KINDS,
        "external target kind",
      );
      requireProtocolString(result.targetText, "external target text");
      return {
        kind: "externalReference",
        locator: result.locator ?? null,
        workbook: result.workbook,
        sheet: result.sheet ?? null,
        sheetEnd: result.sheetEnd ?? null,
        targetKind: result.targetKind,
        targetText: result.targetText,
      };
    case "invalid":
      requireProtocolEnum(
        result.reason,
        INVALID_REASONS,
        "defined name invalid reason",
      );
      requireOptionalProtocolString(result.detail, "defined name invalid detail");
      return {
        kind: "invalid",
        reason: result.reason,
        detail: result.detail ?? null,
      };
    case "unsupported":
      requireProtocolEnum(
        result.reason,
        UNSUPPORTED_REASONS,
        "defined name unsupported reason",
      );
      requireOptionalProtocolString(
        result.detail,
        "defined name unsupported detail",
      );
      return {
        kind: "unsupported",
        reason: result.reason,
        detail: result.detail ?? null,
      };
    case "not_found":
      return { kind: "notFound" };
    default:
      throw protocolError("native defined name result kind is unknown");
  }
}

function normalizeDefinedNameArea(area) {
  requireObject(area, "defined name reference area");
  requireProtocolString(area.range, "defined name area range");
  if (area.kind === "rectangular") {
    requireProtocolFinite(area.sheetId, "defined name area sheet ID");
    requireProtocolString(area.sheetName, "defined name area sheet name");
    return {
      kind: "rectangular",
      sheetId: area.sheetId,
      sheetName: area.sheetName,
      range: area.range,
    };
  }
  if (area.kind === "three_dimensional") {
    return {
      kind: "threeDimensional",
      sheetSpan: normalizeDefinedNameSheetSpan(area.sheetSpan),
      range: area.range,
    };
  }
  throw protocolError("native defined name area kind is unknown");
}

function normalizeDefinedNameSheetSpan(span) {
  requireObject(span, "defined name sheet span");
  requireProtocolFinite(span.startSheetId, "defined name start sheet ID");
  requireProtocolString(span.startSheetName, "defined name start sheet name");
  requireProtocolFinite(span.endSheetId, "defined name end sheet ID");
  requireProtocolString(span.endSheetName, "defined name end sheet name");
  return {
    startSheetId: span.startSheetId,
    startSheetName: span.startSheetName,
    endSheetId: span.endSheetId,
    endSheetName: span.endSheetName,
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
      mergedRanges: sheet.mergedRanges,
      tables: sheet.tables.map((table) => ({
        id: table.id,
        name: table.name,
        displayName: table.displayName,
        range: table.range,
        headerRowCount: table.headerRowCount,
        totalsRowCount: table.totalsRowCount,
        columns: table.columns.map((column) => ({
          id: column.id,
          name: column.name,
          totalsRowFunction: column.totalsRowFunction ?? null,
        })),
      })),
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

function normalizeEditReceiptV2(receipt) {
  requireObject(receipt, "edit receipt v2");
  if (receipt.schemaVersion !== INTEROP_EDIT_SCHEMA_V2) {
    throw protocolError("edit receipt v2 schema version is unsupported");
  }
  if (
    !Array.isArray(receipt.changedTableIds) ||
    receipt.changedTableIds.some(
      (tableId) =>
        typeof tableId !== "number" ||
        !Number.isInteger(tableId) ||
        tableId <= 0 ||
        tableId > 4294967295,
    )
  ) {
    throw protocolError("edit receipt v2 table IDs are malformed");
  }
  return {
    ...normalizeEditReceipt(receipt),
    changedTableIds: receipt.changedTableIds,
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
  normalizeDefinedNameInspection,
  normalizeEditReceipt,
  normalizeEditReceiptV2,
  normalizeFunctionUsage,
  normalizeRangePage,
  normalizeSummary,
  normalizeWriteReport,
};
