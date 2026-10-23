"use strict";

const native = require("./native.js");
const { normalizeCellFormatPage } = require("./lib/formats.js");
const {
  normalizeCommentPage,
  normalizeDataValidationPage,
  normalizeHyperlinkPage,
} = require("./lib/sheet_metadata.js");
const { requireOptionalAbortSignal, runAbortable } = require("./lib/abort.js");
const {
  normalizeDependentPage,
  normalizePrecedentPage,
} = require("./lib/dependencies.js");
const { normalizePhoneticPage } = require("./lib/presentation.js");
const { parseRangeValues, serializeRangeRows } = require("./lib/values.js");
const { targetRequest } = require("./lib/targeted.js");
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
  normalizeTargetCalculation,
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
  normalizeEditWriteReport,
  normalizeCapabilityPage,
  parsePreviewJson,
} = require("./lib/normalization.js");
const {
  limitsJson,
  localeProfileWire,
  requireFinite,
  requireNonNegativeInteger,
  requireOptionalBoolean,
  requireOptionalFinite,
  requireOptionalLocaleProfile,
  requireOptionalString,
  requireOptionalU32,
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

  static async openPath(path, options = {}) {
    requireString(path, "path");
    requireOptions(options);
    requireOptionKeys(options, ["readLimits"]);
    const limits = limitsJson(options.readLimits, "readLimits");
    const task = withSyncErrors(() => native.openWorkbookPath(path, limits));
    return new Workbook(await withErrors(task));
  }

  static fromCsv(bytes, options = {}) {
    if (!(bytes instanceof Uint8Array)) {
      throw inputError("bytes must be a Buffer or Uint8Array");
    }
    requireOptions(options);
    requireOptionKeys(options, ["sheetName", "delimiter"]);
    requireOptionalString(options.sheetName, "sheetName");
    requireOptionalString(options.delimiter, "delimiter");
    return new Workbook(
      withSyncErrors(() =>
        native.openWorkbookCsv(
          Buffer.from(bytes),
          options.sheetName ?? null,
          options.delimiter ?? null,
        ),
      ),
    );
  }

  toCsv(sheet, options = {}) {
    requireString(sheet, "sheet");
    requireOptions(options);
    requireOptionKeys(options, ["delimiter"]);
    requireOptionalString(options.delimiter, "delimiter");
    return withSyncErrors(() =>
      this.#session().toCsv(sheet, options.delimiter ?? null),
    );
  }

  static async fromBytes(bytes, options = {}) {
    if (!(bytes instanceof Uint8Array)) {
      throw inputError("bytes must be a Buffer or Uint8Array");
    }
    requireOptions(options);
    requireOptionKeys(options, ["readLimits"]);
    const limits = limitsJson(options.readLimits, "readLimits");
    const task = withSyncErrors(() =>
      native.openWorkbookBytes(Buffer.from(bytes), limits),
    );
    return new Workbook(await withErrors(task));
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
    for (const name of ["omitEmpty", "valuesOnly", "compact"]) {
      requireOptionalBoolean(options[name], name);
    }
    if (options.maxBytes !== undefined) {
      requireNonNegativeInteger(options.maxBytes, "maxBytes");
    }
    const forms = {
      omitEmpty: options.omitEmpty,
      valuesOnly: options.valuesOnly,
      compact: options.compact,
      maxBytes: options.maxBytes,
    };
    return normalizeRangePage(
      withSyncErrors(() =>
        this.#session().readRange(sheet, start, end, offset, limit, forms),
      ),
    );
  }

  readFormats(sheet, start, end, options = {}) {
    requireString(start, "start");
    requireString(end, "end");
    const { offset, limit } = sheetPage(sheet, options);
    return normalizeCellFormatPage(
      withSyncErrors(() =>
        this.#session().readFormats(sheet, start, end, offset, limit),
      ),
    );
  }

  readValues(sheet, start, end) {
    requireString(sheet, "sheet");
    requireString(start, "start");
    requireString(end, "end");
    return parseRangeValues(
      withSyncErrors(() => this.#session().readValues(sheet, start, end)),
    );
  }

  writeValues(sheet, start, rows) {
    requireString(sheet, "sheet");
    requireString(start, "start");
    const request = { sheet, start, rows: serializeRangeRows(rows) };
    return normalizeEditReceipt(
      withSyncErrors(() => this.#session().writeValues(JSON.stringify(request))),
    );
  }

  precedents(sheet, address, options = {}) {
    requireString(address, "address");
    const { offset, limit } = sheetPage(sheet, options);
    return normalizePrecedentPage(
      withSyncErrors(() => this.#session().precedents(sheet, address, offset, limit)),
    );
  }

  dependents(sheet, address, options = {}) {
    requireString(address, "address");
    const { offset, limit } = sheetPage(sheet, options);
    return normalizeDependentPage(
      withSyncErrors(() => this.#session().dependents(sheet, address, offset, limit)),
    );
  }

  phonetics(sheet, options = {}) {
    const { offset, limit } = sheetPage(sheet, options);
    return normalizePhoneticPage(
      withSyncErrors(() => this.#session().phonetics(sheet, offset, limit)),
    );
  }

  hyperlinks(sheet, options = {}) {
    const { offset, limit } = sheetPage(sheet, options);
    return normalizeHyperlinkPage(
      withSyncErrors(() => this.#session().hyperlinks(sheet, offset, limit)),
    );
  }

  comments(sheet, options = {}) {
    const { offset, limit } = sheetPage(sheet, options);
    return normalizeCommentPage(
      withSyncErrors(() => this.#session().comments(sheet, offset, limit)),
    );
  }

  dataValidations(sheet, options = {}) {
    const { offset, limit } = sheetPage(sheet, options);
    return normalizeDataValidationPage(
      withSyncErrors(() => this.#session().dataValidations(sheet, offset, limit)),
    );
  }

  async scanCapabilities(options = {}) {
    requireOptions(options);
    requireOptionKeys(options, ["offset", "limit", "todaySerial", "nowSerial"]);
    const offset = options.offset ?? 0;
    const limit = options.limit ?? 0;
    requireNonNegativeInteger(offset, "offset");
    requireNonNegativeInteger(limit, "limit");
    requireOptionalFinite(options.todaySerial, "todaySerial");
    requireOptionalFinite(options.nowSerial, "nowSerial");
    const task = withSyncErrors(() =>
      this.#session().scanCapabilities(
        offset,
        limit,
        options.todaySerial ?? null,
        options.nowSerial ?? null,
      ),
    );
    return normalizeCapabilityPage(await withErrors(task));
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
    requireOptionalU32(options.randomSeed, "randomSeed");
    requireOptionalLocaleProfile(options.localeProfile, "localeProfile");
    requireOptionalString(options.arithmeticSemantics, "arithmeticSemantics");
    requireOptionalString(
      options.financialSolverSemantics,
      "financialSolverSemantics",
    );
    requireOptionalAbortSignal(options.signal);
    const calculation = calculationOptions(options);
    const limits = limitsJson(options.calculationLimits, "calculationLimits");
    const session = this.#session();
    const report = await runAbortable(options.signal, (cancellation) =>
      session.calculate(calculation, limits, cancellation),
    );
    return normalizeCalculationReport(report);
  }

  async calculateTargets(targets, options = {}) {
    requireOptions(options);
    const { signal, ...targetOptions } = options;
    requireOptionalAbortSignal(signal);
    const request = targetRequest(targets, targetOptions);
    const session = this.#session();
    const response = await runAbortable(signal, (cancellation) =>
      session.calculateTargets(request, cancellation),
    );
    return normalizeTargetCalculation(parsePreviewJson(response));
  }

  async recalculate(options = {}) {
    requireOptions(options);
    const mode = options.mode ?? "auto";
    if (!["auto", "incremental", "full"].includes(mode)) {
      throw inputError("mode must be auto, incremental, or full");
    }
    requireOptionalFinite(options.todaySerial, "todaySerial");
    requireOptionalFinite(options.nowSerial, "nowSerial");
    requireOptionalU32(options.randomSeed, "randomSeed");
    requireOptionalLocaleProfile(options.localeProfile, "localeProfile");
    requireOptionalString(options.arithmeticSemantics, "arithmeticSemantics");
    requireOptionalString(
      options.financialSolverSemantics,
      "financialSolverSemantics",
    );
    requireOptionalAbortSignal(options.signal);
    const calculation = calculationOptions(options);
    const limits = limitsJson(options.calculationLimits, "calculationLimits");
    const session = this.#session();
    const delta = await runAbortable(options.signal, (cancellation) =>
      session.recalculate(mode, calculation, limits, cancellation),
    );
    return normalizeCalculationDelta(delta);
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
      "randomSeed",
      "localeProfile",
      "arithmeticSemantics",
      "financialSolverSemantics",
      "calculationLimits",
      "signal",
    ]);
    requireOptionalAbortSignal(options.signal);
    const mode = options.mode ?? "auto";
    if (!["auto", "incremental", "full"].includes(mode)) {
      throw inputError("mode must be auto, incremental, or full");
    }
    requireOptionalFinite(options.todaySerial, "todaySerial");
    requireOptionalFinite(options.nowSerial, "nowSerial");
    requireOptionalU32(options.randomSeed, "randomSeed");
    requireOptionalLocaleProfile(options.localeProfile, "localeProfile");
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
    const previewOptions = JSON.stringify({
      mode,
      today_serial: options.todaySerial ?? null,
      now_serial: options.nowSerial ?? null,
      random_seed: options.randomSeed ?? null,
      locale_profile: localeProfileWire(options.localeProfile),
      arithmetic_semantics: options.arithmeticSemantics ?? "excel_near_zero",
      financial_solver_semantics:
        options.financialSolverSemantics ?? "excel_iteration_budget",
    });
    const limits = limitsJson(options.calculationLimits, "calculationLimits");
    const session = this.#session();
    const preview = await runAbortable(options.signal, (cancellation) =>
      session.previewChanges(
        expectedRevision.toString(),
        JSON.stringify(payload),
        previewOptions,
        limits,
        cancellation,
      ),
    );
    return normalizePreviewChanges(parsePreviewJson(preview));
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

  deleteSheet(name) {
    requireString(name, "name");
    withSyncErrors(() => this.#session().deleteSheet(name));
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

  async editsToBytes() {
    const task = withSyncErrors(() => this.#session().editsToBytes());
    return withErrors(task);
  }

  async saveEdits(path, options = {}) {
    requireString(path, "path");
    requireOptions(options);
    requireOptionalBoolean(options.replaceExisting, "replaceExisting");
    const task = withSyncErrors(() =>
      this.#session().saveEditsPath(path, options.replaceExisting ?? false),
    );
    return normalizeEditWriteReport(await withErrors(task));
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

/**
 * Picks the validated calculation options native code reads for calculate and recalculate. They
 * pass as JavaScript values rather than JSON so date-time serials keep every bit.
 */
function sheetPage(sheet, options) {
  requireString(sheet, "sheet");
  requireOptions(options);
  requireOptionKeys(options, ["offset", "limit"]);
  const offset = options.offset ?? 0;
  const limit = options.limit ?? 0;
  requireNonNegativeInteger(offset, "offset");
  requireNonNegativeInteger(limit, "limit");
  return { offset, limit };
}

function calculationOptions(options) {
  return {
    todaySerial: options.todaySerial,
    nowSerial: options.nowSerial,
    randomSeed: options.randomSeed,
    localeProfile: options.localeProfile,
    arithmeticSemantics: options.arithmeticSemantics,
    financialSolverSemantics: options.financialSolverSemantics,
  };
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

function formatDisplayText(value, formatCode, options = {}) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw inputError(INPUT_DETAIL.DISPLAY_VALUE_INVALID);
  }
  requireString(formatCode, "formatCode");
  requireOptions(options);
  requireOptionKeys(options, ["dateSystem", "localeProfile"]);
  requireOptionalString(options.dateSystem, "dateSystem");
  requireOptionalLocaleProfile(options.localeProfile, "localeProfile");
  const request = {
    value,
    format_code: formatCode,
    date_system: options.dateSystem ?? "excel_1900",
    locale_profile: localeProfileWire(options.localeProfile),
  };
  return withSyncErrors(() => native.formatDisplayText(JSON.stringify(request)));
}

module.exports = {
  SCHEMA_VERSION: native.schemaVersion(),
  CellRuneError,
  Workbook,
  formatDisplayText,
  functionCatalog,
};
