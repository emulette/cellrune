import {
  CellRuneError,
  type CalculationDelta,
  type CellValue,
  type DefinedNameInspection,
  type EditReceipt,
  type EditReceiptV2,
  type FunctionCatalogReport,
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
  functionCatalog,
} from "@cellrune/node";
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
  workbook.summary().fingerprint.digestHex.toUpperCase();
  workbook.setNumber("Sheet1", "A1", 1);
  workbook.setFormula("Sheet1", "B1", "=A1+1");
  await workbook.calculate();
  await workbook.calculate({
    arithmeticSemantics: "ieee_754",
    financialSolverSemantics: "extended_search",
  });
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
  const reopened: Workbook = await Workbook.fromBytes(bytes);
  reopened.close();
  reopened.close();
  reopened.closed.valueOf();
}

void check().catch((error: unknown) => {
  if (error instanceof CellRuneError) {
    error.code.toUpperCase();
  }
});
