"use strict";

const native = require("./native.js");
const {
  serializeWorkbookChange,
  serializeWorkbookChangeV2,
} = require("./lib/changes.js");
const {
  CellRuneError,
  INPUT_DETAIL,
  closedError,
  inputError,
  withErrors,
  withSyncErrors,
} = require("./lib/errors.js");
const {
  normalizeCalculationDelta,
  normalizeCalculationDeltaPage,
  normalizeCalculationReport,
  normalizeDefinedNameInspection,
  normalizeEditReceipt,
  normalizeEditReceiptV2,
  normalizeFunctionCatalog,
  normalizeFunctionUsage,
  normalizePreviewChanges,
  normalizeRangePage,
  normalizeSummary,
  normalizeTransactionImpactPage,
  normalizeTransactionReceipt,
  normalizeWriteReport,
  parsePreviewJson,
} = require("./lib/normalization.js");
const {
  requireFinite,
  requireNonNegativeInteger,
  requireOptionalBoolean,
  requireOptionalFinite,
  requireOptionalString,
  requireOptionKeys,
  requireOptions,
  requireString,
  requireU64BigInt,
} = require("./lib/validation.js");

class Workbook {
  #native;

  constructor(nativeWorkbook) {
    this.#native = nativeWorkbook;
  }

  static create() {
    return new Workbook(native.createWorkbook());
  }

  static async openPath(path) {
    requireString(path, "path");
    return new Workbook(await withErrors(native.openWorkbookPath(path)));
  }

  static async fromBytes(bytes) {
    if (!(bytes instanceof Uint8Array)) {
      throw inputError("bytes must be a Buffer or Uint8Array");
    }
    return new Workbook(
      await withErrors(native.openWorkbookBytes(Buffer.from(bytes))),
    );
  }

  get closed() {
    return this.#native === null;
  }

  close() {
    const session = this.#native;
    if (session === null) {
      return;
    }
    withSyncErrors(() => session.close());
    this.#native = null;
  }

  summary() {
    return normalizeSummary(withSyncErrors(() => this.#session().summary()));
  }

  readRange(sheet, start, end, options = {}) {
    requireString(sheet, "sheet");
    requireString(start, "start");
    requireString(end, "end");
    requireOptions(options);
    const offset = options.offset ?? 0;
    const limit = options.limit ?? 0;
    requireNonNegativeInteger(offset, "offset");
    requireNonNegativeInteger(limit, "limit");
    return normalizeRangePage(
      withSyncErrors(() =>
        this.#session().readRange(sheet, start, end, offset, limit),
      ),
    );
  }

  inspectDefinedName(name, options = {}) {
    requireString(name, "name");
    requireOptions(options);
    requireOptionKeys(options, ["currentSheet"]);
    requireOptionalString(options.currentSheet, "currentSheet");
    return normalizeDefinedNameInspection(
      withSyncErrors(() =>
        this.#session().inspectDefinedName(
          name,
          options.currentSheet ?? null,
        ),
      ),
    );
  }

  functionUsage() {
    return normalizeFunctionUsage(
      withSyncErrors(() => this.#session().functionUsage()),
    );
  }

  async calculate(options = {}) {
    requireOptions(options);
    requireOptionalFinite(options.todaySerial, "todaySerial");
    requireOptionalFinite(options.nowSerial, "nowSerial");
    requireOptionalString(options.arithmeticSemantics, "arithmeticSemantics");
    requireOptionalString(
      options.financialSolverSemantics,
      "financialSolverSemantics",
    );
    const task = withSyncErrors(() =>
      this.#session().calculate(
        options.todaySerial ?? null,
        options.nowSerial ?? null,
        options.arithmeticSemantics ?? null,
        options.financialSolverSemantics ?? null,
      ),
    );
    return normalizeCalculationReport(await withErrors(task));
  }

  async recalculate(options = {}) {
    requireOptions(options);
    const mode = options.mode ?? "auto";
    if (!["auto", "incremental", "full"].includes(mode)) {
      throw inputError("mode must be auto, incremental, or full");
    }
    requireOptionalFinite(options.todaySerial, "todaySerial");
    requireOptionalFinite(options.nowSerial, "nowSerial");
    requireOptionalString(options.arithmeticSemantics, "arithmeticSemantics");
    requireOptionalString(
      options.financialSolverSemantics,
      "financialSolverSemantics",
    );
    const task = withSyncErrors(() =>
      this.#session().recalculate(
        mode,
        options.todaySerial ?? null,
        options.nowSerial ?? null,
        options.arithmeticSemantics ?? null,
        options.financialSolverSemantics ?? null,
      ),
    );
    return normalizeCalculationDelta(await withErrors(task));
  }

  async previewChanges(expectedRevision, changes, options = {}) {
    requireU64BigInt(expectedRevision, "expectedRevision");
    if (!Array.isArray(changes) || changes.length === 0) {
      throw inputError("changes must be a non-empty array");
    }
    requireOptions(options);
    requireOptionKeys(options, [
      "mode",
      "todaySerial",
      "nowSerial",
      "arithmeticSemantics",
      "financialSolverSemantics",
    ]);
    const mode = options.mode ?? "auto";
    if (!["auto", "incremental", "full"].includes(mode)) {
      throw inputError("mode must be auto, incremental, or full");
    }
    requireOptionalFinite(options.todaySerial, "todaySerial");
    requireOptionalFinite(options.nowSerial, "nowSerial");
    requireOptionalString(options.arithmeticSemantics, "arithmeticSemantics");
    requireOptionalString(
      options.financialSolverSemantics,
      "financialSolverSemantics",
    );
    const payload = {
      changes: changes.map((change, index) =>
        serializeWorkbookChangeV2(change, index),
      ),
    };
    const task = withSyncErrors(() =>
      this.#session().previewChanges(
        expectedRevision.toString(),
        JSON.stringify(payload),
        JSON.stringify({
          mode,
          today_serial: options.todaySerial ?? null,
          now_serial: options.nowSerial ?? null,
          arithmetic_semantics: options.arithmeticSemantics ?? "excel_near_zero",
          financial_solver_semantics:
            options.financialSolverSemantics ?? "excel_iteration_budget",
        }),
      ),
    );
    return normalizePreviewChanges(parsePreviewJson(await withErrors(task)));
  }

  previewChangesPage(previewId, options = {}) {
    requireU64BigInt(previewId, "previewId");
    requireOptions(options);
    requireOptionKeys(options, ["section", "cursor", "limit"]);
    requireString(options.section, "section");
    if (
      ![
        "affected",
        "evaluated",
        "preview_results",
        "preview_issues",
        "install_results",
      ].includes(options.section)
    ) {
      throw inputError(INPUT_DETAIL.PREVIEW_SECTION_INVALID);
    }
    const limit = options.limit ?? 0;
    requireNonNegativeInteger(limit, "limit");
    const cursor = options.cursor ?? null;
    const cursorJson = cursor === null ? null : JSON.stringify(previewCursorWire(cursor));
    return normalizeTransactionImpactPage(
      parsePreviewJson(
        withSyncErrors(() =>
          this.#session().previewChangesPage(
            previewId.toString(),
            options.section,
            cursorJson,
            limit,
          ),
        ),
      ),
    );
  }

  commitPreview(previewId) {
    requireU64BigInt(previewId, "previewId");
    return normalizeTransactionReceipt(
      parsePreviewJson(
        withSyncErrors(() => this.#session().commitPreview(previewId.toString())),
      ),
    );
  }

  discardPreview(previewId) {
    requireU64BigInt(previewId, "previewId");
    return withSyncErrors(() => this.#session().discardPreview(previewId.toString()));
  }

  applyChanges(expectedRevision, changes) {
    requireU64BigInt(expectedRevision, "expectedRevision");
    if (!Array.isArray(changes) || changes.length === 0) {
      throw inputError("changes must be a non-empty array");
    }
    const payload = {
      changes: changes.map((change, index) =>
        serializeWorkbookChange(change, index),
      ),
    };
    return normalizeEditReceipt(
      withSyncErrors(() =>
        this.#session().applyChanges(
          expectedRevision.toString(),
          JSON.stringify(payload),
        ),
      ),
    );
  }

  applyChangesV2(expectedRevision, changes) {
    requireU64BigInt(expectedRevision, "expectedRevision");
    if (!Array.isArray(changes) || changes.length === 0) {
      throw inputError("changes must be a non-empty array");
    }
    const payload = {
      changes: changes.map((change, index) =>
        serializeWorkbookChangeV2(change, index),
      ),
    };
    return normalizeEditReceiptV2(
      withSyncErrors(() =>
        this.#session().applyChangesV2(
          expectedRevision.toString(),
          JSON.stringify(payload),
        ),
      ),
    );
  }

  changesSince(cursor = 0n, options = {}) {
    requireU64BigInt(cursor, "cursor");
    requireOptions(options);
    const limit = options.limit ?? 0;
    requireNonNegativeInteger(limit, "limit");
    return normalizeCalculationDeltaPage(
      withSyncErrors(() =>
        this.#session().changesSince(cursor.toString(), limit),
      ),
    );
  }

  cancelCalculation() {
    return withSyncErrors(() => this.#session().cancelCalculation());
  }

  calculationActive() {
    return withSyncErrors(() => this.#session().calculationActive());
  }

  setBlank(sheet, address) {
    this.#edit(sheet, address, () => this.#session().setBlank(sheet, address));
  }

  setNumber(sheet, address, value) {
    requireFinite(value, "value");
    this.#edit(sheet, address, () =>
      this.#session().setNumber(sheet, address, value),
    );
  }

  setText(sheet, address, value) {
    requireString(value, "value");
    this.#edit(sheet, address, () =>
      this.#session().setText(sheet, address, value),
    );
  }

  setLogical(sheet, address, value) {
    if (typeof value !== "boolean") {
      throw inputError("value must be a boolean");
    }
    this.#edit(sheet, address, () =>
      this.#session().setLogical(sheet, address, value),
    );
  }

  setError(sheet, address, value) {
    requireString(value, "value");
    this.#edit(sheet, address, () =>
      this.#session().setError(sheet, address, value),
    );
  }

  setFormula(sheet, address, formula, options = {}) {
    requireString(formula, "formula");
    requireOptions(options);
    if (options.dynamicRange !== undefined) {
      requireString(options.dynamicRange, "dynamicRange");
    }
    this.#edit(sheet, address, () =>
      this.#session().setFormula(
        sheet,
        address,
        formula,
        options.dynamicRange ?? null,
      ),
    );
  }

  clearCell(sheet, address) {
    requireString(sheet, "sheet");
    requireString(address, "address");
    return withSyncErrors(() => this.#session().clearCell(sheet, address));
  }

  addSheet(name) {
    requireString(name, "name");
    return withSyncErrors(() => this.#session().addSheet(name));
  }

  renameSheet(currentName, newName) {
    requireString(currentName, "currentName");
    requireString(newName, "newName");
    withSyncErrors(() => this.#session().renameSheet(currentName, newName));
  }

  async toBytes(options = {}) {
    requireOptions(options);
    requireOptionalBoolean(
      options.invalidateUnavailable,
      "invalidateUnavailable",
    );
    const task = withSyncErrors(() =>
      this.#session().toBytes(options.invalidateUnavailable ?? false),
    );
    return withErrors(task);
  }

  async save(path, options = {}) {
    requireString(path, "path");
    requireOptions(options);
    requireOptionalBoolean(
      options.invalidateUnavailable,
      "invalidateUnavailable",
    );
    requireOptionalBoolean(options.replaceExisting, "replaceExisting");
    const task = withSyncErrors(() =>
      this.#session().savePath(
        path,
        options.invalidateUnavailable ?? false,
        options.replaceExisting ?? false,
      ),
    );
    return normalizeWriteReport(await withErrors(task));
  }

  #session() {
    if (this.#native === null) {
      throw closedError();
    }
    return this.#native;
  }

  #edit(sheet, address, operation) {
    requireString(sheet, "sheet");
    requireString(address, "address");
    withSyncErrors(operation);
  }
}

function previewCursorWire(cursor) {
  if (cursor === null || typeof cursor !== "object") {
    throw inputError(INPUT_DETAIL.PREVIEW_CURSOR_INVALID);
  }
  requireU64BigInt(cursor.previewId, "cursor.previewId");
  requireString(cursor.token, "cursor.token");
  return { preview_id: cursor.previewId.toString(), token: cursor.token };
}

function functionCatalog() {
  return normalizeFunctionCatalog(withSyncErrors(() => native.functionCatalog()));
}

module.exports = {
  SCHEMA_VERSION: native.schemaVersion(),
  CellRuneError,
  Workbook,
  functionCatalog,
};
