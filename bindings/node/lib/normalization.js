"use strict";

const { PROTOCOL_DETAIL, protocolError } = require("./errors.js");
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
const PROTOCOL_DETAILS = Object.freeze({
  FUNCTION_CATALOG_ENTRIES: "native function catalog entries are missing",
  FUNCTION_CATALOG_ENTRY: "native function catalog entry is malformed",
});

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
    fingerprint: {
      schemaVersion: summary.fingerprint.schemaVersion,
      digestHex: summary.fingerprint.digestHex,
    },
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

function normalizeFunctionCatalog(report) {
  requireObject(report, "function catalog");
  requireProtocolVersion(report.schemaVersion, "function catalog");
  if (!Array.isArray(report.entries)) {
    throw protocolError(PROTOCOL_DETAILS.FUNCTION_CATALOG_ENTRIES);
  }
  return {
    schemaVersion: report.schemaVersion,
    entries: report.entries.map((entry) => {
      requireObject(entry, "function catalog entry");
      requireProtocolString(entry.name, "function catalog name");
      requireProtocolString(
        entry.canonicalName,
        "function catalog canonical name",
      );
      if (
        typeof entry.alias !== "boolean" ||
        typeof entry.returnsArray !== "boolean" ||
        typeof entry.official !== "boolean"
      ) {
        throw protocolError(PROTOCOL_DETAILS.FUNCTION_CATALOG_ENTRY);
      }
      return {
        name: entry.name,
        canonicalName: entry.canonicalName,
        alias: entry.alias,
        returnsArray: entry.returnsArray,
        official: entry.official,
      };
    }),
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
    outputSha256: report.outputSha256,
  };
}

function normalizeCellReference(cell) {
  return {
    sheetId: cell.sheetId,
    sheetName: cell.sheetName,
    address: cell.address,
  };
}

function parsePreviewJson(value) {
  if (typeof value !== "string") {
    throw protocolError(PROTOCOL_DETAIL.PREVIEW_PAYLOAD_MALFORMED);
  }
  try {
    return JSON.parse(value);
  } catch {
    throw protocolError(PROTOCOL_DETAIL.PREVIEW_PAYLOAD_JSON_INVALID);
  }
}

function normalizePreviewChanges(value) {
  requireObject(value, "preview changes");
  requireProtocolVersion(value.schema_version, "preview changes");
  return {
    schemaVersion: value.schema_version,
    previewId: BigInt(value.preview_id),
    report: normalizeTransactionReport(value.report),
  };
}

function normalizeTransactionReport(report) {
  requireObject(report, "transaction report");
  return {
    contractVersion: report.contract_version,
    baseRevision: BigInt(report.base_revision),
    resultRevision: BigInt(report.result_revision),
    baseFingerprint: normalizeFingerprint(report.base_fingerprint),
    resultFingerprint: normalizeFingerprint(report.result_fingerprint),
    inputSha256: report.input_sha256 ?? null,
    calculatorProvider: {
      name: report.calculator_provider.name,
      version: report.calculator_provider.version,
    },
    calculationOptions: normalizeTransactionOptions(report.calculation_options),
    baseCalculationReused: report.base_calculation_reused,
    baseExecutionMode: report.base_execution_mode,
    baseDecisionReason: report.base_decision_reason,
    candidateRequestedMode: report.candidate_requested_mode,
    candidateExecutionMode: report.candidate_execution_mode,
    candidateDecisionReason: report.candidate_decision_reason,
    editReceipt: normalizeTransactionEditReceipt(report.edit_receipt),
    impactCoverage: report.impact_coverage,
    directAffectedCount: report.direct_affected_count,
    transitiveAffectedCount: report.transitive_affected_count,
    conservativeAffectedCount: report.conservative_affected_count,
    baseEvaluatedCount: report.base_evaluated_count,
    candidateEvaluatedCount: report.candidate_evaluated_count,
    parsedFormulaCount: report.parsed_formula_count,
    functionIterationCount: report.function_iteration_count,
    referenceCellCount: report.reference_cell_count,
    previewChangedCount: report.preview_changed_count,
    previewRemovedCount: report.preview_removed_count,
    introducedIssueCount: report.introduced_issue_count,
    resolvedIssueCount: report.resolved_issue_count,
    changedIssueCount: report.changed_issue_count,
    installDeltaCount: report.install_delta_count,
    installedCalculationRevision:
      report.installed_calculation_revision == null
        ? null
        : BigInt(report.installed_calculation_revision),
    installedCalculationFingerprint:
      report.installed_calculation_fingerprint == null
        ? null
        : normalizeFingerprint(report.installed_calculation_fingerprint),
    installedCalculationOptions:
      report.installed_calculation_options == null
        ? null
        : normalizeTransactionOptions(report.installed_calculation_options),
    installDeltaBasisDiffersFromPreviewBase:
      report.install_delta_basis_differs_from_preview_base,
    installDeltaBasisReasons: report.install_delta_basis_reasons,
    detailCounts: {
      affected: report.detail_counts.affected,
      evaluated: report.detail_counts.evaluated,
      previewResults: report.detail_counts.preview_results,
      previewIssues: report.detail_counts.preview_issues,
      installResults: report.detail_counts.install_results,
    },
  };
}

function normalizeTransactionOptions(options) {
  return {
    todaySerial: options.today_serial ?? null,
    nowSerial: options.now_serial ?? null,
    arithmeticSemantics: options.arithmetic_semantics,
    financialSolverSemantics: options.financial_solver_semantics,
    limits: {
      maxFormulaTokens: options.limits.max_formula_tokens,
      maxFormulaSourceBytes: options.limits.max_formula_source_bytes,
      maxFormulaAstNodes: options.limits.max_formula_ast_nodes,
      maxFormulaNestingDepth: options.limits.max_formula_nesting_depth,
      maxDependencyEdges: options.limits.max_dependency_edges,
      maxReferenceAreas: options.limits.max_reference_areas,
      maxArrayCells: options.limits.max_array_cells,
      maxTextBytes: options.limits.max_text_bytes,
      maxFunctionIterations: options.limits.max_function_iterations,
      maxLetBindings: options.limits.max_let_bindings,
      maxLambdaDepth: options.limits.max_lambda_depth,
      maxLambdaInvocations: options.limits.max_lambda_invocations,
    },
  };
}

function normalizeFingerprint(value) {
  return { schemaVersion: value.schema_version, digestHex: value.digest_hex };
}

function normalizeTransactionEditReceipt(receipt) {
  return {
    schemaVersion: receipt.schema_version,
    baseRevision: BigInt(receipt.base_revision),
    resultRevision: BigInt(receipt.result_revision),
    appliedChangeCount: receipt.applied_change_count,
    changedCells: receipt.changed_cells.map(normalizeTransactionCellReference),
    calculationChangedCells: receipt.calculation_changed_cells.map(
      normalizeTransactionCellReference,
    ),
    createdSheetIds: receipt.created_sheet_ids,
    topologyChanged: receipt.topology_changed,
    calculationMetadataChanged: receipt.calculation_metadata_changed,
  };
}

function normalizeTransactionPage(value) {
  requireObject(value, "transaction detail page");
  requireProtocolVersion(value.schema_version, "transaction detail page");
  return {
    schemaVersion: value.schema_version,
    previewId: BigInt(value.preview_id),
    section: value.section,
    items: value.items.map(normalizeTransactionDetail),
    nextCursor:
      value.next_cursor == null
        ? null
        : { previewId: BigInt(value.next_cursor.preview_id), token: value.next_cursor.token },
    totalCount: value.total_count,
  };
}

function normalizeTransactionDetail(item) {
  const cell = item.cell ? normalizeTransactionCellReference(item.cell) : null;
  switch (item.kind) {
    case "affected":
      return { kind: item.kind, cell, cause: item.cause };
    case "evaluated":
      return { kind: item.kind, cell };
    case "preview_result":
      return {
        kind: item.kind,
        cell,
        previousOrigin: normalizeTransactionOrigin(item.previous_origin),
        previousResult: normalizeTransactionResult(item.previous_result),
        resultOrigin: normalizeTransactionOrigin(item.result_origin),
        result: normalizeTransactionResult(item.result),
      };
    case "preview_issue":
      return {
        kind: item.kind,
        cell,
        changeKind: item.change_kind,
        previous: item.previous ?? null,
        current: item.current ?? null,
      };
    case "install_result":
      return {
        kind: item.kind,
        cell,
        origin: normalizeTransactionOrigin(item.origin),
        result: normalizeTransactionResult(item.result),
      };
    case "unknown":
      return { kind: item.kind };
    default:
      throw protocolError(PROTOCOL_DETAIL.TRANSACTION_DETAIL_KIND_UNKNOWN);
  }
}

function normalizeTransactionCellReference(value) {
  return {
    sheetId: value.sheet_id,
    sheetName: value.sheet_name,
    address: value.address,
  };
}

function normalizeTransactionOrigin(origin) {
  if (origin == null) {
    return null;
  }
  return {
    kind: origin.kind,
    anchor:
      origin.anchor == null ? null : normalizeTransactionCellReference(origin.anchor),
    range: origin.range ?? null,
  };
}

function normalizeTransactionResult(result) {
  if (result == null) {
    return null;
  }
  if (result.kind === "value") {
    return { kind: "value", value: result.value };
  }
  if (result.kind === "unavailable") {
    return {
      kind: "unavailable",
      code: result.code,
      message: result.message,
      detail: result.detail ?? null,
    };
  }
  throw protocolError(PROTOCOL_DETAIL.TRANSACTION_RESULT_MALFORMED);
}

function normalizeTransactionReceipt(value) {
  requireObject(value, "transaction receipt");
  requireProtocolVersion(value.schema_version, "transaction receipt");
  return {
    schemaVersion: value.schema_version,
    edit: normalizeTransactionEditReceipt(value.edit),
    calculationDelta: normalizeTransactionCalculationDelta(value.calculation_delta),
    baseFingerprint: normalizeFingerprint(value.base_fingerprint),
    resultFingerprint: normalizeFingerprint(value.result_fingerprint),
  };
}

function normalizeTransactionCalculationDelta(delta) {
  return {
    schemaVersion: delta.schema_version,
    cursor: BigInt(delta.cursor),
    baseRevision: BigInt(delta.base_revision),
    resultRevision: BigInt(delta.result_revision),
    mode: delta.mode,
    reason: delta.reason,
    dirtyCount: delta.dirty_count,
    evaluatedCount: delta.evaluated_count,
    parsedFormulaCount: delta.parsed_formula_count,
    changedCells: delta.changed_cells.map((change) => ({
      cell: normalizeTransactionCellReference(change.cell),
      origin: change.origin,
      anchor:
        change.anchor == null ? null : normalizeTransactionCellReference(change.anchor),
      range: change.range ?? null,
      result: normalizeTransactionResult(change.result),
    })),
    removedMaterializedCells: delta.removed_materialized_cells.map(
      normalizeTransactionCellReference,
    ),
  };
}

module.exports = {
  normalizeCalculationDelta,
  normalizeCalculationDeltaPage,
  normalizeCalculationReport,
  normalizeDefinedNameInspection,
  normalizeEditReceipt,
  normalizeEditReceiptV2,
  normalizeFunctionCatalog,
  normalizeFunctionUsage,
  normalizeRangePage,
  normalizeSummary,
  normalizePreviewChanges,
  normalizeTransactionImpactPage: normalizeTransactionPage,
  normalizeTransactionReceipt,
  parsePreviewJson,
  normalizeWriteReport,
};
