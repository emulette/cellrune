import type { Buffer } from "node:buffer";

export const SCHEMA_VERSION: number;

export type CellRuneErrorKind =
  | "input"
  | "validation"
  | "read"
  | "write"
  | "state";

export interface ErrorDetails {
  readonly sourceCode: string | null;
  readonly sourceId: string | null;
  readonly detail: string | null;
}

export class CellRuneError extends Error {
  private constructor();
  readonly code: string;
  readonly kind: CellRuneErrorKind;
  readonly details: ErrorDetails;
}

export type CellValue =
  | { readonly kind: "blank" }
  | { readonly kind: "number"; readonly value: number }
  | { readonly kind: "text"; readonly value: string }
  | { readonly kind: "logical"; readonly value: boolean }
  | { readonly kind: "error"; readonly value: string }
  | { readonly kind: "unsupported" };

export type CalculationResult =
  | { readonly kind: "value"; readonly value: CellValue }
  | {
      readonly kind: "unavailable";
      readonly code: string;
      readonly message: string;
      readonly detail: string | null;
    };

export interface Cell {
  readonly address: string;
  readonly formula: string | null;
  readonly sourceValue: CellValue;
  readonly sourceValueState: "literal" | "saved" | "missing" | "invalid";
  readonly calculated: CalculationResult | null;
}

export interface RangePage {
  readonly schemaVersion: number;
  readonly sheet: string;
  readonly start: string;
  readonly end: string;
  readonly totalCells: number;
  readonly offset: number;
  readonly nextOffset: number | null;
  readonly cells: readonly Cell[];
}

export interface DefinedNameSheetSpan {
  readonly startSheetId: number;
  readonly startSheetName: string;
  readonly endSheetId: number;
  readonly endSheetName: string;
}

export type DefinedNameReferenceArea =
  | {
      readonly kind: "rectangular";
      readonly sheetId: number;
      readonly sheetName: string;
      readonly range: string;
    }
  | {
      readonly kind: "threeDimensional";
      readonly sheetSpan: DefinedNameSheetSpan;
      readonly range: string;
    };

export type DefinedNameInspectionResult =
  | {
      readonly kind: "rectangular";
      readonly sheetId: number;
      readonly sheetName: string;
      readonly range: string;
    }
  | {
      readonly kind: "threeDimensional";
      readonly sheetSpan: DefinedNameSheetSpan;
      readonly range: string;
    }
  | {
      readonly kind: "nonRectangular";
      readonly areas: readonly DefinedNameReferenceArea[];
    }
  | { readonly kind: "emptyReference" }
  | {
      readonly kind: "dynamicFormula";
      readonly dynamicKind: "offset" | "indirect" | "spill" | "mixed";
      readonly formula: string;
    }
  | { readonly kind: "constant"; readonly formula: string }
  | {
      readonly kind: "externalReference";
      readonly locator: string | null;
      readonly workbook: string;
      readonly sheet: string | null;
      readonly sheetEnd: string | null;
      readonly targetKind:
        | "reference"
        | "defined_name"
        | "structured_reference";
      readonly targetText: string;
    }
  | {
      readonly kind: "invalid";
      readonly reason:
        | "parse_error"
        | "circular_reference"
        | "unresolved_name"
        | "invalid_reference";
      readonly detail: string | null;
    }
  | {
      readonly kind: "unsupported";
      readonly reason:
        | "non_reference_expression"
        | "context_dependent"
        | "unsupported_expression";
      readonly detail: string | null;
    }
  | { readonly kind: "notFound" };

export interface DefinedNameInspection {
  readonly schemaVersion: number;
  readonly result: DefinedNameInspectionResult;
}

export interface TableColumn {
  readonly id: number;
  readonly name: string;
  readonly totalsRowFunction: string | null;
}

export interface TableSummary {
  readonly id: number;
  readonly name: string;
  readonly displayName: string;
  readonly range: string;
  readonly headerRowCount: number;
  readonly totalsRowCount: number;
  readonly columns: readonly TableColumn[];
}

export interface SheetSummary {
  readonly id: number;
  readonly name: string;
  readonly visibility: "visible" | "hidden" | "very_hidden";
  readonly cellCount: number;
  readonly usedRange: string | null;
  readonly mergedRanges: readonly string[];
  readonly tables: readonly TableSummary[];
}

export interface WorkbookSummary {
  readonly schemaVersion: number;
  readonly semanticRevision: bigint;
  readonly documentBacked: boolean;
  readonly documentKind: "xlsx" | "xlsm" | "new_xlsx" | "open_xml";
  readonly dateSystem: "excel_1900" | "excel_1904";
  readonly diagnosticCount: number;
  readonly sheets: readonly SheetSummary[];
}

export interface CalculationOptions {
  readonly todaySerial?: number;
  readonly nowSerial?: number;
  readonly arithmeticSemantics?: "excel_near_zero" | "ieee_754";
  readonly financialSolverSemantics?:
    | "excel_iteration_budget"
    | "extended_search";
}

export interface CalculationReport {
  readonly schemaVersion: number;
  readonly semanticRevision: bigint;
  readonly formulaCount: number;
  readonly valueCount: number;
  readonly unavailableCount: number;
  readonly materializedCellCount: number;
}

export type RecalculationMode = "auto" | "incremental" | "full";
export type CalculationExecutionMode = "incremental" | "full";
export type CalculationDecisionReason =
  | "initial_calculation"
  | "full_requested"
  | "incremental_requested"
  | "dirty_subset"
  | "no_dirty_formulas"
  | "topology_changed"
  | "options_changed"
  | "dynamic_topology"
  | "dirty_set_covers_workbook"
  | "unknown";

export interface RecalculationOptions extends CalculationOptions {
  readonly mode?: RecalculationMode;
}

export interface CellReference {
  readonly sheetId: number;
  readonly sheetName: string;
  readonly address: string;
}

export type WritableCellValue = Exclude<CellValue, { readonly kind: "unsupported" }>;

export type NumberFormatKind =
  | "general"
  | "number"
  | "date"
  | "time"
  | "date_time"
  | "duration";

export type SheetVisibility = "visible" | "hidden" | "very_hidden";

export type WorkbookChange =
  | {
      readonly kind: "setValue";
      readonly sheet: string;
      readonly address: string;
      readonly value: WritableCellValue;
    }
  | {
      readonly kind: "setFormula";
      readonly sheet: string;
      readonly address: string;
      readonly formula: string;
      readonly dynamicRange?: string | null;
    }
  | {
      readonly kind: "clearCell";
      readonly sheet: string;
      readonly address: string;
    }
  | {
      readonly kind: "setNumberFormat";
      readonly sheet: string;
      readonly address: string;
      readonly id: number;
      readonly code?: string | null;
      readonly formatKind: NumberFormatKind;
    }
  | { readonly kind: "addSheet"; readonly name: string }
  | {
      readonly kind: "renameSheet";
      readonly sheet: string;
      readonly newName: string;
    }
  | {
      readonly kind: "setSheetVisibility";
      readonly sheet: string;
      readonly visibility: SheetVisibility;
    }
  | {
      readonly kind: "setDefinedName";
      readonly name: string;
      readonly scopeSheet?: string | null;
      readonly formula: string;
      readonly hidden: boolean;
    }
  | {
      readonly kind: "removeDefinedName";
      readonly name: string;
      readonly scopeSheet?: string | null;
    }
  | {
      readonly kind: "setDateSystem";
      readonly dateSystem: "excel_1900" | "excel_1904";
    }
  | {
      readonly kind: "setCalculationHints";
      readonly mode?:
        | "automatic"
        | "automatic_except_data_tables"
        | "manual"
        | null;
      readonly calculationId?: number | null;
      readonly fullCalculationOnLoad?: boolean | null;
      readonly forceFullCalculation?: boolean | null;
      readonly iterativeCalculation?: boolean | null;
    };

export interface EditReceipt {
  readonly schemaVersion: number;
  readonly baseRevision: bigint;
  readonly resultRevision: bigint;
  readonly appliedChangeCount: number;
  readonly changedCells: readonly CellReference[];
  readonly calculationChangedCells: readonly CellReference[];
  readonly createdSheetIds: readonly number[];
  readonly topologyChanged: boolean;
  readonly calculationMetadataChanged: boolean;
}

export interface CalculationDeltaCell {
  readonly cell: CellReference;
  readonly origin:
    | "direct_formula"
    | "legacy_array"
    | "dynamic_spill"
    | "unknown";
  readonly anchor: CellReference | null;
  readonly range: string | null;
  readonly result: CalculationResult;
}

export interface CalculationDelta {
  readonly schemaVersion: number;
  readonly cursor: bigint;
  readonly baseRevision: bigint;
  readonly resultRevision: bigint;
  readonly mode: CalculationExecutionMode;
  readonly reason: CalculationDecisionReason;
  readonly dirtyCount: number;
  readonly evaluatedCount: number;
  readonly parsedFormulaCount: number;
  readonly changedCells: readonly CalculationDeltaCell[];
  readonly removedMaterializedCells: readonly CellReference[];
}

export interface CalculationDeltaPage {
  readonly schemaVersion: number;
  readonly requestedCursor: bigint;
  readonly nextCursor: bigint | null;
  readonly deltas: readonly CalculationDelta[];
}

export interface FunctionUsageEntry {
  readonly name: string;
  readonly supported: boolean;
  readonly callCount: number;
  readonly formulaCount: number;
  readonly sampleCells: readonly CellReference[];
}

export interface FunctionUsageReport {
  readonly schemaVersion: number;
  readonly formulaCount: number;
  readonly parsedFormulaCount: number;
  readonly unparsedFormulaCount: number;
  readonly entries: readonly FunctionUsageEntry[];
}

export interface WriteOptions {
  readonly invalidateUnavailable?: boolean;
}

export interface SaveOptions extends WriteOptions {
  readonly replaceExisting?: boolean;
}

export interface WriteReport {
  readonly schemaVersion: number;
  readonly complete: boolean;
  readonly policy: "require_complete" | "invalidate_unavailable" | "unknown";
  readonly materializedCount: number;
  readonly invalidatedCells: readonly CellReference[];
  readonly changedParts: readonly string[];
  readonly removedParts: readonly string[];
  readonly diagnosticCount: number;
}

export class Workbook {
  private constructor();
  static create(): Workbook;
  static openPath(path: string): Promise<Workbook>;
  static fromBytes(bytes: Buffer | Uint8Array): Promise<Workbook>;
  get closed(): boolean;
  close(): void;
  summary(): WorkbookSummary;
  readRange(
    sheet: string,
    start: string,
    end: string,
    options?: { readonly offset?: number; readonly limit?: number },
  ): RangePage;
  inspectDefinedName(
    name: string,
    options?: { readonly currentSheet?: string },
  ): DefinedNameInspection;
  functionUsage(): FunctionUsageReport;
  calculate(options?: CalculationOptions): Promise<CalculationReport>;
  recalculate(options?: RecalculationOptions): Promise<CalculationDelta>;
  applyChanges(
    expectedRevision: bigint,
    changes: readonly WorkbookChange[],
  ): EditReceipt;
  changesSince(
    cursor?: bigint,
    options?: { readonly limit?: number },
  ): CalculationDeltaPage;
  cancelCalculation(): boolean;
  calculationActive(): boolean;
  setBlank(sheet: string, address: string): void;
  setNumber(sheet: string, address: string, value: number): void;
  setText(sheet: string, address: string, value: string): void;
  setLogical(sheet: string, address: string, value: boolean): void;
  setError(sheet: string, address: string, value: string): void;
  setFormula(
    sheet: string,
    address: string,
    formula: string,
    options?: { readonly dynamicRange?: string },
  ): void;
  clearCell(sheet: string, address: string): boolean;
  addSheet(name: string): number;
  renameSheet(currentName: string, newName: string): void;
  toBytes(options?: WriteOptions): Promise<Buffer>;
  save(path: string, options?: SaveOptions): Promise<WriteReport>;
}

declare const packageExports: {
  readonly SCHEMA_VERSION: typeof SCHEMA_VERSION;
  readonly CellRuneError: typeof CellRuneError;
  readonly Workbook: typeof Workbook;
};

export default packageExports;
