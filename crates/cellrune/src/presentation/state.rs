use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    CellAddress, Column, Diagnostic, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, Row, SheetId,
    ValidationError,
};

use super::{PhoneticAnnotation, PhoneticProperties, PhoneticRun};

/// A validated frozen-pane position expressed as fixed row and column counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrozenPane {
    frozen_rows: u32,
    frozen_columns: u32,
}

impl FrozenPane {
    /// Validates and constructs a frozen-pane position.
    ///
    /// `(0, 0)` represents no frozen pane. A nonzero count must leave room for the derived
    /// `topLeftCell`.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when either count cannot be represented within Excel bounds.
    pub fn new(frozen_rows: u32, frozen_columns: u32) -> Result<Self, ValidationError> {
        if frozen_rows >= EXCEL_MAX_ROWS {
            return Err(ValidationError::FrozenRowsOutOfRange { value: frozen_rows });
        }
        if frozen_columns >= EXCEL_MAX_COLUMNS {
            return Err(ValidationError::FrozenColumnsOutOfRange {
                value: frozen_columns,
            });
        }
        Ok(Self {
            frozen_rows,
            frozen_columns,
        })
    }

    /// Returns the number of rows fixed above the scrollable pane.
    pub const fn frozen_rows(self) -> u32 {
        self.frozen_rows
    }

    /// Returns the number of columns fixed to the left of the scrollable pane.
    pub const fn frozen_columns(self) -> u32 {
        self.frozen_columns
    }

    /// Returns whether this value represents no frozen pane.
    pub const fn is_clear(self) -> bool {
        self.frozen_rows == 0 && self.frozen_columns == 0
    }
}

/// One source column range carrying a default phonetic visibility flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnPhoneticVisibility {
    first: Column,
    last: Column,
    visible: bool,
}

impl ColumnPhoneticVisibility {
    /// Constructs a visibility range in inclusive column order.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when `first > last`.
    pub fn new(first: Column, last: Column, visible: bool) -> Result<Self, ValidationError> {
        if first > last {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        Ok(Self {
            first,
            last,
            visible,
        })
    }

    /// Returns the first inclusive column.
    pub const fn first(self) -> Column {
        self.first
    }

    /// Returns the last inclusive column.
    pub const fn last(self) -> Column {
        self.last
    }

    /// Returns the explicitly declared default visibility.
    pub const fn visible(self) -> bool {
        self.visible
    }

    pub(crate) const fn contains(self, column: Column) -> bool {
        column.get() >= self.first.get() && column.get() <= self.last.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CellPresentation {
    pub(crate) annotation: Option<Arc<PhoneticAnnotation>>,
    pub(crate) explicit_visibility: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SheetPresentation {
    pub(crate) worksheet_phonetic_properties: Option<PhoneticProperties>,
    pub(crate) row_phonetic_visibility: BTreeMap<Row, bool>,
    pub(crate) column_phonetic_visibility: Vec<ColumnPhoneticVisibility>,
    pub(crate) cell_phonetics: BTreeMap<CellAddress, CellPresentation>,
    pub(crate) frozen_pane: Option<FrozenPane>,
    pub(crate) right_to_left: bool,
}

/// Borrowed effective phonetic metadata for one cell.
#[derive(Debug, Clone, Copy)]
pub struct CellPhonetics<'a> {
    runs: &'a [PhoneticRun],
    properties: Option<&'a PhoneticProperties>,
    explicit_cell_visibility: Option<bool>,
    explicit_row_visibility: Option<bool>,
    explicit_column_visibility: Option<bool>,
    effective_visibility: bool,
}

impl<'a> CellPhonetics<'a> {
    /// Returns the stored phonetic runs in source order.
    pub const fn runs(self) -> &'a [PhoneticRun] {
        self.runs
    }

    /// Returns the string-item phonetic properties, when present.
    pub const fn properties(self) -> Option<&'a PhoneticProperties> {
        self.properties
    }

    /// Returns the Cell `ph` value when the Cell declared it explicitly.
    pub const fn explicit_cell_visibility(self) -> Option<bool> {
        self.explicit_cell_visibility
    }

    /// Returns the source row `ph` value when declared.
    pub const fn explicit_row_visibility(self) -> Option<bool> {
        self.explicit_row_visibility
    }

    /// Returns the applicable source column `phonetic` value when declared.
    pub const fn explicit_column_visibility(self) -> Option<bool> {
        self.explicit_column_visibility
    }

    /// Returns visibility after applying Cell, row, and column precedence.
    pub const fn effective_visibility(self) -> bool {
        self.effective_visibility
    }
}

/// XLSX presentation metadata kept separate from calculation semantics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentPresentation {
    pub(crate) sheets: BTreeMap<SheetId, SheetPresentation>,
    diagnostics: Vec<Diagnostic>,
    revision: u64,
}

impl DocumentPresentation {
    /// Returns the monotonic presentation revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns presentation-only compatibility diagnostics.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns the default frozen pane for a sheet.
    pub fn frozen_pane(&self, sheet_id: SheetId) -> Option<FrozenPane> {
        self.sheets
            .get(&sheet_id)
            .and_then(|sheet| sheet.frozen_pane)
    }

    /// Returns worksheet-level phonetic display properties.
    pub fn worksheet_phonetic_properties(&self, sheet_id: SheetId) -> Option<&PhoneticProperties> {
        self.sheets
            .get(&sheet_id)
            .and_then(|sheet| sheet.worksheet_phonetic_properties.as_ref())
    }

    /// Returns the explicit row phonetic visibility, when declared.
    pub fn row_phonetic_visibility(&self, sheet_id: SheetId, row: Row) -> Option<bool> {
        self.sheets
            .get(&sheet_id)
            .and_then(|sheet| sheet.row_phonetic_visibility.get(&row).copied())
    }

    /// Returns source column-range phonetic visibility declarations in document order.
    pub fn column_phonetic_visibility(&self, sheet_id: SheetId) -> &[ColumnPhoneticVisibility] {
        self.sheets
            .get(&sheet_id)
            .map_or(&[], |sheet| sheet.column_phonetic_visibility.as_slice())
    }

    /// Resolves phonetic runs and explicit/effective visibility for one cell.
    pub fn cell_phonetics(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<CellPhonetics<'_>> {
        let sheet = self.sheets.get(&sheet_id)?;
        let cell = sheet.cell_phonetics.get(&address)?;
        let row = sheet.row_phonetic_visibility.get(&address.row()).copied();
        let column = sheet
            .column_phonetic_visibility
            .iter()
            .rev()
            .find(|range| range.contains(address.column()))
            .map(|range| range.visible());
        let effective = cell.explicit_visibility.or(row).or(column).unwrap_or(false);
        let annotation = cell.annotation.as_deref();
        Some(CellPhonetics {
            runs: annotation.map_or(&[], PhoneticAnnotation::runs),
            properties: annotation.and_then(PhoneticAnnotation::properties),
            explicit_cell_visibility: cell.explicit_visibility,
            explicit_row_visibility: row,
            explicit_column_visibility: column,
            effective_visibility: effective,
        })
    }

    /// Returns whether no presentation metadata or diagnostics are present.
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty() && self.diagnostics.is_empty()
    }

    pub(crate) fn sheet_mut(&mut self, sheet_id: SheetId) -> &mut SheetPresentation {
        self.sheets.entry(sheet_id).or_default()
    }

    pub(crate) fn sheet(&self, sheet_id: SheetId) -> Option<&SheetPresentation> {
        self.sheets.get(&sheet_id)
    }

    pub(crate) fn cell_presentation(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<&CellPresentation> {
        self.sheet(sheet_id)
            .and_then(|sheet| sheet.cell_phonetics.get(&address))
    }

    pub(crate) fn cell_presentations(&self) -> impl Iterator<Item = &CellPresentation> {
        self.sheets
            .values()
            .flat_map(|sheet| sheet.cell_phonetics.values())
    }

    pub(crate) fn source_frozen_pane(&mut self, sheet_id: SheetId, pane: FrozenPane) {
        self.sheet_mut(sheet_id).frozen_pane = Some(pane);
    }

    pub(crate) fn source_right_to_left(&mut self, sheet_id: SheetId, right_to_left: bool) {
        if right_to_left {
            self.sheet_mut(sheet_id).right_to_left = true;
        }
    }

    pub(crate) fn source_worksheet_phonetic_properties(
        &mut self,
        sheet_id: SheetId,
        properties: PhoneticProperties,
    ) {
        self.sheet_mut(sheet_id).worksheet_phonetic_properties = Some(properties);
    }

    pub(crate) fn source_row_phonetic_visibility(
        &mut self,
        sheet_id: SheetId,
        row: Row,
        visible: bool,
    ) {
        self.sheet_mut(sheet_id)
            .row_phonetic_visibility
            .insert(row, visible);
    }

    pub(crate) fn source_column_phonetic_visibility(
        &mut self,
        sheet_id: SheetId,
        visibility: ColumnPhoneticVisibility,
    ) {
        self.sheet_mut(sheet_id)
            .column_phonetic_visibility
            .push(visibility);
    }

    pub(crate) fn source_cell_phonetics(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        annotation: Option<Arc<PhoneticAnnotation>>,
        explicit_visibility: Option<bool>,
    ) {
        if annotation.is_some() || explicit_visibility.is_some() {
            self.sheet_mut(sheet_id).cell_phonetics.insert(
                address,
                CellPresentation {
                    annotation,
                    explicit_visibility,
                },
            );
        }
    }

    pub(crate) fn right_to_left(&self, sheet_id: SheetId) -> bool {
        self.sheet(sheet_id)
            .is_some_and(|sheet| sheet.right_to_left)
    }

    pub(crate) fn set_frozen_pane(
        &mut self,
        sheet_id: SheetId,
        pane: Option<FrozenPane>,
    ) -> Result<bool, ValidationError> {
        let current = self.frozen_pane(sheet_id);
        if current == pane {
            return Ok(false);
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(ValidationError::PresentationRevisionExhausted)?;
        self.sheet_mut(sheet_id).frozen_pane = pane;
        self.revision = next_revision;
        self.retain_nonempty_sheets();
        Ok(true)
    }

    pub(crate) fn has_cell_annotation(&self, sheet_id: SheetId, address: CellAddress) -> bool {
        self.sheet(sheet_id)
            .and_then(|sheet| sheet.cell_phonetics.get(&address))
            .is_some_and(|cell| cell.annotation.is_some())
    }

    pub(crate) fn set_cell_phonetics(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        annotation: Arc<PhoneticAnnotation>,
        explicit_visibility: bool,
    ) -> Result<bool, ValidationError> {
        let next = CellPresentation {
            annotation: Some(annotation),
            explicit_visibility: Some(explicit_visibility),
        };
        if self
            .sheet(sheet_id)
            .and_then(|sheet| sheet.cell_phonetics.get(&address))
            == Some(&next)
        {
            return Ok(false);
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(ValidationError::PresentationRevisionExhausted)?;
        self.sheet_mut(sheet_id)
            .cell_phonetics
            .insert(address, next);
        self.revision = next_revision;
        Ok(true)
    }

    pub(crate) fn clear_cell_phonetics(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Result<bool, ValidationError> {
        if self
            .sheet(sheet_id)
            .and_then(|sheet| sheet.cell_phonetics.get(&address))
            .is_none()
        {
            return Ok(false);
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(ValidationError::PresentationRevisionExhausted)?;
        if let Some(sheet) = self.sheets.get_mut(&sheet_id) {
            sheet.cell_phonetics.remove(&address);
        }
        self.revision = next_revision;
        self.retain_nonempty_sheets();
        Ok(true)
    }

    pub(crate) fn semantics_match(&self, other: &Self) -> bool {
        self.sheets == other.sheets
    }

    pub(crate) fn retain_nonempty_sheets(&mut self) {
        self.sheets.retain(|_, sheet| {
            sheet.worksheet_phonetic_properties.is_some()
                || !sheet.row_phonetic_visibility.is_empty()
                || !sheet.column_phonetic_visibility.is_empty()
                || !sheet.cell_phonetics.is_empty()
                || sheet.frozen_pane.is_some()
                || sheet.right_to_left
        });
    }

    pub(crate) fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CellPresentation, ColumnPhoneticVisibility, DocumentPresentation};
    use crate::{
        CellAddress, Column, PhoneticAnnotation, PhoneticRun, PhoneticTextRange, Row, SheetId,
    };

    #[test]
    fn cell_visibility_resolves_cell_then_row_then_column() {
        let sheet_id = SheetId::new(1).expect("sheet");
        let address = CellAddress::from_a1("B2").expect("address");
        let run =
            PhoneticRun::new(PhoneticTextRange::new(0, 1).expect("range"), "か").expect("run");
        let mut presentation = DocumentPresentation::default();
        let sheet = presentation.sheet_mut(sheet_id);
        sheet.column_phonetic_visibility.push(
            ColumnPhoneticVisibility::new(
                Column::new(1).expect("A"),
                Column::new(3).expect("C"),
                true,
            )
            .expect("range"),
        );
        sheet
            .row_phonetic_visibility
            .insert(Row::new(2).expect("row"), false);
        sheet.cell_phonetics.insert(
            address,
            CellPresentation {
                annotation: Some(Arc::new(PhoneticAnnotation::new(vec![run], None))),
                explicit_visibility: None,
            },
        );

        let resolved = presentation
            .cell_phonetics(sheet_id, address)
            .expect("phonetics");
        assert_eq!(resolved.explicit_column_visibility(), Some(true));
        assert_eq!(resolved.explicit_row_visibility(), Some(false));
        assert!(!resolved.effective_visibility());
    }
}
