use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroU32;
use std::sync::{Arc, OnceLock};

mod table_index;

pub(crate) use table_index::TableRangeIndex;
use table_index::{TableColumnLocation, TableIndex, TableIndexBuildError, TableLocation};

use crate::calculation::persistent_store::{PersistentRadixMap, PersistentRadixValues};
use crate::{
    Cell, CellAddress, CellContent, CellRange, Column, DefinedName, DefinedNameScope, Diagnostic,
    NumberFormat, Provenance, Row, Table, TableColumn, TableColumnId, TableId, ValidationError,
};

#[cfg(test)]
fn clone_map_cancellable<K, V>(
    source: &BTreeMap<K, V>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<K, V>, ()>
where
    K: Clone + Ord,
    V: Clone,
{
    let mut cloned = BTreeMap::new();
    for (key, value) in source {
        if cancelled() {
            return Err(());
        }
        cloned.insert(key.clone(), value.clone());
    }
    Ok(cloned)
}

/// A validated, non-zero workbook-local sheet identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SheetId(NonZeroU32);

impl SheetId {
    /// Validates and constructs a sheet ID.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::SheetIdZero`] when `value` is zero.
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(ValidationError::SheetIdZero)
    }

    /// Returns the workbook-local numeric ID.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for SheetId {
    type Error = ValidationError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A validated sheet name with its original spelling preserved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SheetName {
    original: Box<str>,
    lookup_key: Box<str>,
}

impl SheetName {
    /// Applies Excel's length, apostrophe, and forbidden-character constraints.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the name violates an Excel sheet-name constraint.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ValidationError::SheetNameEmpty);
        }
        let utf16_len = value.encode_utf16().count();
        if utf16_len > 31 {
            return Err(ValidationError::SheetNameTooLong { utf16_len });
        }
        if value.starts_with('\'') || value.ends_with('\'') {
            return Err(ValidationError::SheetNameApostropheBoundary);
        }
        if let Some(character) = value.chars().find(|character| {
            character.is_control() || matches!(character, ':' | '\\' | '/' | '?' | '*' | '[' | ']')
        }) {
            return Err(ValidationError::SheetNameInvalidCharacter { character });
        }
        let lookup_key = case_insensitive_key(&value).into_boxed_str();
        Ok(Self {
            original: value.into_boxed_str(),
            lookup_key,
        })
    }

    /// Returns the original spelling.
    pub fn as_str(&self) -> &str {
        &self.original
    }

    pub(crate) fn lookup_key(&self) -> &str {
        &self.lookup_key
    }
}

/// Sheet visibility as represented by `SpreadsheetML`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SheetVisibility {
    /// Visible in the workbook UI.
    #[default]
    Visible,
    /// Hidden, but users can make the sheet visible through the normal UI.
    Hidden,
    /// Hidden and unavailable to the normal unhide UI.
    VeryHidden,
}

#[derive(Debug, Clone, Default)]
struct CellStore {
    cells: PersistentRadixMap<Cell>,
    len: usize,
}

struct CellStoreValues<'a> {
    inner: PersistentRadixValues<'a, Cell>,
    remaining: usize,
}

impl<'a> Iterator for CellStoreValues<'a> {
    type Item = &'a Cell;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.inner.next()?;
        self.remaining -= 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for CellStoreValues<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let value = self.inner.next_back()?;
        self.remaining -= 1;
        Some(value)
    }
}

impl ExactSizeIterator for CellStoreValues<'_> {}

impl CellStore {
    fn key(address: CellAddress) -> u128 {
        u128::from(address.row().get() - 1) * u128::from(crate::EXCEL_MAX_COLUMNS)
            + u128::from(address.column().get() - 1)
    }

    fn contains_key(&self, address: &CellAddress) -> bool {
        self.get(address).is_some()
    }

    fn get(&self, address: &CellAddress) -> Option<&Cell> {
        self.cells.get(Self::key(*address))
    }

    fn insert(&mut self, address: CellAddress, cell: Cell) -> bool {
        let previous = self.cells.insert(Self::key(address), cell);
        if previous.is_none() {
            self.len += 1;
        }
        previous.is_some()
    }

    fn remove(&mut self, address: &CellAddress) -> bool {
        if self.cells.get(Self::key(*address)).is_none() {
            return false;
        }
        let removed = self.cells.remove(Self::key(*address));
        if removed.is_some() {
            self.len -= 1;
        }
        removed.is_some()
    }

    fn values(&self) -> CellStoreValues<'_> {
        CellStoreValues {
            inner: self.cells.ordered_values(),
            remaining: self.len,
        }
    }

    const fn len(&self) -> usize {
        self.len
    }

    const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn semantic_fingerprint_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        self.cells.semantic_fingerprint_cancellable(
            &|entries| {
                crate::calculation::identity::cell_chunk_fingerprint_cancellable(
                    entries.entries().map(|(_, cell)| cell),
                    entries.len(),
                    cancelled,
                )
            },
            &crate::calculation::identity::cell_store_node_fingerprint,
            cancelled,
        )
    }
}

/// A sparse, format-neutral worksheet.
#[derive(Debug, Clone)]
pub struct Sheet {
    id: SheetId,
    name: SheetName,
    visibility: SheetVisibility,
    cells: CellStore,
    formula_addresses: Arc<BTreeSet<CellAddress>>,
    column_max_rows: Arc<BTreeMap<u32, u32>>,
    bounds_dirty: bool,
    min_row: Option<Row>,
    min_column: Option<Column>,
    max_row: Option<Row>,
    max_column: Option<Column>,
    merged_ranges: Arc<Vec<CellRange>>,
    tables: Arc<Vec<Table>>,
    semantic_fingerprint: OnceLock<[u8; 32]>,
}

impl Sheet {
    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        if cancelled() {
            return Err(());
        }
        Ok(Self {
            id: self.id,
            name: self.name.clone(),
            visibility: self.visibility,
            cells: self.cells.clone(),
            formula_addresses: Arc::clone(&self.formula_addresses),
            column_max_rows: Arc::clone(&self.column_max_rows),
            bounds_dirty: self.bounds_dirty,
            min_row: self.min_row,
            min_column: self.min_column,
            max_row: self.max_row,
            max_column: self.max_column,
            merged_ranges: Arc::clone(&self.merged_ranges),
            tables: Arc::clone(&self.tables),
            semantic_fingerprint: self.semantic_fingerprint.clone(),
        })
    }

    /// Constructs an empty sparse sheet.
    pub fn new(id: SheetId, name: SheetName, visibility: SheetVisibility) -> Self {
        Self {
            id,
            name,
            visibility,
            cells: CellStore::default(),
            formula_addresses: Arc::new(BTreeSet::new()),
            column_max_rows: Arc::new(BTreeMap::new()),
            bounds_dirty: false,
            min_row: None,
            min_column: None,
            max_row: None,
            max_column: None,
            merged_ranges: Arc::new(Vec::new()),
            tables: Arc::new(Vec::new()),
            semantic_fingerprint: OnceLock::new(),
        }
    }

    /// Returns the sheet ID.
    pub const fn id(&self) -> SheetId {
        self.id
    }

    /// Returns the original validated name.
    pub const fn name(&self) -> &SheetName {
        &self.name
    }

    /// Returns the sheet visibility.
    pub const fn visibility(&self) -> SheetVisibility {
        self.visibility
    }

    /// Inserts a sparse cell and rejects duplicate addresses.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::DuplicateCell`] when the address is already occupied.
    pub fn insert_cell(
        &mut self,
        address: CellAddress,
        content: CellContent,
    ) -> Result<(), ValidationError> {
        self.insert_cell_with_number_format(address, content, NumberFormat::default())
    }

    pub(crate) fn insert_cell_with_number_format(
        &mut self,
        address: CellAddress,
        content: CellContent,
        number_format: NumberFormat,
    ) -> Result<(), ValidationError> {
        if self.cells.contains_key(&address) {
            return Err(ValidationError::DuplicateCell {
                row: address.row().get(),
                column: address.column().get(),
            });
        }
        let is_formula = matches!(content, CellContent::Formula(_));
        self.semantic_fingerprint = OnceLock::new();
        self.update_bounds(address);
        self.update_column_extent(address);
        self.cells.insert(
            address,
            Cell::with_number_format(address, content, number_format),
        );
        if is_formula {
            Arc::make_mut(&mut self.formula_addresses).insert(address);
        }
        Ok(())
    }

    /// Returns a cell by typed address.
    pub fn cell(&self, address: CellAddress) -> Option<&Cell> {
        self.cells.get(&address)
    }

    /// Parses an unqualified A1 address and returns the corresponding sparse cell.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when `address` is not a valid unqualified A1 address.
    pub fn cell_by_a1(&self, address: &str) -> Result<Option<&Cell>, ValidationError> {
        Ok(self.cell(CellAddress::from_a1(address)?))
    }

    /// Iterates sparse cells in deterministic row-major order.
    pub fn cells(&self) -> impl ExactSizeIterator<Item = &Cell> + DoubleEndedIterator {
        self.cells.values()
    }

    pub(crate) fn column_max_rows(&self) -> &BTreeMap<u32, u32> {
        &self.column_max_rows
    }

    pub(crate) fn semantic_cell_store_fingerprint_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        self.cells.semantic_fingerprint_cancellable(cancelled)
    }

    pub(crate) fn semantic_fingerprint_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        if let Some(fingerprint) = self.semantic_fingerprint.get() {
            return Ok(*fingerprint);
        }
        let fingerprint =
            crate::calculation::identity::sheet_fingerprint_cancellable(self, cancelled)?;
        let _ = self.semantic_fingerprint.set(fingerprint);
        Ok(*self
            .semantic_fingerprint
            .get()
            .expect("sheet fingerprint was initialized"))
    }

    pub(crate) fn next_formula_cell_after(&self, after: Option<CellAddress>) -> Option<Cell> {
        let address = match after {
            Some(address) => self
                .formula_addresses
                .range(address..)
                .find(|candidate| **candidate > address),
            None => self.formula_addresses.first(),
        }?;
        self.cells.get(address).cloned()
    }

    /// Returns the number of stored sparse cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Returns whether the sheet stores no cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Returns merged ranges sorted by top-left address, then by bottom-right address.
    ///
    /// The reader stores only validated, pairwise non-overlapping, multi-cell ranges; entries
    /// that fail those checks are reported as diagnostics and dropped.
    pub fn merged_ranges(&self) -> &[CellRange] {
        self.merged_ranges.as_slice()
    }

    /// Stores the validated merged ranges for this sheet.
    ///
    /// Callers must pass ranges already sorted by `(start, end)` and pairwise non-overlapping;
    /// the reader's merge parser is the only producer of that order.
    pub(crate) fn set_merged_ranges(&mut self, merged_ranges: Vec<CellRange>) {
        self.semantic_fingerprint = OnceLock::new();
        self.merged_ranges = Arc::new(merged_ranges);
    }

    /// Returns this sheet's tables in XLSX declaration order.
    pub fn tables(&self) -> &[Table] {
        self.tables.as_slice()
    }

    /// Stores the validated tables for this sheet.
    ///
    /// Workbook-wide name uniqueness is enforced when the snapshot is constructed, not here.
    pub(crate) fn set_tables(&mut self, tables: Vec<Table>) {
        self.semantic_fingerprint = OnceLock::new();
        self.tables = Arc::new(tables);
    }

    pub(crate) fn tables_mut(&mut self) -> &mut [Table] {
        self.semantic_fingerprint = OnceLock::new();
        Arc::make_mut(&mut self.tables).as_mut_slice()
    }

    /// Returns the smallest bounding rectangle containing all sparse cells.
    pub fn used_range(&self) -> Option<CellRange> {
        let start = CellAddress::new(self.min_row?, self.min_column?);
        let end = CellAddress::new(self.max_row?, self.max_column?);
        Some(CellRange::from_ordered(start, end))
    }

    fn update_bounds(&mut self, address: CellAddress) {
        self.min_row = Some(
            self.min_row
                .map_or(address.row(), |row| row.min(address.row())),
        );
        self.min_column = Some(
            self.min_column
                .map_or(address.column(), |column| column.min(address.column())),
        );
        self.max_row = Some(
            self.max_row
                .map_or(address.row(), |row| row.max(address.row())),
        );
        self.max_column = Some(
            self.max_column
                .map_or(address.column(), |column| column.max(address.column())),
        );
    }

    fn update_column_extent(&mut self, address: CellAddress) {
        Arc::make_mut(&mut self.column_max_rows)
            .entry(address.column().get())
            .and_modify(|row| *row = (*row).max(address.row().get()))
            .or_insert(address.row().get());
    }

    pub(crate) fn upsert_cell(
        &mut self,
        address: CellAddress,
        content: CellContent,
        number_format: NumberFormat,
    ) {
        self.upsert_cell_deferred(address, content, number_format);
        self.finish_deferred_cell_edits();
    }

    pub(crate) fn upsert_cell_deferred(
        &mut self,
        address: CellAddress,
        content: CellContent,
        number_format: NumberFormat,
    ) {
        self.semantic_fingerprint = OnceLock::new();
        self.track_formula_address(address, &content);
        let is_new = !self.cells.insert(
            address,
            Cell::with_content_and_number_format(address, content, number_format),
        );
        if is_new {
            self.update_bounds(address);
            self.update_column_extent(address);
        }
    }

    pub(crate) fn upsert_cell_instance_deferred(&mut self, cell: Cell) {
        let address = cell.address();
        self.semantic_fingerprint = OnceLock::new();
        self.track_formula_address(address, cell.content());
        let is_new = !self.cells.insert(address, cell);
        if is_new {
            self.update_bounds(address);
            self.update_column_extent(address);
        }
    }

    pub(crate) fn remove_cell_deferred(&mut self, address: CellAddress) -> bool {
        self.semantic_fingerprint = OnceLock::new();
        if self.formula_addresses.contains(&address) {
            Arc::make_mut(&mut self.formula_addresses).remove(&address);
        }
        let removed = self.cells.remove(&address);
        if removed
            && (self.min_row == Some(address.row())
                || self.max_row == Some(address.row())
                || self.min_column == Some(address.column())
                || self.max_column == Some(address.column())
                || self
                    .column_max_rows
                    .get(&address.column().get())
                    .is_some_and(|row| *row == address.row().get()))
        {
            self.bounds_dirty = true;
        }
        removed
    }

    pub(crate) fn finish_deferred_cell_edits(&mut self) {
        if self.bounds_dirty {
            self.rebuild_bounds();
        }
    }

    pub(crate) fn rename(&mut self, name: SheetName) {
        self.semantic_fingerprint = OnceLock::new();
        self.name = name;
    }

    pub(crate) fn set_visibility(&mut self, visibility: SheetVisibility) {
        self.semantic_fingerprint = OnceLock::new();
        self.visibility = visibility;
    }

    fn rebuild_bounds(&mut self) {
        self.min_row = None;
        self.min_column = None;
        self.max_row = None;
        self.max_column = None;
        self.column_max_rows = Arc::new(BTreeMap::new());
        let addresses = self.cells.values().map(Cell::address).collect::<Vec<_>>();
        for address in addresses {
            self.update_bounds(address);
            self.update_column_extent(address);
        }
        self.bounds_dirty = false;
    }

    fn track_formula_address(&mut self, address: CellAddress, content: &CellContent) {
        if matches!(content, CellContent::Formula(_)) {
            if !self.formula_addresses.contains(&address) {
                Arc::make_mut(&mut self.formula_addresses).insert(address);
            }
        } else if self.formula_addresses.contains(&address) {
            Arc::make_mut(&mut self.formula_addresses).remove(&address);
        }
    }
}

/// Excel's serial date epoch selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DateSystem {
    /// The default 1900 date system, including Excel's historical leap-year behavior.
    #[default]
    Excel1900,
    /// The alternative 1904 date system.
    Excel1904,
}

/// Workbook calculation mode metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalculationMode {
    /// Recalculate automatically.
    Automatic,
    /// Recalculate automatically except for data tables.
    AutomaticExceptDataTables,
    /// Recalculate only when explicitly requested.
    Manual,
}

/// Calculation hints read from workbook metadata without triggering calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CalculationHints {
    mode: Option<CalculationMode>,
    calculation_id: Option<u32>,
    full_calculation_on_load: Option<bool>,
    force_full_calculation: Option<bool>,
    iterative_calculation: Option<bool>,
}

impl CalculationHints {
    /// Constructs workbook calculation hints.
    pub const fn new(
        mode: Option<CalculationMode>,
        calculation_id: Option<u32>,
        full_calculation_on_load: Option<bool>,
        force_full_calculation: Option<bool>,
    ) -> Self {
        Self {
            mode,
            calculation_id,
            full_calculation_on_load,
            force_full_calculation,
            iterative_calculation: None,
        }
    }

    /// Returns a copy with the declared iterative-calculation flag.
    pub const fn with_iterative_calculation(mut self, iterative_calculation: Option<bool>) -> Self {
        self.iterative_calculation = iterative_calculation;
        self
    }

    /// Returns the declared calculation mode.
    pub const fn mode(self) -> Option<CalculationMode> {
        self.mode
    }

    /// Returns the producer's calculation engine ID.
    pub const fn calculation_id(self) -> Option<u32> {
        self.calculation_id
    }

    /// Returns the full-calculation-on-load flag.
    pub const fn full_calculation_on_load(self) -> Option<bool> {
        self.full_calculation_on_load
    }

    /// Returns the force-full-calculation flag.
    pub const fn force_full_calculation(self) -> Option<bool> {
        self.force_full_calculation
    }

    /// Returns the declared iterative-calculation flag.
    pub const fn iterative_calculation(self) -> Option<bool> {
        self.iterative_calculation
    }
}

/// The caller-facing input adapter that supplied workbook bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkbookSourceKind {
    /// Source details are not available.
    #[default]
    Unknown,
    /// A filesystem path adapter supplied the bytes.
    Path,
    /// An in-memory byte buffer supplied the bytes.
    Bytes,
    /// A generic `Read + Seek` adapter supplied the bytes.
    Reader,
}

/// Non-sensitive source metadata retained by the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WorkbookSource {
    kind: WorkbookSourceKind,
    byte_length: Option<u64>,
}

impl WorkbookSource {
    /// Constructs source metadata without retaining a host path.
    pub const fn new(kind: WorkbookSourceKind, byte_length: Option<u64>) -> Self {
        Self { kind, byte_length }
    }

    /// Returns the input adapter kind.
    pub const fn kind(self) -> WorkbookSourceKind {
        self.kind
    }

    /// Returns the input byte length, when known.
    pub const fn byte_length(self) -> Option<u64> {
        self.byte_length
    }
}

/// A versioned, history-independent digest of workbook semantics.
///
/// The digest is an equality and stale-state aid. It is not an input-file hash, electronic
/// signature, or authenticity proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkbookFingerprint {
    schema_version: u16,
    bytes: [u8; 32],
}

impl WorkbookFingerprint {
    /// The semantic hashing schema used by this CellRune release.
    pub const CURRENT_SCHEMA_VERSION: u16 =
        crate::calculation::identity::WORKBOOK_FINGERPRINT_SCHEMA_VERSION as u16;

    pub(crate) const fn current(bytes: [u8; 32]) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            bytes,
        }
    }

    /// Returns the semantic hashing schema version.
    pub const fn schema_version(self) -> u16 {
        self.schema_version
    }

    /// Returns the stable digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Returns the digest as lower-case hexadecimal without a prefix.
    pub fn to_hex(self) -> String {
        self.to_string()
    }
}

impl fmt::Display for WorkbookFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.bytes {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// An immutable workbook snapshot with deterministic sheet lookup and order.
#[derive(Debug, Clone)]
pub struct WorkbookSnapshot {
    sheets: Vec<Sheet>,
    sheet_identity: PersistentRadixMap<Sheet>,
    sheet_id_index: BTreeMap<SheetId, usize>,
    sheet_name_index: BTreeMap<Box<str>, usize>,
    table_index: TableIndex,
    defined_names: Vec<DefinedName>,
    defined_name_index: BTreeMap<DefinedNameScope, BTreeMap<Box<str>, usize>>,
    diagnostics: Vec<Diagnostic>,
    date_system: DateSystem,
    calculation_hints: CalculationHints,
    source: WorkbookSource,
    provenance: Provenance,
    semantic_revision: u64,
    semantic_fingerprint: OnceLock<[u8; 32]>,
}

pub(crate) enum WorkbookBuildError {
    Validation(ValidationError),
    Cancelled,
}

pub(crate) struct WorkbookSnapshotInput {
    pub(crate) sheets: Vec<Sheet>,
    pub(crate) defined_names: Vec<DefinedName>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) date_system: DateSystem,
    pub(crate) calculation_hints: CalculationHints,
    pub(crate) source: WorkbookSource,
    pub(crate) provenance: Provenance,
}

impl From<ValidationError> for WorkbookBuildError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

impl WorkbookSnapshot {
    #[cfg(test)]
    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut sheets = Vec::with_capacity(self.sheets.len());
        for sheet in &self.sheets {
            if cancelled() {
                return Err(());
            }
            sheets.push(sheet.clone_cancellable(cancelled)?);
        }
        let mut defined_names = Vec::with_capacity(self.defined_names.len());
        for name in &self.defined_names {
            if cancelled() {
                return Err(());
            }
            defined_names.push(name.clone());
        }
        let mut defined_name_index = BTreeMap::new();
        for (scope, names) in &self.defined_name_index {
            if cancelled() {
                return Err(());
            }
            defined_name_index.insert(*scope, clone_map_cancellable(names, cancelled)?);
        }
        let mut diagnostics = Vec::with_capacity(self.diagnostics.len());
        for diagnostic in &self.diagnostics {
            if cancelled() {
                return Err(());
            }
            diagnostics.push(diagnostic.clone());
        }
        if cancelled() {
            return Err(());
        }
        Ok(Self {
            sheet_identity: self.sheet_identity.clone(),
            sheets,
            sheet_id_index: clone_map_cancellable(&self.sheet_id_index, cancelled)?,
            sheet_name_index: clone_map_cancellable(&self.sheet_name_index, cancelled)?,
            table_index: self.table_index.clone_cancellable(cancelled)?,
            defined_names,
            defined_name_index,
            diagnostics,
            date_system: self.date_system,
            calculation_hints: self.calculation_hints,
            source: self.source,
            provenance: self.provenance.clone(),
            semantic_revision: self.semantic_revision,
            semantic_fingerprint: self.semantic_fingerprint.clone(),
        })
    }

    pub(crate) fn new_draft() -> Self {
        let sheet_id = SheetId(NonZeroU32::MIN);
        let sheet_name = SheetName {
            original: Box::from("Sheet1"),
            lookup_key: Box::from("sheet1"),
        };
        let sheet = Sheet::new(sheet_id, sheet_name, SheetVisibility::Visible);
        let mut sheet_id_index = BTreeMap::new();
        sheet_id_index.insert(sheet_id, 0);
        let mut sheet_name_index = BTreeMap::new();
        sheet_name_index.insert(Box::from("sheet1"), 0);
        Self {
            sheet_identity: sheet_identity_store_cancellable(std::slice::from_ref(&sheet), &|| {
                false
            })
            .expect("the draft identity store cannot be cancelled"),
            sheets: vec![sheet],
            sheet_id_index,
            sheet_name_index,
            table_index: TableIndex::default(),
            defined_names: Vec::new(),
            defined_name_index: BTreeMap::new(),
            diagnostics: Vec::new(),
            date_system: DateSystem::Excel1900,
            calculation_hints: CalculationHints::default(),
            source: WorkbookSource::default(),
            provenance: Provenance::new(crate::ProviderIdentity::writer(), None),
            semantic_revision: 0,
            semantic_fingerprint: OnceLock::new(),
        }
    }

    /// Validates workbook-level uniqueness and constructs an immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when sheet IDs or names are not unique.
    pub fn new(
        sheets: Vec<Sheet>,
        date_system: DateSystem,
        calculation_hints: CalculationHints,
        source: WorkbookSource,
        provenance: Provenance,
    ) -> Result<Self, ValidationError> {
        Self::new_with_metadata(
            sheets,
            Vec::new(),
            Vec::new(),
            date_system,
            calculation_hints,
            source,
            provenance,
        )
    }

    /// Validates and constructs a snapshot with names and compatibility diagnostics.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when sheet IDs, sheet names, defined names, or table
    /// identities violate workbook-level uniqueness and scope constraints.
    pub fn new_with_metadata(
        sheets: Vec<Sheet>,
        defined_names: Vec<DefinedName>,
        diagnostics: Vec<Diagnostic>,
        date_system: DateSystem,
        calculation_hints: CalculationHints,
        source: WorkbookSource,
        provenance: Provenance,
    ) -> Result<Self, ValidationError> {
        match Self::new_with_metadata_cancellable(
            WorkbookSnapshotInput {
                sheets,
                defined_names,
                diagnostics,
                date_system,
                calculation_hints,
                source,
                provenance,
            },
            &|| false,
        ) {
            Ok(workbook) => Ok(workbook),
            Err(WorkbookBuildError::Validation(error)) => Err(error),
            Err(WorkbookBuildError::Cancelled) => {
                unreachable!("the non-cancellable snapshot builder cannot be cancelled")
            }
        }
    }

    pub(crate) fn new_with_metadata_cancellable(
        input: WorkbookSnapshotInput,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, WorkbookBuildError> {
        Self::new_with_metadata_cancellable_reusing_identity(input, None, cancelled)
    }

    pub(crate) fn new_with_metadata_cancellable_from_previous(
        input: WorkbookSnapshotInput,
        previous: &Self,
        touched_sheets: &BTreeSet<SheetId>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, WorkbookBuildError> {
        Self::new_with_metadata_cancellable_reusing_identity(
            input,
            Some((previous, touched_sheets)),
            cancelled,
        )
    }

    fn new_with_metadata_cancellable_reusing_identity(
        input: WorkbookSnapshotInput,
        identity_base: Option<(&Self, &BTreeSet<SheetId>)>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, WorkbookBuildError> {
        let WorkbookSnapshotInput {
            sheets,
            defined_names,
            diagnostics,
            date_system,
            calculation_hints,
            source,
            provenance,
        } = input;
        let mut sheet_id_index = BTreeMap::new();
        let mut sheet_name_index = BTreeMap::new();
        let mut identity_order_unchanged =
            identity_base.is_some_and(|(previous, _)| previous.sheets.len() == sheets.len());
        for (index, sheet) in sheets.iter().enumerate() {
            if cancelled() {
                return Err(WorkbookBuildError::Cancelled);
            }
            if identity_order_unchanged
                && identity_base
                    .and_then(|(previous, _)| previous.sheets.get(index))
                    .is_none_or(|previous| previous.id() != sheet.id())
            {
                identity_order_unchanged = false;
            }
            if sheet_id_index.insert(sheet.id(), index).is_some() {
                return Err(ValidationError::DuplicateSheetId {
                    value: sheet.id().get(),
                }
                .into());
            }
            let name_key = Box::<str>::from(sheet.name().lookup_key());
            if sheet_name_index.insert(name_key, index).is_some() {
                return Err(ValidationError::DuplicateSheetName {
                    name: sheet.name().as_str().to_owned(),
                }
                .into());
            }
        }
        let mut defined_name_index = BTreeMap::<DefinedNameScope, BTreeMap<Box<str>, usize>>::new();
        let mut defined_name_keys = std::collections::BTreeSet::<Box<str>>::new();
        for (index, defined_name) in defined_names.iter().enumerate() {
            if cancelled() {
                return Err(WorkbookBuildError::Cancelled);
            }
            if let DefinedNameScope::Sheet(sheet_id) = defined_name.scope()
                && !sheet_id_index.contains_key(&sheet_id)
            {
                return Err(ValidationError::DefinedNameUnknownSheet {
                    sheet_id: sheet_id.get(),
                }
                .into());
            }
            let previous = defined_name_index
                .entry(defined_name.scope())
                .or_default()
                .insert(Box::from(defined_name.lookup_key()), index);
            if previous.is_some() {
                return Err(ValidationError::DuplicateDefinedName {
                    name: defined_name.name().to_owned(),
                }
                .into());
            }
            defined_name_keys.insert(Box::from(defined_name.lookup_key()));
        }
        let table_index = TableIndex::new_cancellable(&sheets, &defined_name_keys, cancelled)
            .map_err(|error| match error {
                TableIndexBuildError::Validation(error) => WorkbookBuildError::Validation(error),
                TableIndexBuildError::Cancelled => WorkbookBuildError::Cancelled,
            })?;
        if cancelled() {
            return Err(WorkbookBuildError::Cancelled);
        }
        let sheet_identity = if identity_order_unchanged {
            let (previous, touched_sheets) =
                identity_base.expect("unchanged identity order requires a previous workbook");
            let mut identity = previous.sheet_identity.clone();
            for sheet_id in touched_sheets {
                if cancelled() {
                    return Err(WorkbookBuildError::Cancelled);
                }
                let Some(index) = sheet_id_index.get(sheet_id).copied() else {
                    return Err(ValidationError::UnknownSheetId {
                        value: sheet_id.get(),
                    }
                    .into());
                };
                identity.insert(index as u128, sheets[index].clone());
            }
            identity
        } else {
            sheet_identity_store_cancellable(&sheets, cancelled)
                .map_err(|()| WorkbookBuildError::Cancelled)?
        };
        Ok(Self {
            sheet_identity,
            sheets,
            sheet_id_index,
            sheet_name_index,
            table_index,
            defined_names,
            defined_name_index,
            diagnostics,
            date_system,
            calculation_hints,
            source,
            provenance,
            semantic_revision: 0,
            semantic_fingerprint: OnceLock::new(),
        })
    }

    /// Returns sheets in workbook order.
    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    /// Returns defined names in workbook XML order.
    pub fn defined_names(&self) -> &[DefinedName] {
        &self.defined_names
    }

    pub(crate) fn defined_name(
        &self,
        scope: DefinedNameScope,
        name: &str,
    ) -> Option<(usize, &DefinedName)> {
        let key = case_insensitive_key(name);
        let index = *self.defined_name_index.get(&scope)?.get(key.as_str())?;
        Some((index, &self.defined_names[index]))
    }

    /// Returns deterministic read-time compatibility diagnostics.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns a sheet by stable ID.
    pub fn sheet_by_id(&self, id: SheetId) -> Option<&Sheet> {
        self.sheet_id_index
            .get(&id)
            .map(|index| &self.sheets[*index])
    }

    pub(crate) fn sheet_position(&self, id: SheetId) -> Option<usize> {
        self.sheet_id_index.get(&id).copied()
    }

    /// Returns a table by its workbook-global formula/UI display name.
    ///
    /// OOXML `displayName` values are workbook-global and case-insensitive even though each table
    /// is owned by its worksheet. The worksheet-local programmatic `name` is not a lookup key.
    pub fn table(&self, name: &str) -> Option<&Table> {
        let location = self.table_location(name)?;
        Some(&self.sheets[location.sheet_index].tables()[location.table_index])
    }

    /// Returns a table by its stable workbook-local identifier.
    pub fn table_by_id(&self, id: TableId) -> Option<&Table> {
        let location = self.table_location_by_id(id)?;
        Some(&self.sheets[location.sheet_index].tables()[location.table_index])
    }

    /// Returns a table column by the stable identities of its table and column.
    pub fn table_column_by_id(
        &self,
        table_id: TableId,
        column_id: TableColumnId,
    ) -> Option<&TableColumn> {
        let location = self.table_column_location_by_id(table_id, column_id)?;
        Some(
            &self.sheets[location.table.sheet_index].tables()[location.table.table_index].columns()
                [location.column_index],
        )
    }

    /// Returns a table column by stable table ID and case-insensitive column name.
    pub fn table_column(&self, table_id: TableId, name: &str) -> Option<&TableColumn> {
        let location = self.table_column_location(table_id, name)?;
        Some(
            &self.sheets[location.table.sheet_index].tables()[location.table.table_index].columns()
                [location.column_index],
        )
    }

    /// Returns the table whose full range contains one worksheet address.
    pub fn containing_table(&self, sheet_id: SheetId, address: CellAddress) -> Option<&Table> {
        let location = self.containing_table_location(sheet_id, address)?;
        Some(&self.sheets[location.sheet_index].tables()[location.table_index])
    }

    pub(crate) fn table_location(&self, name: &str) -> Option<TableLocation> {
        self.table_index.by_display_name(name)
    }

    pub(crate) fn table_location_by_id(&self, table_id: TableId) -> Option<TableLocation> {
        self.table_index.by_id(table_id)
    }

    pub(crate) fn table_column_location_by_id(
        &self,
        table_id: TableId,
        column_id: TableColumnId,
    ) -> Option<TableColumnLocation> {
        self.table_index.column_by_id(table_id, column_id)
    }

    pub(crate) fn table_column_location(
        &self,
        table_id: TableId,
        name: &str,
    ) -> Option<TableColumnLocation> {
        self.table_index.column_by_name(table_id, name)
    }

    pub(crate) fn containing_table_location(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<TableLocation> {
        self.table_index.containing(sheet_id, address)
    }

    /// Returns a sheet using deterministic case-insensitive lookup.
    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        self.sheet_index_by_name(name)
            .map(|index| &self.sheets[index])
    }

    pub(crate) fn sheet_index_by_name(&self, name: &str) -> Option<usize> {
        let key = case_insensitive_key(name);
        self.sheet_name_index.get(key.as_str()).copied()
    }

    /// Returns the workbook date system.
    pub const fn date_system(&self) -> DateSystem {
        self.date_system
    }

    /// Returns calculation metadata without initiating recalculation.
    pub const fn calculation_hints(&self) -> CalculationHints {
        self.calculation_hints
    }

    /// Returns non-sensitive source metadata.
    pub const fn source(&self) -> WorkbookSource {
        self.source
    }

    /// Returns snapshot provenance.
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// Returns the monotonic semantic revision associated with this snapshot.
    pub const fn semantic_revision(&self) -> u64 {
        self.semantic_revision
    }

    /// Returns the versioned, history-independent semantic fingerprint.
    pub fn fingerprint(&self) -> WorkbookFingerprint {
        WorkbookFingerprint::current(
            self.semantic_fingerprint_cancellable(&|| false)
                .expect("non-cancellable fingerprinting cannot be cancelled"),
        )
    }

    pub(crate) fn semantic_fingerprint_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        if let Some(fingerprint) = self.semantic_fingerprint.get() {
            return Ok(*fingerprint);
        }
        let fingerprint =
            crate::calculation::identity::workbook_fingerprint_cancellable(self, cancelled)?;
        let _ = self.semantic_fingerprint.set(fingerprint);
        Ok(*self
            .semantic_fingerprint
            .get()
            .expect("semantic fingerprint was initialized"))
    }

    pub(crate) fn semantic_sheet_tree_fingerprint_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        self.sheet_identity.semantic_fingerprint_cancellable(
            &|entries| {
                crate::calculation::identity::sheet_tree_leaf_fingerprint_cancellable(
                    entries.entries(),
                    entries.len(),
                    cancelled,
                )
            },
            &crate::calculation::identity::sheet_tree_node_fingerprint,
            cancelled,
        )
    }

    pub(crate) const fn with_semantic_revision(mut self, semantic_revision: u64) -> Self {
        self.semantic_revision = semantic_revision;
        self
    }
}

fn sheet_identity_store_cancellable(
    sheets: &[Sheet],
    cancelled: &impl Fn() -> bool,
) -> Result<PersistentRadixMap<Sheet>, ()> {
    PersistentRadixMap::from_sorted_iter_cancellable(
        sheets
            .iter()
            .enumerate()
            .map(|(index, sheet)| (index as u128, sheet.clone())),
        cancelled,
    )
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}
