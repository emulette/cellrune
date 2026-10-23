import {
  type DependentPage,
  type PrecedentPage,
  type RangeValue,
  type PhoneticPage,
  type LimitDiagnostics,
  type CapabilityPage,
  type CellFormatPage,
  type CommentPage,
  type HyperlinkPage,
  type DataValidationPage,
  type CalculationLimitOptions,
  type ReadLimitOptions,
  type EditWriteReport,
  type TargetCalculationResult,
  CellRuneError,
  type CalculationDelta,
  type CellValue,
  type DefinedNameInspection,
  type EditReceipt,
  type EditReceiptV2,
  type FunctionCatalogReport,
  type LocaleProfile,
  type PreviewChanges,
  type PreviewCursor,
  type RangePage,
  type TableSummary,
  type TransactionImpactPage,
  type WorkbookTransactionReceipt,
  type WorkbookChange,
  type WorkbookChangeV2,
  type WriteReport,
  Workbook,
  formatDisplayText,
  functionCatalog,
} from "@cellrune/node";

async function checkTargets(workbook: Workbook): Promise<void> {
  const result: TargetCalculationResult = await workbook.calculateTargets([{sheet: "Sheet1", start: "A1"}], {limits: {maxResultCells: 10}});
  const revision: bigint = result.semanticRevision;
  revision.toString();
  result.cells[0].cell.address.toUpperCase();
  // @ts-expect-error Targeted calculation keeps the default calculation limits.
  await workbook.calculateTargets([{sheet: "Sheet1", start: "A1"}], {calculationLimits: {maxArrayCells: 10}});
}
void checkTargets;
import type { Buffer } from "node:buffer";

// @ts-expect-error CellRuneError instances are created by the binding.
new CellRuneError("manual construction is unsupported");

const tableSummary: TableSummary = {
  id: 1,
  name: "SalesObject",
  displayName: "Sales",
  range: "A1:B3",
  headerRowCount: 1,
  totalsRowCount: 0,
  columns: [
    { id: 1, name: "Region", totalsRowFunction: null },
    { id: 2, name: "Amount", totalsRowFunction: "sum" },
  ],
};
tableSummary.id.toFixed(0);

async function check(): Promise<void> {
  const catalog: FunctionCatalogReport = functionCatalog();
  catalog.entries[0].canonicalName.toUpperCase();
  const workbook: Workbook = Workbook.create();
  const callCount: bigint = workbook.functionUsage().entries[0].callCount;
  callCount.toString();
  workbook.summary().fingerprint.digestHex.toUpperCase();
  workbook.setNumber("Sheet1", "A1", 1);
  workbook.setFormula("Sheet1", "B1", "=A1+1");
  await workbook.calculate();
  await workbook.calculate({
    arithmeticSemantics: "ieee_754",
    financialSolverSemantics: "extended_search",
  });
  const korean: LocaleProfile = { locale: "ko-KR", displayLanguage: "en-US" };
  await workbook.calculate({ localeProfile: korean });
  const shown: string = formatDisplayText({ kind: "number", value: 1 }, "0.00", {
    dateSystem: "excel_1904",
    localeProfile: korean,
  });
  shown.toUpperCase();
  // @ts-expect-error Only the built-in locales are accepted.
  await workbook.calculate({ localeProfile: { locale: "ja-JP" } });
  const changes: readonly WorkbookChange[] = [
    {
      kind: "setValue",
      sheet: "Sheet1",
      address: "A1",
      value: { kind: "number", value: 2 },
    },
  ];
  const receipt: EditReceipt = workbook.applyChanges(
    workbook.summary().semanticRevision,
    changes,
  );
  const changesV2: readonly WorkbookChangeV2[] = [
    {
      kind: "renameTableColumn",
      tableId: 1,
      columnId: 2,
      newName: "Gross Amount",
    },
  ];
  const receiptV2: EditReceiptV2 = workbook.applyChangesV2(
    receipt.resultRevision,
    changesV2,
  );
  receiptV2.changedTableIds.map((tableId) => tableId.toFixed(0));
  receipt.calculationChangedCells.length;
  receipt.calculationMetadataChanged.valueOf();
  const delta: CalculationDelta = await workbook.recalculate({
    mode: "incremental",
  });
  const preview: PreviewChanges = await workbook.previewChanges(
    delta.resultRevision,
    changes,
  );
  const previewPage: TransactionImpactPage = workbook.previewChangesPage(
    preview.previewId,
    { section: "preview_results", limit: 1 },
  );
  const cursor: PreviewCursor | null = previewPage.nextCursor;
  cursor?.token.toUpperCase();
  const transactionReceipt: WorkbookTransactionReceipt = workbook.commitPreview(
    preview.previewId,
  );
  transactionReceipt.calculationDelta.resultRevision === preview.report.resultRevision;
  delta.resultRevision === receipt.resultRevision;
  workbook.changesSince(0n);
  const page: RangePage = workbook.readRange("Sheet1", "A1", "B1");
  const inspection: DefinedNameInspection = workbook.inspectDefinedName(
    "InputArea",
    { currentSheet: "Sheet1" },
  );
  if (inspection.result.kind === "rectangular") {
    inspection.result.sheetName.toUpperCase();
    inspection.result.range.toUpperCase();
  }
  if (inspection.result.kind === "externalReference") {
    inspection.result.locator?.toUpperCase();
    inspection.result.workbook.toUpperCase();
    inspection.result.targetText.toUpperCase();
  }
  const value: CellValue = page.cells[0].sourceValue;
  if (value.kind === "number") {
    value.value.toFixed(2);
  }
  const bytes: Buffer = await workbook.toBytes();
  const writeReport: WriteReport = await workbook.save("typing-check.xlsx");
  writeReport.outputSha256.toUpperCase();
  const limits: CalculationLimitOptions = { maxArrayCells: 2_000_000 };
  await workbook.calculate({ calculationLimits: limits });
  const readLimits: ReadLimitOptions = { maxEntryUncompressedBytes: 1 << 27 };
  const reopened: Workbook = await Workbook.fromBytes(bytes, { readLimits });
  const edited: Buffer = await reopened.editsToBytes();
  const capabilities: CapabilityPage = await reopened.scanCapabilities({ limit: 10 });
  capabilities.entries[0]?.cell.address.toUpperCase();
  const formats: CellFormatPage = reopened.readFormats("Sheet1", "A1", "B2", { limit: 4 });
  const fill = formats.cells[0]?.format.fill;
  if (fill?.kind === "pattern") {
    fill.pattern.toUpperCase();
  }
  reopened.applyChangesV2(reopened.summary().semanticRevision, [
    { kind: "setCellFormat", sheet: "Sheet1", address: "A1", font: { bold: true } },
    { kind: "setRowHeight", sheet: "Sheet1", row: 1, height: null },
  ]);
  const csv: Buffer = Workbook.fromCsv(new Uint8Array([49, 44, 50]), { delimiter: "," }).toCsv("Sheet1");
  csv.byteLength.toFixed(0);
  const links: HyperlinkPage = reopened.hyperlinks("Sheet1");
  links.entries[0]?.target?.toUpperCase();
  const notes: CommentPage = reopened.comments("Sheet1", { offset: 0 });
  notes.entries[0]?.text.toUpperCase();
  const validations: DataValidationPage = reopened.dataValidations("Sheet1", { limit: 10 });
  for (const rule of validations.entries) {
    rule.ranges[0]?.toUpperCase();
    if (rule.validationType === "list") {
      rule.formula1?.toUpperCase();
    }
  }
  // @ts-expect-error Sheet metadata pages take only an offset and a limit.
  reopened.dataValidations("Sheet1", { cursor: 1 });
  const compactPage = reopened.readRange("Sheet1", "A1", "B2", { compact: true, maxBytes: 4096 });
  compactPage.rows?.[0]?.length.toFixed(0);
  // @ts-expect-error Range forms are booleans.
  reopened.readRange("Sheet1", "A1", "B2", { compact: 1 });
  const abort = new AbortController();
  void reopened.calculate({ signal: abort.signal });
  void reopened.calculateTargets([{ sheet: "Sheet1", start: "A1" }], { signal: abort.signal });
  // @ts-expect-error A signal is an AbortSignal.
  void reopened.recalculate({ signal: true });
  const precedentPage: PrecedentPage = reopened.precedents("Sheet1", "A1", { limit: 10 });
  precedentPage.entries[0]?.range.toUpperCase();
  const dependentPage: DependentPage = reopened.dependents("Sheet1", "A1");
  dependentPage.entries[0]?.cell.address.toUpperCase();
  const grid: RangeValue[][] = reopened.readValues("Sheet1", "A1", "B2");
  for (const value of grid.flat()) {
    if (value.kind === "unavailable") {
      value.code.toUpperCase();
    }
  }
  reopened.writeValues("Sheet1", "C1", [[{ kind: "number", value: 1 }, { kind: "blank" }]]);
  // @ts-expect-error A rectangle write takes writable values only.
  reopened.writeValues("Sheet1", "C1", [[{ kind: "unsupported" }]]);
  const phonetics: PhoneticPage = reopened.phonetics("Sheet1", { limit: 10 });
  for (const annotated of phonetics.entries) {
    annotated.runs[0]?.text.toUpperCase();
    annotated.properties?.phoneticType?.toUpperCase();
  }
  const pane = reopened.summary().sheets[0]?.frozenPane;
  if (pane !== undefined) {
    pane.rows.toFixed(0);
  }
  const presentation: readonly WorkbookChangeV2[] = [
    { kind: "setFrozenPane", sheet: "Sheet1", rows: 1, columns: 0 },
    { kind: "widenSheetAutoFilter", sheet: "Sheet1", range: "A1:C3" },
    {
      kind: "setPhonetics",
      sheet: "Sheet1",
      address: "A1",
      runs: [{ startUtf16: 0, endUtf16: 1, text: "あ" }],
      properties: { phoneticType: "Hiragana" },
    },
  ];
  presentation.length.toFixed(0);
  try {
    reopened.phonetics("Missing");
  } catch (error) {
    if (error instanceof CellRuneError) {
      const limit: LimitDiagnostics | undefined = error.details.limit;
      limit?.observed?.toFixed(0);
    }
  }
  edited.byteLength.toFixed(0);
  reopened.close();
  reopened.addSheet("Scratch");
  const removal: readonly WorkbookChange[] = [
    { kind: "deleteSheet", sheet: "Scratch" },
  ];
  reopened.applyChanges(reopened.summary().semanticRevision, removal);
  reopened.addSheet("Other");
  reopened.deleteSheet("Other");
  reopened.close();
  reopened.closed.valueOf();
}

void check().catch((error: unknown) => {
  if (error instanceof CellRuneError) {
    error.code.toUpperCase();
  }
});

export async function checkEditReport(workbook: Workbook): Promise<void> {
  const report: EditWriteReport = await workbook.saveEdits("edited.xlsx", {
    replaceExisting: true,
  });
  report.inputSha256.toUpperCase();
  report.affectedFormulas[0]?.address.toUpperCase();
  if (report.requestsHostRecalculation) {
    [...report.changedParts].sort();
  }
}
