# Changelog

All notable changes to CellRune are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Releases follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file records concise user-visible and release-operator changes. Design rationale, test
inventories, and measurements belong in the linked documentation rather than in release entries.

## [Unreleased]

### Added

- `WorkbookChange::delete_sheet(SheetId)` and `WorkbookDraft::delete_sheet` delete a sheet with
  the names scoped to it and its tables, and rewrite the remaining cell formulas, defined names,
  and table formulas as Excel saves them: a reference to the sheet or to one of its tables becomes
  `#REF!` (`SUM('S2'!A1:A3)` becomes `SUM(#REF!)`), a deleted end of a 3-D reference moves to the
  nearest remaining worksheet, passing over chartsheets (`'S2:S3'!A1` becomes `'S3'!A1`), and
  `INDIRECT` text is untouched. A qualified use of a name scoped to the deleted sheet becomes
  `#REF!` where Excel writes `[0]!Name`. Data validation formulas on other sheets that name it
  become `#REF!` in the model, as in the saved part, and the sheet's data validations, hyperlinks,
  comments, cell formats, column widths, and row heights leave the draft. Deleting the last
  visible sheet fails with `validation.last_visible_sheet`. The receipt reports a topology change,
  so a session recalculates fully.
- Saving a document draft removes a deleted sheet's entry and every package part only it reached,
  a pivot cache once all of its pivot tables went, and their content-type overrides, renumbers
  `localSheetId`, and moves `workbookView` `activeTab` and `firstSheet` past the deleted tab; a
  deleted active sheet hands activity to the nearest visible sheet after it, else before it, as
  Excel does. Relationship targets that cannot be parts, such as cell links in drawing
  relationships, and relationships to parts the package lacks are ignored. Conditional formats,
  data validations, and chart series that name the sheet become `#REF!`, a sparkline reading it
  loses its data source, and internal hyperlinks keep their location, as in Excel. A remaining
  pivot cache or pivot chart that reads the deleted sheet, a form control (`fmlaLink`,
  `fmlaRange`), VML control (`x:FmlaLink`, `x:FmlaRange`), linked shape (`textlink`), or data
  consolidation (`dataRef`) that names it, or a slicer or timeline cache while it holds a pivot
  table or table, fails with `xlsx.write.unsupported_preservation` and a detail naming the parts.
  Saving edits without calculating also clears the results of formulas that read the sheet list or
  a 3-D range across the deleted sheet.
- `WorkbookSnapshot::highest_sheet_id()` returns the highest sheet ID a workbook has used,
  including sheets deleted in the same draft or session; added sheets take the next ID, so within
  a draft or session a deleted sheet's ID is never reused, even by a later change of the batch
  that deleted it. A saved file does not record deleted IDs, so after reopening it an added sheet
  can take one.
- `WorkbookChange::move_sheet(SheetId, position)` and `WorkbookDraft::move_sheet` move a
  worksheet or chartsheet to another tab position. Sheet IDs and scoped names stay with their
  sheets and the active sheet stays the same sheet. A 3-D reference keeps its text and follows tab
  order, unless the move reverses its ends: the moved end then becomes its neighbour inside the old
  span, as Excel 16.114 saves it (`'S2:S5'!A1` becomes `'S3:S5'!A1` when `S2` moves after `S5`).
  Saving into the source package reorders the `<sheet>` entries, renumbers `localSheetId`, and
  keeps `activeTab` and `firstSheet` on their sheets. A position past the last tab fails with the
  new `validation.sheet_position_out_of_range`.
- `WorkbookChange::rename_defined_name(scope, name, new_name)` renames a defined name and rewrites
  the uses that resolve to it, by Excel's scope rules, in cell formulas, defined names, table
  formulas, and data validations, and on save in preserved conditional formats, data validations,
  x14 formulas, chart series, and pivot cache sources that read a workbook name. Removing a name and
  adding it under a new one left those uses as `#NAME?`. A rename fails with the new
  `validation.unknown_defined_name` for a missing name and `validation.defined_name_rename_shadowed`
  when the other scope already uses the new name.
- `WorkbookChange::create_table(SheetId, CellRange, Option<TableName>)` creates a table whose first
  row is its header, naming blank headers `Column1`, `Column2`, and so on, with Excel's default
  name `Table` and the number after the workbook's table count. Saving into the source package adds
  the table part. A range without a data row fails with the new `validation.table_range_invalid`,
  and one over a merged range or legacy array with the new `validation.table_range_overlap`.
- `WorkbookChange::set_table_range(TableId, CellRange)` grows or shrinks a table by rows and columns
  in one edit. A structured reference that names a removed column, alone or as an end of a column
  range, becomes `#REF!`, as Excel 16.114 saves it.
- `WorkbookChange::convert_table_to_range(TableId)` converts a table to a plain range. Its cells
  keep their values and formulas, and every structured reference to it becomes a sheet-qualified
  absolute reference, as Excel 16.114 saves it (`Things[val]` becomes `'S2'!$D$2:$D$4`, `[@val]`
  becomes `'S2'!$D2`). Saving into the source package removes the table part and its entries, and
  fails with `xlsx.write.unsupported_preservation` naming the part when a preserved pivot cache
  reads the table.
- `WorkbookChange::set_sheet_auto_filter(SheetId, Option<CellRange>)` creates, changes, or removes a
  sheet-level autoFilter together with its hidden `_xlnm._FilterDatabase` name, keeping the
  criteria of the columns a changed filter still covers. The canonical writer now writes a sheet's
  `<autoFilter>`.
- `WorkbookChange::copy_sheet(SheetId, position)` and `WorkbookDraft::copy_sheet` copy a worksheet
  within its workbook to a tab position as the next sheet ID, named `Name (2)` or the next free
  number, and a name already ending in a number in parentheses counts on from it (`Data (2)` copies
  as `Data (3)`), as Excel 16.114 saves it. In the copy, single-sheet references to the source name the
  copy while other sheets' and 3-D references keep their text; its tables take new IDs and Excel's
  numbered names (`Things` becomes `Things4` beside three tables), with the structured references
  inside the copy following them; the source's scoped names are copied to the copy, and a workbook
  name that reads the source becomes a name of the copy too. Saving into the source package
  duplicates the parts the worksheet reaches, with chart series naming the copy and table parts
  following later changes to the copied tables' ranges; the copy's part replays the row, column,
  and range edits of the copy and those its source had before the copy, and the source's part
  only its own. It fails with
  `xlsx.write.unsupported_preservation` naming the part for a pivot table, form control, threaded
  comment, slicer, timeline, query table, or ActiveX control. A table created on the copy, or on
  its source before the copy, is written as a new part, and a table converted to a range there
  leaves the copy's parts. The copy does not take its source's tab selection.
- Edit schema v1 accepts `move_sheet`, `copy_sheet`, `set_sheet_auto_filter`, and
  `rename_defined_name`, and v2 accepts `create_table`, `set_table_range`, and
  `convert_table_to_range`, so interop, Python, Node.js, and the MCP `workbook_apply_changes` tools
  expose the sheet, name, and table edits. A copy reports its ID in `created_sheet_ids`, and the
  tables `create_table` and `copy_sheet` create are in `changed_table_ids`.
- `WorkbookChange::insert_rows(SheetId, Row, count)`, `delete_rows(SheetId, first, last)`,
  `insert_columns`, `delete_columns`, `move_rows(SheetId, first, last, destination)`, and
  `move_columns` edit a sheet's grid as Excel 16.114 does. References in cell formulas, defined
  names, and table formulas follow the cells they read; a reference to deleted cells becomes
  `#REF!` (`'S1'!#REF!` from another sheet) and a range a deletion reaches in part shrinks; an
  insertion at a range's first line moves it and one inside it widens it; a moved range follows
  its ends. 3-D references, `INDIRECT` text, and internal hyperlink locations keep their text.
  Merged ranges, tables, autoFilters, conditional formats, data validations, comments,
  hyperlinks, spill and array ranges, row heights, and column widths move with their cells, and
  inserted lines take the cell formats of the line before them; in a calculation session those
  copied formats count against the table materialization limit
  (`session.table_materialization_limit_exceeded`). As Excel 16.114 does, a column deletion removes
  the table columns it reaches, turning structured references naming them, alone or as an end of a
  column range, into `#REF!`, and removes a table it reaches whole with every reference to it; a
  row move within a table's data reorders the rows and keeps the table. An edit fails with the new
  `validation.structural_edit_empty`, `validation.content_shifted_off_sheet`,
  `validation.merged_range_split`, `validation.table_header_shift`,
  `validation.table_data_rows_deleted`, or `validation.table_split` where Excel refuses it, and a
  column deletion reaching a table with filter criteria, a sort, or more than one header row with
  `validation.unsupported_table_authoring_metadata`.
- `WorkbookChange::move_range(SheetId, CellRange, CellAddress)` cuts and pastes cells within a
  sheet: references to the moved cells follow them, a range only partly moved keeps its text, and
  moved formulas keep theirs. As Excel 16.114 does, a reference to cells the paste overwrites
  becomes `#REF!` when they cover it, and a range shrinks to its part left when they cover its
  whole width or height and run past one of its ends, while a range they only reach the end of or
  lie inside keeps its text. Moving part of the cells of a data validation or conditional format
  splits it as Excel does: the cells left keep the rule, and the moved cells get a rule of their
  own whose formulas read as the rule did there, following the move (`A1>B1` on A1:A6 with A2:A3
  cut to D2 adds `D2>B2` on D2:D3), with new conditional-format priorities; a data bar, icon set,
  or color scale paired with an extension rule by ID is refused at save with
  `xlsx.write.unsupported_preservation`. Cutting part of a spill range fails with
  `validation.array_formula_part_edit`, as Excel refuses it.
- `WorkbookChange::copy_range(SheetId, CellRange, SheetId, CellRange)` copies or fills a range
  onto a destination that repeats it a whole number of times, shifting the relative parts of
  references, so `SUM(A2:A5)` copied from B2 to C6 becomes `SUM(B6:B9)` and a reference shifted
  off the grid becomes `#REF!`. Another shape fails with the new
  `validation.copy_destination_shape`, and a spill reference, legacy array formula, or data-table
  formula with the new `validation.copy_unsupported_formula`.
- `EditReceipt::inverse()` returns, for a batch with these edits or with a sheet deletion, move, or
  copy, a defined-name rename, or a table creation, range change, conversion to a range, or sheet
  autoFilter change, an `EditInverse`: the changes that undo the batch when applied as one batch to
  its result, each structural change undone by its own reverse in reverse order (a deleted sheet
  added again under its name at its position and filled from its cells, formats, names, and
  tables), and the `UnrestoredContent` they cannot bring back by kind, sheet, and table; preserved
  package content a deletion or move changes is listed. An inverse without changes means the batch
  left nothing to undo. The engine keeps no undo stack, and a batch whose undo needs an edit the
  workbook refuses has no inverse. Interop and
  Python receipts
  carry it as `inverse`, edit-schema-v2 `changes` and `unrestored` (a `set_formula` change's new
  optional `dynamic_array` restores a dynamic-array formula without a known spill range), Node.js
  receipts as camel-case
  `applyChangesV2` changes, and the MCP `workbook_apply_changes` tools return it; interop edit
  schema v1, Python, and MCP accept the new `insert_rows`, `delete_rows`, `insert_columns`,
  `delete_columns`, `move_rows`, `move_columns`, `move_range`, and `copy_range` kinds, and Node.js
  `insertRows`, `deleteRows`, `insertColumns`, `deleteColumns`, `moveRows`, `moveColumns`,
  `moveRange`, and `copyRange`.
- Saving into the source package rewrites the edited worksheet's rows, cells, and dimension, its
  drawing anchors, notes, comments, and threaded comments, and the qualified references other
  sheets, charts, and pivot caches make to it. It fails with `xlsx.write.unsupported_preservation`
  naming the part for coordinates it cannot rewrite, a pivot table the edit moves or splits, a
  form control it moves, a linked shape or control that names cells it moves, a conditional
  format part of which a range move cuts while its formula reads relative to its cells, a pivot
  cache that reads cells it moves, and a sheet copy whose source or copy has such edits.
- Interop, Python, Node.js, and MCP expose the sheet autoFilter and table column widening edits,
  frozen panes, and phonetic annotations. Edit schema v2 adds `widen_table_columns`,
  `widen_sheet_auto_filter`, and the presentation-only `set_frozen_pane`, `set_phonetics`, and
  `clear_phonetics` operations (Node.js camel case), which are also the new `WorkbookChange`
  kinds `SetFrozenPane`, `SetPhonetics`, and `ClearPhonetics`. Sheet summaries report a sheet's
  `frozen_pane` when it has one, and Python and Node.js `phonetics`, interop
  `WorkbookSession::phonetics`, and the new `workbook_phonetics` MCP tool page through a sheet's
  phonetic runs.
- Limit failures report their figures on every surface: errors carry an optional `details.limit`
  with the exceeded phonetic run limit's `name`, the read error's `observed` value, and targeted
  calculation's `requested_cell_count` or `evaluated_count`, and unavailable results and
  calculation issues that exceeded a calculation limit name it in an optional `limit`. Both are
  omitted when there is nothing to report, so other responses are unchanged.
  `CalculationLimitKind::as_str` and `PhoneticRunLimitKind::as_str` return the limit names.
- Python `read_values` and `write_values` and Node.js `readValues` and `writeValues`, over interop
  `WorkbookSession::read_values` and `write_values`, read a rectangle of up to 1,048,576 cells as
  rows of typed values and write equal-length rows of values as one atomic edit batch, without a
  numpy or Arrow dependency. A read returns each cell's current calculated result, or its
  `unavailable` result, once the workbook has calculated the current revision, and otherwise its
  literal or saved value.
- MCP `workbook_read_range`, interop `WorkbookSession::read_range_with_options`, Python
  `read_range`, and Node.js `readRange` take optional forms: `omit_empty` leaves out empty cells,
  `values_only` returns address and value pairs in `values`, `compact` returns `rows` of values
  grouped by worksheet row, and `max_bytes` returns the longest prefix that fits, failing with the
  new `interop.page.byte_limit_exceeded` code when not even one position does; a Python
  `max_bytes` that is not a non-negative integer fails with the new
  `interop.page.byte_limit_invalid`. Offset and limit
  still count positions of the range, and a read without them returns the same page as before.
- Node.js `calculate`, `recalculate`, `calculateTargets`, and `previewChanges` take an optional
  `signal` (`AbortSignal`) that cancels that call cooperatively; the Promise rejects with the
  existing `session.cancelled`, or `calculation.target.cancelled` for targeted calculation, and an
  already aborted signal cancels before calculating. Interop
  `WorkbookSession::prepare_recalculation_cancellable` takes the caller's token.
- `WorkbookCalculationSession::precedents` and `dependents` report the rectangles a cell's formula
  reads and the formulas that read a cell directly, from the dependency graph of the calculation
  installed for the current revision, as `CellPrecedents` (`Precedent` with a `PrecedentKind`) and
  `CellDependents`. Formulas whose references depend on values, such as `INDIRECT` and `OFFSET`,
  are marked dynamic, and a dependents result counts the dynamic formulas it cannot list. Interop
  `WorkbookSession::precedents` and `dependents`, Python and Node.js `precedents` and
  `dependents`, and the new `workbook_precedents` and `workbook_dependents` MCP tools page through
  them and fail with `interop.calculation.required` before the current revision is calculated.
- The sessionless `function_catalog` MCP tool pages through the calculated functions with each
  name's canonical name and alias, array-result, and official-list flags, over the new interop
  `function_catalog_page`.
- `docs/ERROR_CODES.md` lists every stable error and calculation issue code by surface: the Rust
  core enums, interop, MCP, and the Node.js wrapper.
- `bindings/scripts/mcp_registry.py` assembles one MCP Bundle, `cellrune-mcp-<version>.mcpb`, from
  a release's three verified `cellrune-mcp` archives, running the platform's executable on one
  approved directory, and writes the official MCP registry `server.json` that names that bundle's
  release download URL and SHA-256. The release workflow attaches the bundle to the GitHub release
  beside the three archives and, behind the `mcp-registry` environment, publishes that entry with
  `mcp-publisher` over GitHub OIDC, skipping a version the registry already lists; CI runs the
  bundle and entry tests.
- Sheet deletion is the `delete_sheet` change kind in interop `WorkbookChangeDto::DeleteSheet`,
  Python (`DeleteSheetChange`), and MCP `workbook_apply_changes` and `workbook_apply_changes_v2`,
  `deleteSheet` in Node.js, and the `WorkbookSession::delete_sheet`, Python `delete_sheet`, and
  Node.js `deleteSheet` methods. Deltas name a deleted sheet's removed results by the name it had.
- `SKEW`, `SKEW.P`, and `KURT` calculate the sample skewness, population skewness, and sample
  kurtosis Microsoft documents, collecting numbers as `STDEV.S` does. Fewer than three numbers
  (four for `KURT`) or a zero standard deviation returns `#DIV/0!`.
- `Sheet::data_validations()` reads every worksheet data validation rule as a read-only model:
  `DataValidation` exposes its ranges, `DataValidationType` (any value, whole, decimal, list, date,
  time, text length, custom), `DataValidationOperator`, `formula1`/`formula2` as stored,
  `allow_blank`, `in_cell_dropdown` (the inverse of OOXML `showDropDown`), the input and error
  message flags and texts, and `DataValidationErrorStyle`. Rules come from `<dataValidations>` and
  from the Excel 2010 `x14:dataValidations` extension, which Excel uses for lists that name another
  sheet. A rule with a malformed range list, a repeated formula, or an unknown token is left
  unmodeled and its XML is preserved as before. A sheet rename rewrites the rules' formulas as the
  saved part rewrites them, and rules are not part of the semantic fingerprint.
- The rules are paged by `WorkbookSession::data_validations` in interop, Python
  `Workbook.data_validations`, Node.js `Workbook.dataValidations`, and the MCP tool
  `workbook_data_validations`, each taking a sheet name, `offset`, and `limit`.
- A minimal cell formatting model: `WorkbookDraft::cell_format(sheet_id, address)` returns the
  `CellFormat` (`Font`, `Fill`, `Border`, `Alignment`, with `Color` as an ARGB, theme, indexed, or
  automatic color and a tint) a cell displays with, resolved as Excel does from the cell's own
  style, else a blank cell's, its row's, or its column's. `DocumentPresentation::column_widths`,
  `column_width`, `row_heights`, and `row_height` read explicit widths and heights; a `<col>`
  without `customWidth` at its sheet's standard width, as Excel writes a column kept only for its
  style, has no width of its own. Package-backed
  documents capture them on open; a gradient fill reads as `Fill::Gradient` and its stops stay in
  the source record.
- `WorkbookChange::set_cell_format` (a `CellFormatEdit` whose font, border, and alignment parts
  change only the properties they name, and whose fill replaces the fill), `set_column_width`, and
  `set_row_height` are presentation changes: they advance the presentation
  revision, keep the semantic revision and any calculation current, and validate their values
  with the new `validation.font_name_invalid` and `validation.format_value_out_of_range` codes.
  The document writer adds only the font, fill, border, and `cellXfs` records an edit needs,
  reusing an identical existing record and placing new fills after the two reserved ones, keeping
  every unedited record, attribute, and component of the source style; it splits `<col>`
  spans, gives a column whose width is cleared but that keeps a style the standard width, since
  Excel reads a `<col>` without `width` as a hidden column, writes rows that gain a height, and keeps an edited cell's format when its content is
  cleared. The canonical writer writes the same model, and both verify the reopened output. A
  session rejects a batch prepared before another presentation-only batch was installed with
  `session.revision_mismatch`, so neither batch's edits are lost.
- `Sheet::hyperlinks()` and `Sheet::comments()` read worksheet hyperlinks (`Hyperlink`: range,
  external target from the worksheet relationship, in-workbook location, display text, screen
  tip) and cell comments (`Comment`: address, author, plain text without phonetic runs) as
  read-only metadata. An Excel 365 comment thread reads as one `Comment` with the author's display
  name from the persons part and its other comments as `replies()` (`CommentReply`: author, text)
  in place of the placeholder note Excel writes for it. These parts were not read before, so a hyperlink with a malformed `ref`, a
  relationship part that does not parse, or a comments part that does not read leaves only that
  metadata out and never fails the read. A sheet rename rewrites internal hyperlink locations as
  the saved part does. Interop `WorkbookSession::hyperlinks`/`comments`, Python
  `Workbook.hyperlinks`/`comments`, Node.js `Workbook.hyperlinks`/`comments`, and the MCP tools
  `workbook_hyperlinks` and `workbook_comments` page through them; a thread's entry carries
  `replies`, omitted for a note.
- RFC 4180 CSV import and export of one sheet: `read_csv(bytes, &CsvReadOptions)` returns a
  one-sheet `WorkbookDraft`, and `write_csv(workbook, calculation, sheet_id, CsvWriteOptions)`
  writes UTF-8 with CRLF records from A1 through the last used cell, formulas as their calculated
  values. Reading never creates a formula: unquoted decimal numbers, `TRUE`/`FALSE`, and Excel error
  spellings become numbers, logicals, and errors, quoted fields and everything else text, and
  writing quotes text that would read back as anything else, including text that starts with a
  byte order mark, so a write and a read keep every value at its address. The delimiter is `,`,
  `;`, tab, `|`, or a space; a field longer than 32,767 UTF-16 code units fails with
  `csv.read.field_too_long`, and a sheet without formulas exports without a calculation. Failures
  carry the new `csv.*` codes. Interop
  `WorkbookSession::open_csv_bytes`/`save_csv_bytes`, Python `Workbook.from_csv`/`to_csv`,
  Node.js `Workbook.fromCsv`/`toCsv`, and the MCP tools `workbook_open_csv` and
  `workbook_export_csv` expose them.
- Interop `WorkbookSession::read_formats` returns a row-major `CellFormatPageDto` with the
  range's column widths and the page's row heights, the longest complete prefix that fits the
  response limit when it is bounded, and edit schema v2 accepts `set_cell_format`,
  `set_column_width`, and `set_row_height`. Python `Workbook.read_formats`, Node.js
  `Workbook.readFormats`, the MCP tool `workbook_read_formats`, and every surface's
  `apply_changes_v2` expose them.
- `TEXT` formats with Excel number-format codes: up to four sections with `[<=100]`-style
  conditions, colors and `[Color1]` through `[Color56]` (accepted and not shown), quoted and
  escaped literals, `General` for numbers of up to 11 characters, date and time codes in both date
  systems including serial 60 as 1900-02-29, elapsed `[h]`, `[m]`, and `[s]`, `AM/PM` and `A/P`,
  fractional seconds, scientific notation, `# ?/?` fractions, trailing-comma thousands scaling,
  `?` space padding, `_` spacing, `*` fills (dropped, as Excel's `TEXT` drops them), and literals
  between digit placeholders such as `000-00-0000`. Codes 0.1.26 already formatted keep their
  results, including its rounding, its `#NUM!` from 2^53 rounded units, and `TEXT(1,"0.#")` as
  `1`; other codes follow Excel as measured, so `TEXT(1,"0.#;-0.#")` is `1.` and
  `TEXT(1.999,"# ?/?")` is `2    `. Fixed fraction denominators such as `# ?/8`, conditional
  sections that select a negative number, a third section after one without a condition, elapsed
  codes beside `AM/PM`, lowercase `am/pm`, era, Buddhist-year, and weekday-letter codes, system
  date formats, runs of unquoted letters that are not codes, `General` numbers Excel would shorten,
  and numbers past 2^53 rounded units in the other codes report
  `calculation.unsupported_expression`. A lone unquoted letter that is not a code shows as written
  and a lone `n`, `N`, `B`, or `E` is `#VALUE!`, as in Excel 16.114 for Mac and Excel for the web:
  `TEXT(0,"0.00;-0.00;zero")` is `z1900ro`, where Mac Excel 2021 shows `z3333o`. `yyyyy` is the
  four-digit year. A text value is still `#VALUE!`;
  formatting text, as Excel's `TEXT("abc","@")` does, is not in 0.1.27.
- `CalculationOptions::with_locale_profile(LocaleProfile)` selects built-in locale conventions
  instead of the host's: `Locale::EnUs`, the default, which keeps every earlier result, or
  `Locale::KoKr`. The regional format decides the `DOLLAR` currency symbol (`₩` in Korean, which
  Excel 16.114 writes as `₩1.50` and `(₩1.50)` with the default two decimals), the names `TEXT`
  shows, and the double-byte rules of the byte functions; `LocaleProfile::with_display_language`
  sets the language of the `GROUPBY` and `PIVOTBY` total labels (`합계` in Korean) apart. Korean
  month, weekday, and `AM`/`PM` names and the Korean label of a grand total over several levels
  report `calculation.unsupported_expression` until measured. The Excel
  oracle audit calculates each host profile with its locale conventions.
- The locale profile is `CalculationOptionsDto::locale_profile`, a `LocaleProfileDto` of `"en-US"`
  or `"ko-KR"` tags with an optional `display_language`, and the transaction report's calculation
  options record a profile other than the default `en-US` one, so reports under the default are
  unchanged. Python `locale_profile=` and Node.js `localeProfile` on
  calculate, recalculate, targeted calculation, and previews, and MCP `locale_profile` on
  `workbook_recalculate`, `workbook_calculate_targets` options, and `workbook_preview_changes`
  carry it. An unknown tag or key from any Python call or from Node.js `calculate` and
  `recalculate` reports the new `interop.calculation.locale_profile_invalid`, and Python reads a
  `display_language` of `None` as absent; Node.js targeted and preview payloads and MCP payloads
  fail as invalid payloads.
- `format_display_text` returns the text a cell shows for a value, a number-format code, a date
  system, and a locale profile, with the `TEXT` engine for numbers and the code's text section for
  text. Unlike `TEXT`, which keeps the 0.1.26 results of the codes it already formatted, it shows
  what Excel shows for those codes too: nothing for `0` in `#` and the decimal point of `1` in
  `0.#`; codes it cannot render, including a `*` fill, which needs the column width, and a
  `General` number longer than 11 characters, report `display_format.unsupported`. Interop
  `format_display_text`, Python `format_display_text`, Node.js `formatDisplayText`, and the MCP
  `format_display_text` tool expose it, and a request a binding cannot read reports
  `interop.display_format.request_invalid`.
- The reader reads `<row hidden>`, including on rows that omit `r`, into `Sheet::hidden_rows` and
  `Sheet::is_row_hidden`, and which worksheet-level autoFilter columns declare a filter condition
  into `SheetAutoFilter::filtered_column_ids`; Excel stores the rows a filter hides as hidden rows.
  The sheet summaries of interop, Python, Node.js, and MCP report `hidden_rows` as A1 row ranges and
  `auto_filter_filtered_columns` only when the sheet has them, so other summaries are unchanged. The
  workbook fingerprint folds both only when present, so
  workbooks without them keep their fingerprints, and a document-backed write verifies that its
  output keeps the hidden rows.
- `FIXED(number, [decimals], [no_commas])` rounds to `decimals` places (default 2, at most 127;
  negative values round left of the decimal point) and returns the text with `,` grouping unless
  `no_commas` is true.
- `LENB`, `LEFTB`, `RIGHTB`, `MIDB`, `FINDB`, `SEARCHB`, and `REPLACEB` count bytes, and `ASC` and
  `DBCS` convert between half-width and full-width forms. Without a double-byte locale the byte
  functions count UTF-16 code units, as Excel does (`LENB("😀")` is 2), and `ASC` and `DBCS`
  leave text unchanged. Under `Locale::KoKr` ASCII counts one byte and Hangul syllables, Hangul
  compatibility jamo, full-width ASCII forms, and the ideographic space count two; `ASC` turns
  full-width ASCII forms and the ideographic space into ASCII, and `DBCS` does the reverse. Other
  Korean characters and positions inside a surrogate pair or a double-byte character report
  `calculation.unsupported_expression`.
- `AGGREGATE(function_num, options, ref1, …)` and `AGGREGATE(function_num, options, array, k)` apply
  the nineteen documented functions with options that skip hidden rows (1, 3, 5, 7), error values
  (2, 3, 6, 7), and cells holding `SUBTOTAL` or `AGGREGATE` (0 to 3). As in Excel, rows a filter
  hides count as hidden rows, so options 0, 2, 4, and 6 keep them. Functions 1 to 13 take
  references, 14 to 19 one array and `k`, and a calculated array keeps its hidden rows and nested
  totals; 3-D references, a missing `k`, and out-of-range arguments are `#VALUE!`. `SUBTOTAL` is
  unchanged. With these and the other functions added in 0.1.27, the catalog has 495 official
  names and 496 accepted entries.
- `FREQUENCY` and `MODE.MULT` spill one column. `FREQUENCY` counts the numbers at or below each
  bin and above the next lower one, in the given bin order, then those above the highest bin;
  text and blank bins add no row, a logical value in a data array constant is ignored, and an
  error in the data is the whole result, as in Excel. Logical values elsewhere, errors in the bins
  or of two kinds in the data, and equal bins out of order report
  `calculation.unsupported_expression`. `MODE.MULT` returns every most frequent number in
  first-occurrence order, and `#N/A` when none repeats.
- `CHISQ.TEST` (and `CHITEST`), `CHISQ.INV`, and `CHISQ.INV.RT` (and `CHIINV`) calculate the
  chi-square test and both quantiles with Microsoft's argument errors; the right tail is inverted
  directly so tiny probabilities keep their precision, and from 20,000 degrees of freedom both
  quantiles integrate the gamma density so they keep their digits up to the 10^10 limit
  (`CHISQ.INV(0.5,1E9)` is 999999999.3333334, as Excel 16.114 measures). As in Excel, the test accepts tables whose
  counts match in any shape, takes the degrees of freedom from the first table's shape, drops
  pairs with text, and returns `#DIV/0!` for an expected 0 and `#NUM!` for a negative one; blanks,
  logical values, and errors in the tables report `calculation.unsupported_expression`.
- `CUMIPMT` and `CUMPRINC` total the interest or principal of end- or beginning-of-period level
  payments from closed forms that keep their digits at small rates, and `VDB` totals
  declining-balance depreciation with the switch to straight line, with Microsoft's argument
  errors and `#NUM!` for a `VDB` end period after the life. A `CUMIPMT` or `CUMPRINC` start of 1.5
  counts as period 2, and `VDB` takes fractional periods as Excel does while they stay on the
  declining balance; other fractional periods report `calculation.unsupported_expression`.
- `AMORLINC` and `AMORDEGRC` calculate French accounting depreciation with the prorated first
  period, Microsoft's `AMORDEGRC` coefficients and whole-number rounding, and Excel's results
  through the end of the life of Microsoft's example. They return `#NUM!` for a salvage above
  cost, basis 2 or 5, a rate of 0, a negative period, a purchase after the first period, or an
  `AMORDEGRC` life below 3 or between 4 and 5 years, and 0 from period 1 on for a salvage equal to
  the cost. An `AMORDEGRC` period between 0 and 1, a purchase on the first period's end, other end
  periods, and other undocumented edges report `calculation.unsupported_expression`.
- `IMSIN`, `IMCOS`, `IMTAN`, `IMCOT`, `IMSEC`, `IMCSC`, `IMSINH`, `IMCOSH`, `IMSECH`, and
  `IMCSCH` return complex text like the other complex functions, `#VALUE!` for a logical inumber,
  and `#NUM!` at the pole zero. `IMCOSH(COMPLEX(3,4))` gives Mac Excel 2021's
  `-6.58066304055116-7.58155274274655i`; Excel Online writes the correctly rounded `...654i`,
  which f64 `sinh` and `sin` miss by one ULP. `IMTAN` and `IMCOT` keep their digits near a real pole, and a bounded result past
  an overflow is its limit once the rest underflows (`IMTAN("1+400i")` is `i`); before that it
  reports `calculation.unsupported_expression`.
- `ENCODEURL` keeps ASCII letters, digits, and `.` and encodes `:`, `/`, and the space, as
  Microsoft documents; text with other characters reports `calculation.unsupported_expression`.
  Authored `ENCODEURL` formulas are stored as `_xlfn.ENCODEURL`.
- `FILTERXML` returns the string values of the nodes an XPath 1.0 expression selects from
  well-formed XML, down one column, and `#VALUE!` for invalid XML or an undeclared namespace prefix,
  as Microsoft documents. A value that reads as a number or is empty, an empty selection, a result
  that is not a node set, a DTD, and white-space-only text that would change the result report
  `calculation.unsupported_expression`, since Excel for Windows cannot be measured. Authored
  `FILTERXML` formulas are stored as `_xlfn.FILTERXML`.
- `CELL` returns the `"address"`, `"contents"`, `"row"`, `"col"`, and `"type"` information defined
  by the workbook alone, as Excel does for a range's row, an empty cell's contents (0), a formula's
  text result (`"l"`), and an info type in any case. `"format"`, `"color"`, and `"parentheses"`
  read the upper-left cell's number format and return the codes Microsoft documents (`"F2"`,
  `"C2-"`, `"D4"`, ...; `yyyy-mm-dd` is `"G"` as in Excel); a session edit of a number format
  recalculates the formulas that call `CELL`. Width, alignment, protection, and file-name info
  types, the format of an empty position or an undocumented format, an omitted or empty reference,
  another sheet's address, and a range's `"col"` or `"type"` report
  `calculation.unsupported_expression`.
- `BAHTTEXT` reads amounts as Thai baht text as Excel does, including zero, negative amounts,
  amounts below one baht, rounding to the nearest satang, and เอ็ด for every final one after a
  higher digit; a half satang whose direction depends on the rounding rule reports
  `calculation.unsupported_expression`.
- Formulas accept trim references, which Microsoft documents as `TRIMRANGE` shorthand: `A1:.B9`
  trims trailing blank rows and columns, `A1.:B9` leading ones, and `A1.:.B9` both, on cell,
  whole-column, and whole-row ranges. `FormulaText::from_user_input` stores them as
  `_xlfn.TRIMRANGE(A1:B9,2,2)` with mode 1, 2, or 3; trim references read from storage text keep
  their dots through structural edits and copies.
- `WorkbookChange::import_sheet(ImportedSheet, position)` adds a worksheet read with
  `ImportedSheet::from_draft` from another workbook, as Excel copies a sheet into another workbook:
  the sheet keeps its name or takes `Name (2)`, its tables keep their names unless taken, its
  scoped names and the workbook names that read it come too, and custom number formats are
  renumbered where the target uses the ID for another code. Cells, formats, column widths, row
  heights, the frozen pane, merged ranges, hidden rows, tables, and the auto-filter come along, and
  print settings do not. References to another sheet or workbook, data validations, hyperlinks,
  comments, and package-only content such as conditional formats or drawings fail with the new
  `validation.sheet_import_unsupported`. Interop `read_sheet_for_import` and `import_sheet`, Python
  `import_sheet`, Node.js `importSheet`, and the MCP `workbook_import_sheet` tool expose it. A
  generated worksheet now writes its merged ranges and hidden rows.
- `ExcelError::from_spelling(&str)` returns the error whose canonical spelling, such as `#N/A` or
  `#PYTHON!`, is the given text, and `None` for any other text.

### Changed

- Formulas that read a range, such as `SUM`, `COUNTBLANK`, `AND`, the conditional aggregates
  and counts, the database functions, lookups, and array operands, and the indexes exact and
  approximate lookups and criteria share, find the range's stored cells in one ordered walk of
  the sheet instead of one lookup per cell, with the same results and the same reads charged.
- Saving a document draft's cell edits into a source worksheet copies the XML the edits leave
  alone straight from the source part instead of rebuilding it event by event, and reads the
  source styles of edited cells only until it has found them all, with the same output.
- Saving a document draft verifies the sheets it rewrote by reopening them in full, and the
  sheets the draft left alone and the save copied unchanged, while the shared strings, styles, and
  cell metadata stay as they were, by their names and visibility only, so a save of one edited
  sheet no longer reparses the whole workbook.
- Repeated approximate `VLOOKUP`, `HLOOKUP`, and `MATCH` searches of one range use the index exact
  searches share, which answers where the linear search stops and what it returns, with the same
  results and the same cell reads and work charged.
- Saving compresses the parts it writes new and every part of a workbook it writes from scratch
  with deflate, as Excel does, instead of storing them uncompressed; a source part the save keeps
  or replaces keeps its own compression.
- Reading a literal cell outside every array area no longer looks it up among formula results,
  kept results, and issue sets first, so formulas over large literal ranges calculate faster with
  the same results.
- MCP `workbook_apply_changes` stages its batch on a blocking worker, as `workbook_apply_changes_v2`
  does, so a large batch no longer holds the server's async runtime, and a cancelled request stops
  the staging cooperatively and fails with `mcp.operation.cancelled` without changing the session.
  The worker holds the session from staging through installation, so a concurrent recalculation or
  edit cannot fail the batch with `session.stale_result` or `session.revision_mismatch` after it was
  staged. Interop `WorkbookSession::prepare_changes_cancellable` stages a v1 batch with a cancellation
  token.
- The Python binding requires `pyo3` and `pyo3-build-config` 0.30.1 or a later 0.30 release.
  Supported Python versions are unchanged.
- Formulas over overlapping ranges share the dependency areas those ranges are stored as, so a
  running total such as `SUM($B$2:Bn)` over formula cells adds a few dependency edges per row
  instead of one per formula above it. A formula counts toward `max_dependency_edges` one edge per
  cell or area it waits for and one per member of each area it adds, but never more than the
  distinct formula cells and array anchors its references cover, so a formula that calculated
  before still does; a 5,000-row running total over formulas no longer reports the limit for its
  last thousand rows. Calculation order is unchanged; a formula that reads one that now
  calculates can return a different value, as described below.
- `SUMIF`, `SUMIFS`, `AVERAGEIF`, `AVERAGEIFS`, `COUNTIF`, `COUNTIFS`, `MAXIFS`, and `MINIFS`
  calculate when the cells they read fit `max_array_cells`, where a row stops at its first
  unmatched criterion, instead of reporting the limit whenever their ranges together hold more
  cells than it allows. A call whose ranges exceed the limit and that a calculation-engine issue
  stops still reports `max_array_cells`. A two-condition `SUMIFS` over 340,000 rows now
  calculates.
- After edits that only add, replace, or remove cell formulas, `Auto` recalculation parses just
  the edited formulas, updates the dependency graph in place, and evaluates them and their
  dependents, reporting `Incremental` with `DirtySubset`, or `Full` with `DirtySetCoversWorkbook`
  when they are every formula, instead of `TopologyChanged`; the delta's `parsed_formula_count`
  counts the edited formulas. A workbook that already holds references incremental calculation
  cannot follow reports `DynamicTopology`, as after a value edit. An edit that adds such a
  reference, touches a cell that is or was part of an array formula, or lets the dependency graph
  reach `max_dependency_edges` still compiles the workbook again and reports `Full` with
  `TopologyChanged` and every formula as dirty. Forced `Incremental` and `Full` recalculation and
  transactions behave as before, and results are those of a full calculation.
- A full calculation with references known only from values, such as `OFFSET` or `INDIRECT` with
  calculated arguments or a range ending in `INDEX`, evaluates the workbook once instead of twice
  when that pass read every formula after calculating it and the resolved dependencies neither
  limit nor cycle other formulas; workbooks with a dynamic array formula without a declared spill
  range still take two passes. The function iterations and reference cells a calculation reports
  are then those of one pass.
- A formula that reads a cell 0.1.26 reported unavailable can return a different value now that
  the cell calculates. Such cells are an `OFFSET` or `INDIRECT` positioned through another dynamic
  reference, a conditional aggregate whose ranges exceed `max_array_cells` while the cells it reads
  fit, and a formula that `max_dependency_edges` left out of the dependency graph. Conditional
  aggregates such as `COUNTIF` and `SUMIF` and lookups such as `XLOOKUP` and `MATCH` skipped the
  unavailable cell and now read its value: with `K1` holding `OFFSET(K5,L1,0)`, which returns 2,
  and `J1` holding `OFFSET(J5,K1,0)`, which now returns 9 instead of `UnsupportedExpression`,
  `COUNTIF(J1:K1,">0")` returns 2 instead of 1, `SUMIF(J1:K1,">0")` 11 instead of 2,
  `XLOOKUP(9,J1:K1,J1:K1,-1)` 9 instead of -1, and `MATCH(9,J1:K1,0)` 1 instead of `#N/A`. A cycle
  through a formula the dependency-edge limit left out, which 0.1.26 did not detect, now reports
  `CircularReference` for each formula in it and `BlockedByUpstream` for the formulas that read
  it: where the limit left out `E1` holding `SUM(A1:B1)+D1`, `D1` holding `COUNTIF(E1,">0")`
  returned 0, and a formula reading `D1` calculated from that 0.
- Minimal-write impact analysis treats an `OFFSET` whose position the edited workbook gives
  without calculating anything, such as `OFFSET(A1,1,0)` or `OFFSET(A1,C1,0)` with a value in
  `C1`, as reading the cells it resolves to, so an unrelated edit keeps its saved result.
  `INDIRECT` and an `OFFSET` whose position reads a formula, directly or through a function such
  as `COUNTIF`, are still affected by any edit.
- An edit-schema-v2 batch with more than one invalid operation reports the error of the first
  invalid operation in batch order. Before, its schema-v1 operations were converted ahead of its
  table operations, so an invalid schema-v1 operation was reported even when an invalid table
  operation preceded it. A batch with one invalid operation reports the same error as before.

### Fixed

- A custom number format whose code is `General`, as Numbers declares the built-in format 0,
  reads as General instead of a date format, and saving an edit of a workbook Numbers exported no
  longer fails with `xlsx.write.output_verification_failed`.
- Saving a draft whose dynamic-array formula was set without a declared range writes the spill
  range the calculation resolved as the anchor's `ref`, as Excel does. It was written as
  `<f t="array">` without `ref`, so on reopening, in CellRune or Excel, the spilled values were
  literals in the spill's way and the anchor became `#SPILL!`.
- MCP `workbook_apply_changes_v2` keeps the session locked from staging through installation. It
  released the session in between, so a recalculation, targeted calculation, or presentation edit
  that ran in that gap failed the staged batch with `session.stale_result` or
  `session.revision_mismatch`.
- Node.js `previewChangesPage` continues from the `nextCursor` it returned instead of failing with
  `interop.preview.cursor_invalid`.
- MCP rejects a workbook path that contains a `..` component with `mcp.path.invalid`. An input
  such as `/outside/dir/../../root/book.xlsx` was matched to its root after folding `..` but opened
  as spelled, so whether `/outside/dir` existed changed the result, and Save As already refused
  such destinations with a misleading detail or `mcp.path.outside_root`.
- A dynamic reference whose position comes from another dynamic reference, such as an `OFFSET`
  row read through a second `OFFSET`, calculates instead of reporting
  `UnsupportedExpression`: a full calculation repeats while a pass reads a formula before
  calculating it, ordering each pass by the references the previous pass resolved, for at most
  eight passes.
- Packages that failed to read now open: a `cellXfs` `xf` without the optional `numFmtId` reads as
  General; a cell without the optional `r` takes the column after the previous cell of its row and
  a row without `r` the row after the previous one, whose height, style, hidden state, and `ph`
  phonetic visibility apply to that row; a cell holding `#FIELD!`, `#BLOCKED!`,
  `#CONNECT!`, `#BUSY!`, `#UNKNOWN!`, or `#PYTHON!` reads as the new `ExcelError` variant, which a
  formula reading it reports as an unsupported capability instead of a guessed value; and a
  shared formula whose relative reference shifts past the last row or column wraps around the
  grid, as Excel's copy does. A worksheet that a save rewrites gets an explicit `r` on such rows
  and cells; other sheets keep their bytes.
- A document open no longer fails on a sheet view or pane it cannot model, such as one without
  `workbookViewId`, with a fractional or negative split, or whose `activePane` or `topLeftCell`
  disagrees with its split; the pane stays preserved but unmodeled with an `xlsx.pane.invalid`
  presentation diagnostic, as a snapshot read already ignored it.
- Incremental recalculation reports a recalculated formula's capability issue as a full
  calculation does: `UnsupportedName` for an undefined name, `UnsupportedFunction`, `ParseError`,
  or `UnsupportedExpression` with its detail, ahead of `CircularReference`, instead of the value
  or `UnsupportedExpression` the formula evaluated to. A recalculated formula that a calculation
  engine issue stops while it reads such a formula reports `BlockedByUpstream`, as in a full
  calculation.
- A formula that incremental recalculation evaluates reads the value a formula with a capability
  issue evaluates to, as in a full calculation, instead of reporting `BlockedByUpstream`: after an
  edit of `A1`, `B1+A1` with `B1` holding `NB*3` and no name `NB` is `#NAME?` again. The pass
  evaluates such a formula too when it reads its result.
- A sheet rename that gives an end of a 3-D reference a name that needs quotes writes the range as
  one quoted token, `'New Name:End'!A1`, as MS-XLSX requires, instead of `'New Name':End!A1`.
- A rename or deletion that writes a sheet name reading as a cell reference, such as `S2` or
  `R1C1`, quotes it (`'S2'!A1`, `'S2:S4'!A1`), as Excel writes it. Excel reads an unquoted
  `S2:S3!A1` as a range starting at cell `S2`.

## [0.1.26] - 2026-10-23

### Added

- `RAND`, `RANDBETWEEN`, and `RANDARRAY` calculate from an explicit seed set with
  `CalculationOptions::with_random_seed(u32)`; without one they report
  `calculation.volatile_input_missing` as `TODAY` and `NOW` do, and CellRune never reads host
  randomness. A draw depends only on the seed, the formula cell's sheet ID and address, the call's
  position in its formula (through defined names and `LAMBDA` calls), the `MAKEARRAY`, `MAP`,
  `BYROW`, `BYCOL`, `SCAN`, or `REDUCE` iteration or the `GROUPBY` or `PIVOTBY` group, the computed
  database-criteria record, and the array element, so wherever full calculation produces a value,
  targeted, incremental, and preview calculation produce the same one. The generator is not
  cryptographically secure. Argument edges follow Excel: `RANDBETWEEN` returns `#NUM!` when the
  bottom exceeds the top, and `RANDARRAY` returns `#VALUE!` for a negative dimension, a minimum
  above the maximum, or a fractional bound with `whole_number`, and `#CALC!` for a zero dimension;
  both return `#NUM!` for a span too wide for a double. Authored `RANDARRAY` formulas are stored as
  `_xlfn.RANDARRAY`, and the catalog grows to 457 official names and 458 accepted entries.
- The random seed is `CalculationOptionsDto::random_seed` and the transaction report's
  `calculation_options.random_seed` in interop, Python `random_seed` and Node.js `randomSeed` on
  calculate, recalculate, targeted calculation, and previews, and MCP `random_seed` on
  `workbook_recalculate`, `workbook_calculate_targets` options, and `workbook_preview_changes`. It
  must be an integer from 0 through 4294967295.

### Changed

- CellRune requires `quick-xml` 0.42. For valid UTF-8 input, read errors, written bytes, and
  calculation results are unchanged, and each element may still declare up to 256 namespaces however
  deeply nested. With invalid UTF-8, an element past the XML depth limit whose start tag holds
  invalid UTF-8 reports the part's error code instead of `xlsx.xml_depth_exceeded`, and an unknown
  namespace prefix spelled with invalid UTF-8 is no longer reported as the error detail.
- When only the `TODAY()` serial, the `NOW()` serial, or the random seed differs from the installed
  calculation, `Auto` recalculation evaluates only the formulas that read the changed input and
  their dependents, reporting `DirtySubset` or `NoDirtyFormulas`, and `Incremental` recalculation
  proceeds instead of failing with `session.incremental_unsafe`. Workbooks with uncertain dynamic
  references still recalculate in full when such a formula is dirty, and any other option change
  still recalculates in full. Results are unchanged.
- The Node binding requires `napi` 3.15.0 and `napi-build` 2.6.0 or later. `napi-derive-backend`
  6.1.6 generates calls to `napi::bindgen_prelude::defer_native_borrow_scope`, which `napi` 3.15.0
  added, and `napi` 3.15.0 builds with `napi-build` 2.6.0, so a freshly resolved graph with the
  earlier floors no longer compiled.

### Fixed

- Invalid UTF-8 anywhere in a parsed XML part, including a comment, a processing instruction, an
  unread element or attribute, or bytes before the XML declaration, fails the read with the part's
  error code, such as `xlsx.invalid_worksheet`, instead of opening.
- Renaming a sheet fails with `xlsx.write.invalid_generated_xml` when a chart or pivot cache part is
  UTF-16 or contains invalid UTF-8, instead of writing that part unchanged with its reference to the
  old sheet name or its invalid bytes.
- A save after a draft changes the date system no longer fails with
  `xlsx.write.output_verification_failed` when the draft also changed the number format of a `t="d"`
  date cell, changed its value and back, or cleared and re-entered it: such cells are written as
  their serial numbers too.
- Python `calculate_targets` rejects `calculation_limits` with
  `interop.calculation.target.invalid_payload`, like any other unknown option, instead of applying
  it. Targeted calculation keeps the default calculation limits, as in Node.js and MCP.
- Python `read_limits` and `calculation_limits` fail with `interop.limit.payload_invalid` when given
  a list, tuple, or other object that is not a `dict`, which set limits by position, or a limit
  whose value is `None`, which kept its default.
- Node.js `previewChanges` reports an invalid `calculationLimits` with
  `interop.limit.payload_invalid` instead of `interop.change.payload_invalid`. `readLimits` and
  `calculationLimits` that are not plain objects fail with `interop.limit.payload_invalid` instead
  of `interop.input.invalid` or being read as no limits, and a snake_case limit name such as
  `max_sheets`, which was applied, fails as an unknown name.
- A table-qualified structured reference to one column whose name holds an underscore, `\ ! ( ) % ?
  ; ~`, a backtick, or `. - @ & =`, such as `Orders[unit_price]`, no longer fails with
  `calculation.parse_error`, so such formulas calculate and renaming any column of their table no
  longer fails the batch. A column rename writes such a reference as Excel saves it,
  `Orders[unit_price]` and `Orders[a@b]`, instead of `Orders[[unit_price]]` and `Orders[[a'@b]]`; a
  leading `@` keeps its escape. As in Excel, a selector list still needs the name bracketed, as in
  `Orders[[#This Row],[unit_price]]`.
- Formulas authored with `IMAGE`, `STOCKHISTORY`, `TRANSLATE`, `DETECTLANGUAGE`, or `FIELDVALUE`
  gain the `_xlfn.` prefix Excel stores, so Excel no longer reports `#NAME?` when it recalculates
  them. Such a formula reads back as `_xlfn.IMAGE(...)`, and its unsupported-function detail is
  `_XLFN.IMAGE`, as for `AGGREGATE`.
- A `t="d"` cell holding `9999-12-31T24:00` or `9999-12-31T24:00:00` reads as 0 in either date
  system, as in Excel, instead of 2958466 (1900) or 2957004 (1904). A `t="d"` value wrapped in a
  non-ASCII space such as U+00A0 or U+3000, which Excel offers to repair and 0.1.25 accepted, fails
  the read with `xlsx.invalid_cell_value`; surrounding ASCII spaces are still ignored.
- Saving an edited and calculated document draft no longer fails with
  `xlsx.write.output_verification_failed` when the result of a legacy (CSE) array or dynamic-array
  formula changed or was invalidated: output verification checks the array and spill follower cells
  against their calculated results instead of the results the source saved.
- Renaming a column of a table or resizing it no longer fails to save with
  `xlsx.write.output_verification_failed` when a calculated column or totals row formula contains an
  entity or character reference, such as the `&amp;`, `&lt;`, and `&gt;` Excel stores for `&`, `<`,
  and `>`.
- Python `calculate_targets` fails with `interop.calculation.target.invalid_payload` when `limits`
  is a list, tuple, or other object that is not a `dict`, and MCP `workbook_calculate_targets`
  rejects a `limits` array as invalid arguments, as both already did for a string or number. Both
  set target limits by position, so `[1]` set `max_targets` and `[]` passed.
- Node.js `calculateTargets`, `previewChanges`, `applyChanges`, and `applyChangesV2`, Python
  `calculate_targets`, `preview_changes`, `apply_changes`, and `apply_changes_v2`, and MCP tools
  read `TODAY()` and `NOW()` serials and numeric cell values exactly, instead of up to one ULP away
  when the number has 16 or 17 significant digits, such as a `NOW()` serial of 45158.311281307004
  calculated as 45158.31128130701.
- Python `calculate_targets`, `apply_changes`, `apply_changes_v2`, `preview_changes`, and
  `preview_changes_page` and MCP `workbook_calculate_targets`, `workbook_apply_changes`,
  `workbook_apply_changes_v2`, `workbook_preview_changes`, and `workbook_preview_changes_page`
  reject a list or tuple (MCP: an array) given for a target, a change or its `value`, a preview
  cursor, or MCP target `options`, with the error each already uses for a malformed payload: Python
  `interop.calculation.target.invalid_payload`, `interop.change.payload_invalid`, or
  `interop.preview.cursor_invalid`, and MCP invalid arguments. These were read by position, so
  `["Sheet1", "A1"]` was a target, MCP options `[null, null, 7]` set the random seed, and a `value`
  of `["number", 1.0]` was applied.

## [0.1.25] - 2026-10-18

### Added

- Python `Workbook.save_edits` and `Workbook.edits_to_bytes`, Node.js `saveEdits` and
  `editsToBytes`, and the MCP tool `workbook_save_edits_as` write the edits of a workbook opened
  from a file without calculating, as `write_xlsx_draft_edits_bytes` does, and return an edit report
  with the affected formulas, whether the host is asked to recalculate, the changed and removed
  parts, and input and output SHA-256. Interop adds `WorkbookSession::save_edits_bytes`,
  `prepare_edits_save`, and `save_edits_path` with `EditWriteOptionsDto` and `EditWriteReportDto`.
  A workbook not opened from a package fails with `xlsx.write.invalid_package_plan`.
- Python `read_limits` when opening and `calculation_limits` when calculating, recalculating, and
  previewing, and Node.js `readLimits` and `calculationLimits`, change individual read and
  calculation limits; omitted limits keep their defaults. Interop adds `ReadLimitOptionsDto`,
  `CalculationLimitOptionsDto` as `CalculationOptionsDto::calculation_limits`,
  `WorkbookSession::open_bytes_with_limits`, and `open_path_with_limits`. A zero limit fails with
  `interop.limit.invalid` and an unreadable limit object with `interop.limit.payload_invalid`. MCP
  clients keep the default limits.
- Python `Workbook.scan_capabilities` and Node.js `scanCapabilities` page through the static
  capability scan that MCP's `workbook_scan_capabilities` returns.
- Range cells carry a `number_format` with its ID, code, and kind; sheet summaries an `auto_filter`
  range; and workbook summaries `full_calculation_on_load`, across interop, Python, Node.js, and
  MCP.
- `XlsxWriteError::unavailable_formulas` names the first 64 formulas whose unavailable results
  failed a strict save with `xlsx.write.incomplete_calculation`, reporting array and spill cells by
  their owning formula. Interop, Python, Node.js, and MCP carry them as `details.cells`; errors
  without cells keep their previous JSON shape.
- Python wheels for Linux arm64 (`manylinux_2_28`), Linux x86_64 and arm64 on musl
  (`musllinux_1_2`), macOS x86_64, and Windows arm64, matching the npm platform packages. Windows
  arm64 wheels start at Python 3.11, the first CPython with a Windows arm64 build.

### Changed

- Exact `VLOOKUP`, `HLOOKUP`, `MATCH`, and `XLOOKUP` searches of a range that earlier searches in
  the same calculation have already scanned several times over use a shared index instead of
  scanning, so a column of lookups into a large table no longer rescans it for every lookup. A
  range searched only a few times keeps scanning. Results, and the work charged against
  calculation limits, are unchanged.
- Collecting dependencies reads only the formulas in a range reference's columns, so formulas
  beside a large table they reference, such as a column of lookups, no longer take quadratic time
  to prepare. Results are unchanged.
- `COUNT`, `COUNTA`, `MIN`, `MAX`, `PRODUCT`, and other aggregates that do not sum no longer build
  decimal traces for referenced numbers. Results are unchanged.
- Python `apply_changes`, `apply_changes_v2`, the cell setters, `clear_cell`, `add_sheet`,
  `rename_sheet`, `changes_since`, and `close` release the interpreter lock and wait for the
  workbook session, so concurrent calls on one workbook still run in turn. Reads, `calculate`,
  `recalculate`, and saves on another thread also wait instead of failing with
  `interop.session.unavailable`; `cancel_calculation` and `calculation_active` still never wait.
- CellRune requires `zip` 9, the first release free of RUSTSEC-2026-0367, RUSTSEC-2026-0368, and
  RUSTSEC-2026-0369. Those advisories concern `zip`'s extraction APIs, which CellRune does not call.

### Fixed

- A structured reference whose selector list names a column without brackets, such as the
  `[[#This Row],Qty]` that xlsxwriter writes for `[@Qty]` in calculated columns, no longer fails
  with `calculation.parse_error`, so such workbooks calculate and an unrelated edits-only save no
  longer asks the host to recalculate. Excel reads this spelling; a bracketless item or a name
  with escapes in a list still fails.
- A cell typed `t="d"`, which openpyxl writes with `iso_dates` and SheetJS with `cellDates`, no
  longer fails the read with `xlsx.unsupported_cell_type`: its ISO 8601 date, date-time, or time
  reads as a serial in the workbook's date system, a date before the date system's first day reads
  as `#VALUE!` as in Excel, and an invalid one fails with `xlsx.invalid_cell_value`. A save after
  the date system changes writes such cells as their serial numbers.

## [0.1.24] - 2026-10-10

### Added

- `write_xlsx_draft_edits_bytes`, `write_xlsx_draft_edits`, and `write_xlsx_draft_edits_path`
  write the edits of a draft opened from a document without calculating. Formulas the edits can
  affect lose their saved results and only then does `calcPr` gain `fullCalcOnLoad`; every other
  saved result, `calcChain.xml` while every formula keeps its cell and array kind, `calcPr`, and
  untouched parts keep their source bytes. `EditWriteReport::affected_formulas` lists the cleared
  formulas. Formulas whose references are known only after evaluation, such as `INDIRECT`,
  `OFFSET`, a formula that does not parse, a call with arguments its function rejects, or a
  formula without text, count as affected whenever anything changed.
- `Sheet::auto_filter` returns a worksheet-level autoFilter as a `SheetAutoFilter` with its range
  and filter column IDs.
- `WorkbookChange::widen_sheet_auto_filter` and `WorkbookChange::widen_table_columns` widen a
  sheet autoFilter or a worksheet table by columns. Filter and sort state are kept, filter column
  IDs shift when the range grows leftward, new table columns take their header text as names, and
  `_xlnm._FilterDatabase` follows the widened filter. Rejected widenings report
  `validation.column_widening_range_invalid`, `validation.column_widening_overlap`,
  `validation.sheet_auto_filter_overlaps_table`, `validation.sheet_auto_filter_not_found`, or
  `validation.table_totals_cell_occupied`.
- `open_xlsx_document_vec` keeps a caller's `Vec<u8>` as the retained archive without copying it,
  and `XlsxDocument::into_input_bytes` returns the retained bytes, without copying them when the
  document is their only owner. The other `open_xlsx_document_*` entry points now hold one copy of
  the archive while opening instead of two.
- `WorkbookSnapshot::non_worksheet_tab_ids` returns the IDs of chartsheets, dialogsheets, and
  macrosheets.

### Changed

- `WorkbookFingerprint` now covers a worksheet-level autoFilter, so a workbook that declares one
  gets a different digest than in 0.1.23 under the same schema version.

### Fixed

- `FormulaText::from_user_input` stores formulas in Excel's saved spelling, so a host that
  recalculates an authored file no longer reports `#NAME?`: future functions gain `_xlfn.`
  (`_xlfn._xlws.` for `FILTER` and `SORT`), `LET` and `LAMBDA` parameters `_xlpm.`, functions passed
  as `LAMBDA` arguments `_xleta.`, `B1#` becomes `_xlfn.ANCHORARRAY(B1)`, and `@x` becomes
  `_xlfn.SINGLE(x)`. Formula text read back through Rust, Python, Node.js, and MCP shows this
  spelling, as for formulas read from a file.
- A cell a document draft creates takes the style of its row when the row has `customFormat`, else
  its column's style, as Excel does; an empty styled cell keeps its own style unless that style
  names no `cellXfs` entry, in which case the cell is styled as a new one.
- Writing, clearing, or table-resizing a shared formula's anchor keeps the other cells of the
  group as ordinary formulas instead of failing the save.
- Value, formula, clear, and annotated-text edits to a legacy array formula's cells other than its
  anchor fail at once with `validation.array_formula_part_edit`.
- A cell added and then removed in the same draft, including by a table resize, no longer makes
  every later save fail.
- Writing text to a table header cell, including through `set_annotated_text`, renames that column
  and its structured references; blank or cleared headers fail with
  `validation.table_column_name_empty`, and number or formula headers with
  `validation.table_header_requires_text`.
- Changing a defined name or renaming a sheet keeps every other `<definedName>` element and the
  attributes of the changed one, such as `comment`, `function`, and `vbProcedure`.
- Renaming a sheet also rewrites its name in conditional formatting and data validation formulas,
  including `x14` ones, sparklines, hyperlink locations, chart series, and pivot cache sources. A
  chart reference that cannot be rewritten fails the save with `xlsx.write.unsupported_preservation`.
- A `calcPr` created in a workbook without one is placed in schema order.
- An edited document is verified under the read limits it was opened with, so a document opened
  with raised limits saves after an edit, and outputs between 256 and 512 MiB no longer fail
  verification.
- A worksheet saved with an empty `<sheetData/>` accepts new cells.
- Whole-column and whole-row references include spilled and declared array results below the last
  stored cell.
- A range ending in `INDEX`, such as `B2:INDEX(D2:D20,F1)`, no longer leaves a stale result after
  its end moves: edit impact covers every cell the end can reach, and a session recalculates in
  full when a formula or array result lies in that reach. Calculation order follows only the cells
  the range covers, so a formula inside the reach is not a circular reference.
- Incremental recalculation reads a failed declared spill's area as `#SPILL!`, as full and targeted
  calculation do.
- Exceeding `max_dependency_edges` leaves the formulas collected before the limit calculated; only
  the rest and their dependents report the limit.
- Criteria functions, exact `MATCH`, `XMATCH`, `XLOOKUP`, database criteria, and `UNIQUE` charge
  the function work budget once per compared text instead of once per byte, so tens of thousands
  of text rows no longer exceed the default limit.
- `ROUND`, `ROUNDUP`, `ROUNDDOWN`, and `TRUNC` with a digit count at or below -2^31 no longer panic
  in builds with overflow checks.
- A ZIP whose end records declare more entries than `max_entries` fails with
  `xlsx.too_many_entries` before its central directory is read. The counts of every end record the
  ZIP reader may fall back to are summed, so a decoy record that fails to parse cannot hide the one
  read after it. A ZIP64 entry with a huge compressed size fails with `xlsx.invalid_zip` instead of
  overflowing.
- Interop numbers a sheet added in a batch past chartsheet, dialogsheet, and macrosheet IDs, so
  later changes in the batch find the new sheet.
- A Rust panic in a Node.js call throws an error instead of aborting the process, and a session left
  unusable by a panic reports `interop.session.poisoned` in Python and Node.js instead of
  `interop.session.unavailable`.
- The Node binding requires `napi-derive` 3.6.11 or later. `napi-derive-backend` 6.1.5 moved to
  `syn` 3, so a freshly resolved graph with an earlier `napi-derive` no longer compiled.

## [0.1.23] - 2026-10-01

### Added

- `XlsxDocument::input_bytes` borrows the exact archive bytes a document retains for round-trip
  writing, without copying them; `input_hash` is their SHA-256.
- `XlsxReadError::observed` returns the value that exceeded a read limit, in that limit's
  unit, and `XlsxReadError::phonetic_run_limit` returns whether `xlsx.too_many_phonetic_runs`
  exceeded the per-item or total run limit as a `PhoneticRunLimitKind`.
- `CalculationIssue::limit` returns the `CalculationLimitKind` a
  `calculation.resource_limit_exceeded` issue exceeded.
- `TargetCalculationError::requested_cell_count` returns a lower bound on the cells a targeted
  calculation requested when it passed its result-cell limit, and `evaluated_count` returns the
  evaluations performed before its work limit stopped it.

Existing error codes, details, and messages are unchanged.

### Fixed

- Reading a package part reserves its declared size once and stops inflating one byte past it, so
  an entry that under-declares its size fails with `xlsx.declared_size_mismatch` without first
  allocating up to the entry limit. Such an entry now reports that code even when it would inflate
  past the entry limit, where it previously reported `xlsx.entry_too_large`. An entry that claims
  more compressed bytes than the whole archive fails with `xlsx.invalid_zip` before any part is
  read.

## [0.1.22] - 2026-09-30

### Added

- `PERCENTILE.EXC`, `QUARTILE.EXC`, `PERCENTRANK.EXC`, `TRIMMEAN`, `STDEVA`, `STDEVPA`, `VARA`,
  `VARPA`, `STEYX`, `FISHER`, `FISHERINV`, `PROB`, `CHISQ.DIST`, `CHISQ.DIST.RT`, `WEIBULL.DIST`,
  `CONFIDENCE.NORM`, `CONFIDENCE.T`, `IMLOG10`, and `IMLOG2`, with legacy names `CHIDIST`,
  `WEIBULL`, and `CONFIDENCE`, across Rust, Python, Node.js, and MCP calculation.
- `ReadLimits::with_max_total_cell_text_bytes` (default 256 MiB) bounds the text that cells
  materialize from shared strings, inline strings, and cached results. Exceeding it fails with
  `xlsx.total_cell_text_too_large`.
- Workbooks containing chartsheets, dialogsheets, or macrosheets now read, calculate, and save.
  Those tabs are skipped for calculation with an `xlsx.sheet.non_worksheet` warning and keep
  their position; `SHEET` and `SHEETS` count every tab.

### Changed

- MCP `workbook_recalculate` installs a completed calculation even when its delta exceeds
  `--max-response-bytes`. It then reports `mcp.recalculation.response_byte_limit_exceeded`
  with the installed `result_revision` and `delta_cursor`, and Save As and range reads use
  the installed results. Other tools keep the no-state-change meaning of
  `mcp.response.byte_limit_exceeded`.
- MCP input paths outside every allowed root, including UNC shares and device paths, return
  `mcp.path.outside_root` before any filesystem access, whether or not they exist.
- `SUBTOTAL` supports function numbers 6, 7, 8, 10, 11, and 106 to 111, ignores referenced
  cells that hold another `SUBTOTAL` or `AGGREGATE`, and returns `#VALUE!` for other numbers.
- Faster `LARGE`, `SMALL`, `MEDIAN`, `PERCENTILE`, `QUARTILE`, and `PERCENTRANK`, and less work
  for statistics over large ranges; faster worksheet reading.
- The Windows ARM64 Node addon is built on a native Windows ARM64 runner.
- Update `napi` to 3.12.2, `rmcp` to 3.4.1, `cap-std` to 4.0.3, and the build tooling to
  maturin 1.15.0 and `@napi-rs/cli` 3.10.5.

### Fixed

- Stack overflow on small host stacks for formulas, defined-name chains, and recursive `LAMBDA`
  calls within the default calculation limits.
- `IRR`, `XIRR`, and `RATE` convergence after a Newton step past -1; `LET` locals named `r`, `c`,
  or with an R1C1 shape such as `R1C1` or `RC`, and `LAMBDA` parameters named `r` or `c`;
  `XLOOKUP` implicit intersection in legacy formulas; `MATCH` match types selected by sign.
- Error cells no longer abort exact or linear approximate `MATCH`, `VLOOKUP`, `HLOOKUP`,
  `XLOOKUP`, and `XMATCH` searches; `VLOOKUP`/`HLOOKUP` exact matches and `SEARCH` honor `?`,
  `*`, and `~` wildcards.
- `PERCENTRANK`, `PERCENTRANK.INC`, and `PERCENTRANK.EXC` return 1 for a single matching value.
- `POISSON.DIST` for large and zero means, `MIRR` with non-numeric cells, overflow in
  `MEDIAN`/`PERCENTILE`/`QUARTILE` interpolation, Unicode case in `UNIQUE`, and a `DATEDIF`
  `"MD"` panic in overflow-checked builds.
- `_xHHHH_` escapes in cell text are decoded on read and written for control characters, so
  results such as `CHAR(2)` save and reopen.
- Shared formulas that reference names containing non-ASCII characters expand correctly.
- Documents whose frozen panes write an explicit zero split, as LibreOffice does for a row-only or
  column-only freeze, open instead of failing with `xlsx.invalid_frozen_pane`.
- Growing a table that has a totals row no longer leaves the old totals row inside the data area,
  where its `SUBTOTAL` formulas referenced themselves; that row now receives the calculated columns.
  New totals formulas name their table, as in `SUBTOTAL(109,Table[Column])`.

## [0.1.21] - 2026-09-27

### Changed

- Aggregate static function demand without expanding every repeated defined-name call.
  Counts saturate at `u64::MAX`, meaning at least that many calls; smaller counts are exact.
- Function usage report schema 2 encodes JSON/MCP `call_count` as a decimal string.
  Node `callCount` changes from `number` to `bigint`; use bigint arithmetic and convert
  the field to a string before JSON serialization. Rust `u64` and Python `int` remain.

### Fixed

- Warnings-denied MCP builds with current `rmcp` and the Node derive dependency floor.
- Update the Node build tooling's `js-yaml` dependency to 4.3.2 for its merge-key CPU fix.
- Stack overflow during function usage analysis of deeply chained names and callable aliases.
- Numeric text and large-number parity in `ISEVEN`/`ISODD`, `ERROR.TYPE(#SPILL!)`,
  `SLN` zero-life errors, and integer truncation in `QUARTILE.INC`/`QUARTILE`.
- `LOOKUP` candidate type selection and invalid text holidays in `NETWORKDAYS`/`WORKDAY`.
  The existing numeric-only holiday handling in the `INTL` variants is preserved.

## [0.1.20] - 2026-09-08

### Added

- `CODE`, `NUMBERVALUE`, `DDB`, `XNPV`, `MDETERM`, `RANK.AVG`, and
  `FORECAST.LINEAR` / `FORECAST` across Rust, Python, Node.js, and MCP calculation.
- `NORM.INV`, `NORM.S.INV`, `LOGNORM.DIST`, and `LOGNORM.INV`, with legacy names
  `NORMINV`, `NORMSINV`, `LOGNORMDIST`, and `LOGINV`. Normal quantiles resolve
  representable extreme-tail probabilities with bounded, cancellable refinement.

### Changed

- Reduced calculation preparation allocations by borrowing normalized lookup keys, moving
  parser source components into the resulting syntax tree, and reusing worksheet formula indexes
  during dependency collection.
- `RANK` and `RANK.EQ` count numeric positions directly without sorting; they charge the actual
  linear counting work to the function-iteration limit.

## [0.1.19] - 2026-09-07

### Changed

- Reduced edit preparation work for large workbooks by looking up changed formula cells directly
  in the compiled calculation state.
- Reduced repeated scanning when writing new XLSX rows and applying batches of edits to existing
  worksheets, while preserving output validation and unchanged worksheet content.
- Reduced copying and allocation when applying large incremental calculation results, while
  retaining shared immutable snapshots and the existing automatic calculation selection rules.
- `UNIQUE` now groups duplicates in one pass and charges actual tuple, text, and comparison work.
  Larger inputs can complete within existing limits; text-heavy inputs observe the work budget.
- Reduced targeted calculation setup by reusing worksheet formula addresses, column extents, and
  declared array metadata while preserving source identity and current full-result reuse.
- Cache capability and function-usage reports on demand for repeated queries on the same immutable
  workbook and calculation options, including paged queries through language bindings and MCP.
- Reduced multi-sheet XLSX save work by sharing ZIP access and part lookup within each save,
  and reading only each worksheet's edits and calculation results.
- Reduced temporary allocation in `SUM`, `AVERAGE`, `MIN`, `MAX`, and `PRODUCT` by aggregating
  collected arguments directly, with the same coercion, evaluation order, and arithmetic policy.
- Opening an in-memory XLSX document avoids an extra archive copy and checks the archive size
  limit before buffering the input.

## [0.1.18] - 2026-09-06

### Added

- Targeted partial calculation across Rust, Python (`calculate_targets`), Node.js
  (`calculateTargets`), and MCP (`workbook_calculate_targets`). The first request parses and
  evaluates requested cells and their required precedents, with deduplicated rectangular targets,
  typed values/issues, deterministic ordering, and bounded work. Dynamic references and known
  array followers use the existing formula evaluator and array collision rules.
- Separate immutable partial results carry the source revision, fingerprint, provenance, options,
  limits, and parsed/evaluated/reused counts. Compatible current complete calculations can supply
  cached results. Partial requests preserve full-cache, dirty, delta, preview, and save contracts;
  cancelled, stale, or oversized responses are withheld. MCP now exposes 17 workbook tools.

## [0.1.17] - 2026-08-26

### Changed

- Simplified release evidence to rely on functional and structural assertions: removed function
  catalog SHA golden files, the duplicated Excel Oracle artifact/hash chain, and source-commit
  fields from the manual performance harness JSON.
- Advanced the committed Excel Oracle metadata, suite, case-manifest, and observations schemas to
  v3 while retaining their fixture references, case inventory, host identity, saved values, and
  semantic comparison checks.

### Fixed

- Package-backed drafts can now add sheets to valid workbooks whose relationship namespace is
  declared on each existing `sheet`. Added sheets bind their relationship attributes explicitly,
  including alternate prefixes and Strict OOXML relationship namespaces, and the result passes
  full-package verification and reopen.

## [0.1.16] - 2026-08-17

### Added

- Retained, immutable verified change previews across the Rust interop session, Python, Node.js,
  and local MCP. A session retains one active calculation and one published preview; two-phase
  publication preserves an earlier preview when a replacement fails, is cancelled, or cannot fit
  its response. Commit installs only a current captured base, while a pre-commit cancellation or
  resource failure remains retryable.
- Four local stdio MCP tools: `workbook_preview_changes`,
  `workbook_preview_changes_page`, `workbook_commit_preview`, and
  `workbook_discard_preview`. The server now exposes 16 high-level workbook tools; preview paging
  is byte-bounded and delegates its opaque cursor to the core transaction pager.
- `OutputHash`, a distinct SHA-256 identity for exact verified output archive bytes. Rust write
  reports expose `output_hash()`, Python reports expose `output_sha256`, Node reports expose
  `outputSha256`, and MCP Save As returns the shared report.
- `DATEVALUE`, `TIMEVALUE`, `NETWORKDAYS.INTL`, and `WORKDAY.INTL`, plus the implemented
  `XLOOKUP` calculation contract including its array result shape, exact/approximate/wildcard
  matching, and forward/reverse/binary search modes.

### Changed

- The deterministic source catalog contains 416 official Excel-facing names and 417 accepted
  entries, including the non-official OOXML dummy-function marker. `XLOOKUP` remains a catalogued
  name and the four date functions are new official names.
- Public documentation now states the headless backend and local-stdio MCP boundary, the retained
  preview lifecycle, binding call shapes, verified output identity, and the 0.1.16 formula
  contracts.

## [0.1.15] - 2026-08-13

### Added

- Twenty-six fixed-income security functions: `ACCRINT`, `ACCRINTM`, `COUPDAYBS`, `COUPDAYS`,
  `COUPDAYSNC`, `COUPNCD`, `COUPNUM`, `COUPPCD`, `DISC`, `DURATION`, `INTRATE`, `MDURATION`,
  `ODDFPRICE`, `ODDFYIELD`, `ODDLPRICE`, `ODDLYIELD`, `PRICE`, `PRICEDISC`, `PRICEMAT`,
  `RECEIVED`, `TBILLEQ`, `TBILLPRICE`, `TBILLYIELD`, `YIELD`, `YIELDDISC`, and `YIELDMAT`,
  built on a typed day-count and coupon-schedule model plus a safeguarded yield root solver that
  charges the calculation budget and observes cancellation.

### Changed

- The public function catalog now contains 412 official Excel-facing names and 413 accepted
  entries including the non-official OOXML dummy-function marker. The 0.1.15 catalog and semantic
  registry have byte-exact snapshots, while the versioned 0.1.14 snapshots remain unchanged.
- Excel Oracle expectations classify all 78 active nominal, coercion, and domain cases for the
  twenty-six fixed-income functions as matches in both frozen host profiles; two existing
  composite cases unlocked by the family also match.
- Replaced the pinned third-party `setup-zig` action with a repository-owned, checksum-verified
  Zig 0.14.1 installer so the Linux binding artifact matrix no longer depends on a Node 20
  runtime.

## [0.1.14] - 2026-08-10

### Added

- `CONVERT` with a typed, case-sensitive registry covering 102 unit entries and 152 accepted unit
  spellings across mass, distance, time, pressure, force, energy, power, magnetism, temperature,
  volume, area, information, and speed, including explicit decimal and binary prefix policies.
- Four budgeted first-party Bessel functions: `BESSELI`, `BESSELJ`, `BESSELK`, and `BESSELY`,
  with family-specific domain, parity, recurrence, series, and asymptotic handling.
- Fourteen complex-text functions: `COMPLEX`, `IMABS`, `IMAGINARY`, `IMARGUMENT`, `IMCONJUGATE`,
  `IMREAL`, `IMDIV`, `IMPOWER`, `IMPRODUCT`, `IMSUB`, `IMSUM`, `IMEXP`, `IMLN`, and `IMSQRT`,
  sharing one finite parser, canonical formatter, suffix policy, and scaled arithmetic layer.

### Changed

- The public function catalog now contains 386 official Excel-facing names and 387 accepted
  entries including the non-official OOXML dummy-function marker. The versioned 0.1.13 catalog
  and semantic snapshots remain byte-exact.
- Topology-stable recalculation now shares immutable workbook, compiled topology, and installed
  result roots; reuses persistent impact indexes and topological ranks; and patches only dirty
  result, spill, trace, and delta chunks. No-dirty calculation bypasses evaluator setup, graph
  scheduling, formula snapshot scans, and result-map comparison.
- Excel Oracle expectations classify all 33 active nominal and boundary cases for the nineteen new
  functions as matches in both frozen host profiles. Eight Bessel numeric cases use their reviewed
  worksheet-compatibility tolerance while independent high-precision kernel tests remain stricter.

### Fixed

- Avoided intermediate overflow in extreme finite unit identity conversions and complex power,
  square-root, magnitude, and exponential calculations.
- Preserved out-of-lock cancellation, resource accounting, stale-result rejection, source
  identity, decimal traces, and dynamic-dependency full-recalculation fallbacks on the optimized
  session path.

## [0.1.13] - 2026-08-09

### Added

- Eight F-distribution and variance-test names: `F.DIST`, `F.DIST.RT`, `F.INV`, `F.INV.RT`,
  `F.TEST`, `FDIST`, `FINV`, and `FTEST`, with direct upper-tail evaluation and scaled variance
  ratios that preserve representable subnormal probabilities.
- Nine t-distribution and t-test names: `T.DIST`, `T.DIST.2T`, `T.DIST.RT`, `T.INV`, `T.INV.2T`,
  `T.TEST`, `TDIST`, `TINV`, and `TTEST`; plus `Z.TEST`, `ZTEST`, and `COVARIANCE.S`. Stable online
  moments and scaled test standard errors protect extreme finite inputs from avoidable
  intermediate overflow.
- A direct upper regularized incomplete-beta kernel, a uniform central expansion for large shapes,
  and compensated continued-fraction arithmetic outside the central region. Every iterative step
  observes cancellation and the calculation resource budget.

### Changed

- The public function catalog now contains 367 official Excel-facing names and 368 accepted
  entries including the non-official OOXML dummy-function marker.
- Excel Oracle expectations classify all 60 active cases for the twenty new names as matches in
  both frozen host profiles.

### Fixed

- Preserved accurate direct F upper tails and subnormal `F.TEST` probabilities instead of losing
  them through complementary subtraction or an overflowing materialized variance ratio.
- Scaled pooled and Welch `T.TEST` intermediates, validated F inverse degrees of freedom before
  endpoint shortcuts, mapped overflowed or underflowed internal Z scores to exact tails, charged
  the large-shape beta expansion to the configured function-iteration limit, and observed
  cancellation while filtering long sample arrays.

## [0.1.12] - 2026-08-03

### Added

- Seven gamma functions: `GAMMA`, `GAMMA.DIST`, `GAMMADIST`, `GAMMA.INV`, `GAMMAINV`, `GAMMALN`,
  and `GAMMALN.PRECISE`. The cumulative and quantile forms share one regularized
  incomplete-gamma kernel, the density is built on the same `ln_gamma` primitive, and the origin
  follows Excel's documented pole, limit, and zero cases. Scale ratios and the Lanczos
  normalization stay in log space, preserving valid extreme finite inputs.
- Four beta functions: `BETA.DIST`, `BETADIST`, `BETA.INV`, and `BETAINV`, including the optional
  `A`/`B` interval bounds with the `1 / (B - A)` density Jacobian. The legacy `BETADIST` spelling
  keeps its own typed adapter entry because it has no cumulative flag. Normalized coordinates,
  Jacobians, and inverse interpolation remain finite even when `B - A` itself overflows.
- Seven binomial functions: `BINOM.DIST`, `BINOMDIST`, `BINOM.DIST.RANGE`, `BINOM.INV`,
  `CRITBINOM`, `NEGBINOM.DIST`, and `NEGBINOMDIST`. `BINOM.INV` bisects over the integer support
  and verifies the minimal-`k` contract against regularized incomplete-beta CDF evaluations before
  returning; full-support and degenerate cases resolve exactly without enumerating the support,
  and probability complements stay in `ln1p` form at extreme inputs.
- Two hypergeometric functions: `HYPGEOM.DIST` and `HYPGEOMDIST`. Sample sizes through 10,000 use
  a falling-factorial product that never differences two large `lnΓ` values, which keeps very
  large populations accurate.
- A shared first-party special-function foundation behind all four families: a Lanczos `ln_gamma`,
  regularized incomplete gamma and beta kernels, and a budget-aware safeguarded inverse solver
  whose every bracket and refinement step charges the calculation budget.

### Changed

- The public function catalog now contains 347 official Excel-facing names and 348 accepted
  entries including the non-official OOXML dummy-function marker. Rust, Python, and Node.js report
  the same catalog; none of the twenty new names returns an array.
- Excel Oracle expectations now classify all 60 active 0.1.12 distribution cases as matches in
  both frozen host profiles.

## [0.1.11] - 2026-08-02

### Added

- Twelve database functions: `DAVERAGE`, `DCOUNT`, `DCOUNTA`, `DGET`, `DMAX`, `DMIN`,
  `DPRODUCT`, `DSTDEV`, `DSTDEVP`, `DSUM`, `DVAR`, and `DVARP`. They use typed field and header
  resolution, criteria rows with Excel AND/OR semantics, statically validated formula criteria,
  and cumulative calculation budgets.
- Rank-aware `LINEST` and `LOGEST` statistics plus `TREND` and `GROWTH` prediction arrays, backed by
  normalized row/column inputs, stable moments, and column-pivoted Householder QR.
- `MINVERSE` with partial-pivot LU, bounded `MUNIT` identity arrays, and `ARABIC`/`ROMAN`
  conversion across Excel's distinct parser and formatter domains and all five Roman forms.

### Changed

- The public function catalog now contains 327 official Excel-facing names and 328 accepted
  entries including the non-official OOXML dummy-function marker. Rust, Python, and Node.js expose
  the same array-result flags for the new regression and matrix functions.
- Variance, covariance, regression, and database statistics share stable mergeable moments with a
  compensated sum and explicit sample/population boundaries.
- Excel Oracle expectations now classify all 57 active 0.1.11 cases as matches in both frozen host
  profiles and verify every materialized cell in the Apache POI matrix fixture.
- The MCP server now requires `rmcp` 3.0.1 or newer within the 3.x family and uses its typed
  complete-response wrappers. The checked-in MCP dependency notice also covers the new
  all-target `uuid`/`getrandom` graph. The `v0.1.11` release artifacts were built from the
  tagged commit that includes this upgrade.

### Fixed

- Legacy array formulas now broadcast a scalar result across the declared CSE range, matching
  Excel instead of filling follower cells with `#N/A`.

## [0.1.10] - 2026-08-01

### Added

- Seven bounded array reshape and ordering functions: `EXPAND`, `SORTBY`, `TOCOL`, `TOROW`,
  `TRIMRANGE`, `WRAPCOLS`, and `WRAPROWS`.
- Five text and PCRE2 functions: `ARRAYTOTEXT`, `TEXTSPLIT`, `REGEXEXTRACT`, `REGEXREPLACE`, and
  `REGEXTEST`. Regex compilation, scanning, captures, copies, output, and cancellation are governed
  by the existing calculation limits.
- Five reference and introspection functions: `FORMULATEXT`, `ISFORMULA`, `SHEET`, `SHEETS`, and
  `XMATCH`. `XMATCH` supports exact, wildcard, next-smaller, and next-larger matching with forward,
  reverse, and sorted search modes.
- `GROUPBY`, `PERCENTOF`, and `PIVOTBY`, with typed header, total, sort, filter, relationship, and
  relative-total options. Grouped calculations accept supported built-in aggregate callables and
  compatible `LAMBDA` values.

### Changed

- One typed function registry now drives evaluator dispatch, formula capability and usage scans,
  public catalog metadata, and Rust, MCP, Python, and Node.js catalog serialization. The catalog
  contains 307 official Excel-facing names and 308 accepted entries including the non-official
  OOXML dummy-function marker.
- The new array-producing functions use the shared spill materialization and dependency topology,
  keeping full and incremental recalculation aligned under the same array, text, iteration, and
  cancellation limits.
- Excel Oracle expectations now validate reviewed array divergences by exact materialized range,
  shape, and typed cell contents while preserving host-specific regex availability and localized
  generated total labels.

## [0.1.9] - 2026-07-31

### Added

- Typed non-zero `TableColumnId` values, immutable lookup indexes for tables and columns, and
  containing-table lookup by worksheet address. The original scalar `TableColumn::id()` and
  positional constructor signatures remain compatible; the constructor now rejects a zero
  column ID instead of accepting an invalid OOXML identity.
- Complete read-only table metadata for table type, totals-row visibility and labels,
  calculated-column and totals-row formulas, typed auto-filter criteria and sort conditions,
  dynamic-filter numeric and ISO date-time bounds, and table style flags.
  `XlsxDocument::table_part` exposes the accepted source part for a stable table ID.
- Per-table filter declaration and filter/sort attribute-text read limits with dedicated
  `TooManyTableFilterItems` and `TableFilterTextTooLarge` error codes.
- Lossless typed formula syntax for structured references, spill references, reference union,
  intersection, range operators, 3-D sheet spans, external workbook targets, and sheet-qualified
  defined names, with UTF-8 source spans and bounded source/AST work.
- Structured-table and spill-reference evaluation, typed table/spill dependency topology,
  multi-area reference identity, and the official `AREAS` function. The supported catalog now
  contains 287 Excel-facing names.
- Bounded `analyze_defined_name` APIs that classify rectangular, 3-D, ordered multi-area, empty,
  dynamic, constant, external, invalid, unsupported, and missing names without requiring a
  calculation session. The same query is available in Python and Node.js.
- Atomic stable-ID table rename, table-column rename, and data-row resize operations. Renames update
  worksheet, defined-name, calculated-column, and totals-row formulas; resize can expand a
  header-only empty table and materialize modeled formulas and totals.
- A separate edit schema v2 for Rust interop, Python, Node.js, and MCP. Existing v1 edit shapes are
  unchanged; v2 adds table operations and changed-table IDs.

### Changed

- The workbook fingerprint schema moved from 2 to 3 and now folds the complete persisted table
  metadata model. Table formulas share the existing per-formula and workbook-wide formula-byte
  read budgets.
- Canonical XLSX generation serializes modeled worksheet-table parts, worksheet table
  relationships, and content-type declarations, reopens them for semantic verification, and
  fails closed for opaque metadata or query/XML/differential/custom-style dependencies it cannot
  synthesize.
- Missing `autoFilter@ref` values are represented distinctly from an explicit range and inherit
  the owning table range minus totals rows. Missing `tableColumns@count` values are accepted, and
  mismatches use the actual `tableColumn` children with an `xlsx.table.normalized` diagnostic.
- Table and spill topology now participates in incremental dependency identity. Geometry changes
  safely select a full graph rebuild, while ordinary value edits retain incremental calculation.
- `SessionLimits` now bounds cumulative formula-rewrite formula/source/AST/edit work and table-row
  materialization. Edit staging, final snapshot/table-index construction, interop, and MCP v2
  propagate cooperative cancellation without installing partial state.
- Source-linked table authoring patches the accepted table part by SpreadsheetML namespace and
  exact structural path, preserving foreign namespace and opaque extension content. Canonical and
  source-linked outputs are reopened and compared against the complete table model.

### Fixed

- XLSX reads now diagnose and drop a later table whose range overlaps an already accepted table
  instead of allowing an ambiguous workbook snapshot.
- Filter and sort metadata whose ranges extend outside the owning table are diagnosed and dropped
  with that table. Source-linked writes preserve accepted table parts byte-for-byte, while output
  verification now compares the complete table model.
- Table names, totals metadata, filter/sort tokens and cardinalities, grouped dates, numeric
  thresholds, and icon indexes are validated before they enter the semantic model. Table XML also
  rejects misplaced or non-1.0 declarations, malformed comments, invalid processing-instruction
  targets, non-XML document whitespace, and child-order or filter-choice violations.
- Strict SpreadsheetML dynamic filters reject Transitional-only `maxVal`, validate ISO date-time
  bounds, and retain ambiguous prose cases for source-linked preservation instead of silently
  canonicalizing them. Built-in table-style detection now rejects leading-zero lookalikes such as
  `TableStyleLight01`.
- Table and column identity matching now uses the same Unicode case-insensitive key as workbook
  lookup. Column rename rejects leading/trailing ASCII spaces that OOXML would normalize.
- Table rename no longer rewrites external-workbook or quoted 3-D lookalikes, and malformed related
  formulas fail atomically with their owning cell, name, or table-formula location.
- Table-column rename no longer overwrites annotated header text, and semantic no-ops no longer
  over-report changed cells or rewritten formulas.
- Table-row resize streams validation and materialization instead of allocating all targets first,
  preserves row- and column-oriented sort ranges, and never installs after a resource limit or
  cancellation.

## [0.1.8] - 2026-07-30

### Added

- First-class `LAMBDA` values with lexical closures, immediate invocation, workbook and sheet-local
  defined-name callables, bounded recursion, omitted-argument handling through `ISOMITTED`, and
  case-insensitive `_xlfn.LAMBDA`/`_xlpm.` storage spellings.
- The callable helper family `BYROW`, `BYCOL`, `REDUCE`, `SCAN`, and `MAKEARRAY`. `MAP`, the new
  helpers, and direct invocation share one typed callable kernel, so closures may flow through
  `LET` or a defined name without requiring a literal `LAMBDA(...)` at the call site.
- Excel-oracle v2 host-matrix coverage with 1,527 stable cases, 1,496 active cases, and independent
  Excel Online and Mac Excel 2021 observations.

### Changed

- Function-usage and capability scans expand named LAMBDA bodies, count their official built-ins,
  and no longer report user callable names as unsupported worksheet functions.
- Oracle validation now binds every observed array-result cell to the raw XLSX cache and verifies
  the complete suite, manifest, metadata, observation, feature-set, profile, count, and SHA-256
  provenance chain.

### Fixed

- Named LAMBDA bodies now participate in scheduling and dependency invalidation, so forward
  dependencies calculate in order and incremental results stay identical to a full pass.
- Named LAMBDAs no longer capture caller `LET` bindings. Ordinary defined names and callable bodies
  are evaluated and inspected outside the caller's lexical frame.
- Defined-name cycles and nesting limits can no longer hide behind a callable body and overflow the
  evaluator stack.
- LAMBDA argument errors propagate before body evaluation, a missing body is rejected, helper
  scalar contracts reject multi-cell arrays and references, and callable coercion returns
  `#VALUE!` while a callable left as the final worksheet result returns `#CALC!`.
- `REDUCE` without an initial value now seeds from the first array item and supports array-valued
  accumulators; BYROW and BYCOL charge work by input cells rather than only output dimensions.

## [0.1.7] - 2026-07-29

### Added

- `LET` evaluation with left-to-right single evaluation, lexical shadowing, range and array
  bindings, Excel storage-name normalization, and shared local-name validation. The official
  Excel-facing function catalog now contains 279 names: 266 kernels and 13 aliases.
- Explicit calculation limits for LET bindings, LAMBDA nesting, and cumulative LAMBDA/MAP
  invocations. Resource exhaustion remains a structured unavailable result and cannot be hidden by
  worksheet error-catching functions.
- Excel Online and Mac Excel 2021 saved-workbook expectations. Each workbook is recorded
  independently, and an unavailable saved value is classified as `host_unsupported`.

### Changed

- MAP and LET now share trace-aware scope values. Scalar, array, calculated-cell, and reference
  bindings preserve decimal traces without changing the selected arithmetic mode:
  `ExcelNearZero` still applies Excel cancellation correction and `Ieee754` still returns the raw
  IEEE-754 result.
- The Excel oracle audit now runs in the standard Rust test suite without a separate CI or
  release-only gate. Cases remain available for later patches even when neither recorded Excel
  workbook provides a usable saved value.

### Fixed

- Excel-saved formulas can no longer be replaced silently: only case-declared, token-aware spelling
  rewrites are accepted, and strings, structured references, scoped parameters, and shared-formula
  relative references are preserved.
- Rich errors now resolve through `metadata.xml` and `richData` records, while empty PIVOTBY caches
  are distinguished from the exact circular-reference fixture instead of being mislabeled as
  circular.

## [0.1.6] - 2026-07-28

### Added

- Excel table (ListObject) reading. Worksheets discover their table parts through the worksheet
  relationship graph, and each sheet exposes validated `Table` models: the stable non-zero table
  ID, worksheet-local programmatic `name`, workbook-global formula/UI `displayName`, full range,
  header and totals row counts, and columns in declaration order with the stable XLSX column
  identifier that survives renames. `WorkbookSnapshot::table` resolves `displayName`
  case-insensitively. Snapshot builders reject duplicate IDs and display names workbook-wide,
  duplicate programmatic names within a worksheet, and display-name conflicts with defined names.
  Readers report those conflicts as `xlsx.table.*` warnings and drop only the later table.
- Merged-range reading. Each sheet exposes `merged_ranges()` sorted by top-left address.
  Unparseable, reversed, and single-cell declarations become `xlsx.merged_range.*` warning
  diagnostics; overlapping declarations keep the earlier range in sorted order and drop the later
  one with a diagnostic.
- New read limits with dedicated error codes: `max_merged_ranges`, `max_tables`,
  `max_table_columns`, and `max_table_name_bytes`. Exceeding a limit fails the read with
  `TooManyMergedRanges`, `TooManyTables`, `TooManyTableColumns`, or `TableNameTooLarge`.
- Structured references such as `Table1[Amount]` and `[@Amount]` are now recognized by the lexer
  and classified as the new `calculation.unsupported_structured_reference` capability issue
  instead of a misleading `calculation.parse_error`. External-workbook spellings
  (`[1]Sheet1!A1`, `[Book1.xlsx]Sheet1!A1`, `[1]!Name`) remain parse errors; the two are told
  apart by what follows the closing bracket, never by the bracket contents.
- Interop, Python, Node, and MCP surfaces expose per-sheet `merged_ranges` and `tables`
  summaries, including the table ID and both OOXML names. Both new sheet-summary fields deserialize
  with honest empty defaults, so payloads from producers that predate them keep working.

### Changed

- The workbook fingerprint schema byte moved from 1 to 2 and now folds merged ranges and the
  complete table model, so table or merge differences can no longer allow a stale calculation to
  be written into a package. Fingerprints persisted under the previous schema are intentionally
  invalidated.
- Opening a table-bearing worksheet's relationship part applies the same OPC structural
  strictness the workbook-level relationships always had. Worksheets without `<tableParts>`
  never open their relationship part, so table-free workbooks see no behavior change.
- Duplicate `<mergeCells>` elements in one worksheet now fail the read as structurally invalid,
  matching the existing duplicate-`sheetData` policy.

### Fixed

- Table parsing now requires and preserves CT_Table `id`, validates `tableColumns@count` against
  the actual children, and charges every declared column even inside a duplicate invalid
  `tableColumns` container.
- `max_tables` is charged when each `<tablePart>` declaration is encountered, so declarations
  with missing or unresolved relationship IDs can no longer bypass the workbook-wide parse budget.

## [0.1.5] - 2026-07-28

### Added

- Direct 3-D references for the audited aggregate family: `SUM`, `AVERAGE`, `AVERAGEA`, `COUNT`,
  `COUNTA`, `MAX`, `MAXA`, `MIN`, `MINA`, `PRODUCT`, `STDEV.P`, `STDEV.S`, `VAR.P`, and `VAR.S`,
  including the catalogued legacy aliases. Inclusive workbook tab order determines the span, so
  hidden sheets participate and reverse-written endpoints are equivalent.
- Integration coverage for the complete supported-function catalog, hidden sheets, reverse spans,
  whole-column 3-D inputs, and a fuzz invariant that compares 3-D `SUM` with an explicit sheet
  fold under the exact array-cell budget.

### Changed

- The capability scanner and evaluator now share one explicit direct-3-D-consumer policy.
  `INDEX` and `VLOOKUP` return Excel's `#VALUE!`, `OFFSET` returns `#REF!`, and other consumers
  remain an `UnsupportedSheetRange` engine capability issue.
- Incremental calculation stores 3-D dependencies as one compact rectangle span per reference
  instead of one rectangle per sheet, so compiled state grows with the number of references rather
  than with the number of sheets in the span. The dependency-edge budget is unchanged: it still
  counts the distinct edges a span actually produces.
- Whole-column operands combined by unary or binary array operators use a common extent: the
  greatest populated row among the columns those operands reference, with a one-row minimum for an
  otherwise empty sheet and blanks for unpopulated cells. Directly resolved sources and operator
  outputs share one cumulative array-cell budget. Function arguments retain independent bounded
  contexts, and a returned array joins an enclosing cumulative context only when another direct
  whole-column operand established it.
- Four existing Excel-saved 3-D cases and two whole-column-array cases were activated by changing
  only their reviewed classifications; the oracle workbook and saved Excel results were not
  regenerated.

### Fixed

- Edits to the first, middle, or last sheet of a 3-D dependency now dirty the dependent formula,
  while edits outside the span do not.
- Reference-returning functions such as multi-cell `OFFSET` remain usable as aggregate inputs
  after the compact sheet-span representation is introduced.
- A whole-column array expression no longer takes its height from the sheet-wide used range. That
  made the value depend on cells outside every dependency rectangle, so writing into a column the
  expression never named changed the answer a full recalculation produced while leaving the
  incremental pass holding the previous one. `COUNT` and `AVERAGE` over whole-column products were
  affected; `SUM` was not, because the extra rows fold to zero.
- A defined name that resolves to a 3-D reference is now classified the same way regardless of
  operand order. The capability scan reached each name once per formula, so in
  `SUM(Name)+COUNTBLANK(Name)` whichever operand the walk visited first decided the whole formula,
  and that order reported an unsupported formula as supported.
- A 3-D operand of the range operator now returns Excel's `#VALUE!`, matching both the capability
  scan for that position and the existing answer for a range whose endpoints sit on different
  sheets, instead of an engine-capability error the scan did not predict.

## [0.1.4] - 2026-07-27

### Added

- Committed Excel-saved binary oracle workbooks and an explicit local audit command replace the
  generated JSON conformance matrix. The audit is not part of CI or publication.
- `CalculationHints` retains the optional XLSX iterative-calculation declaration across read and
  write operations, and it is carried by the interop DTO and the Python, Node.js, and MCP
  boundaries alongside the calculation mode and the host recalculation flags. `CalculationHints`
  describes the whole `calcPr` declaration, so applying one replaces the workbook's declaration
  rather than merging into it: use `with_iterative_calculation` to carry the flag through an
  edit that would otherwise clear it. CellRune still does not perform iterative calculation; the
  declaration is preserved, not obeyed.

### Changed

- `ExcelNearZero` now matches Excel's observed narrow correction boundary:
  `=0.1+0.2-0.3` becomes zero, while `=100.1-100-0.1` keeps its residue.
- CI runs the normal Rust regression suite; Excel oracles, external corpora, engine comparisons,
  and accuracy metrics remain explicit local development checks.
- Release validation and publication dependencies are separated by core, Python, Node.js, and MCP
  artifact family.

### Fixed

- Formula analysis now handles nested calls, defined names, and lambda shadowing consistently,
  preventing stale incremental results and misclassified volatile-input failures.

## [0.1.3] - 2026-07-27

### Added

- `CellPhonetics::resolved_runs` returns each phonetic run's range as byte offsets into the cell's
  base text, translated from the UTF-16 code units XLSX stores, as `ResolvedPhoneticRun`.
- `DocumentPresentation::phonetic_cell_entries` iterates a sheet's cells that carry at least one
  run, in address order. Cells holding only `phoneticPr` display properties without `rPh` runs are
  skipped and remain reachable through `cell_phonetics`.
- `ArithmeticSemantics` on `CalculationOptions` selects how arithmetic that cancels to near zero is
  handled. The default `ExcelNearZero` returns `0` for `=0.1+0.2-0.3`, in the operator path, the
  array path, and `SUM`, `AVERAGE`, `SUMIF(S)`, `AVERAGEIF(S)`, `SUBTOTAL`, `SUMPRODUCT` and `NPV`.
  `Ieee754` keeps the residue, as 0.1.2 did.
- `FinancialSolverSemantics` on `CalculationOptions` selects the solver budget. The default
  `ExcelIterationBudget` applies Microsoft's documented policy — 20 iterations/`1e-7` for `IRR` and
  `RATE`, 100/`1e-8` for `XIRR`. `ExtendedSearch` searches longer, as 0.1.2 did.
- Both options are carried by the interop DTO and exposed by the Python, Node.js, and MCP
  boundaries. `docs/NUMERICS.md` records the policies.

### Changed

- **Calculated numbers change by default.** Arithmetic that cancels to near zero now returns zero,
  and the financial solvers now give up where Excel gives up instead of searching four times
  longer. Callers who depended on 0.1.2's numbers should select `ArithmeticSemantics::Ieee754` and
  `FinancialSolverSemantics::ExtendedSearch`; callers comparing against Excel need no change.
- CellRune is dual-licensed under `MIT OR Apache-2.0` instead of MIT alone. Releases 0.1.0 through
  0.1.2 remain under MIT; the dual license applies from 0.1.3 onward. Every distributed artifact
  now carries `LICENSE-MIT` and `LICENSE-APACHE` in place of the single `LICENSE` file.
- The packaged-license CI gate enumerates its copies from the distribution directories rather than
  naming each file. The previous list covered three of the eleven copies, so the eight npm platform
  packages carried license text no gate compared against the original.

## [0.1.2] - 2026-07-26

### Added

- A single data-driven conformance expectation suite. `conformance/` holds redistributable
  matrices — every literal, every formula, and the value a recorded oracle saved for it — and
  the normal workspace test run reconstructs every matrix and compares CellRune's calculation
  against the oracle. New functions and syntax extend the same data set.
  The first matrix is the Apache POI `FormulaEvalTestData` corpus (Apache-2.0): 1,295 cases
  against a saved Microsoft Excel 2013 calculation cache, 1,290 matching within a
  scale-relative `1e-8` and 5 divergences documented case by case. A documented divergence is
  enforced in both directions, so neither the oracle side nor the CellRune side of it can drift
  silently, and a divergence without an explanatory note fails the test. An extraction tool
  (`extract_conformance_matrix`) accepts ordinary and resolved shared formulas and rejects
  workbook metadata the v1 matrix cannot reproduce.
- A semver invariant inside the existing test suite (`tests/public_api.rs`). The sixteen exported
  enums that are not `non_exhaustive` are matched without wildcard arms with every variant
  payload bound at its exact type, and function-pointer constants pin the positional signatures
  of `WorkbookSnapshot::new_with_metadata` and the monomorphic read → calculate → write entry
  points. This remains one test as the API grows.

### Changed

- Rust dependency requirements now declare the oldest API-compatible, advisory-clean versions
  instead of using the release that happened to be newest when each manifest was edited. All
  workspace requirements live in the root manifest, the lockfile still selects the reproducible
  current graph, and every release family a broad range admits is exercised by the floor or the
  newest-compatible suite. Fifteen of the twenty external workspace floors moved down; five
  remain at their current releases because they are the first secure version or belong to the
  same release family (`quick-xml`, PyO3 and its build helper, and `calamine`), or because their
  types appear in CellRune's public signatures and a public dependency must stay within one
  semver family (`cap-std`). The fuzz crate's `libfuzzer-sys` stays exact-pinned as a tool
  input: its graph is excluded from the workspace and always built `--locked`, so no suite
  could exercise a range there.
- The `conformance/` fixtures now live in `binding-contract/`, which is what they are: a
  cross-binding API behavior contract exercised by the Rust interop, Python, and Node test
  suites. Two 1 KB fixtures cannot carry an Excel conformance claim, so the `conformance/` name
  is reserved for the Excel expectation suite that can.
- The nightly sanitizer campaign remains one job over four stable trust boundaries. It records
  every target's result before failing, so one crash cannot starve later campaigns and new
  functions do not create new fuzz jobs.
- The MSRV check covers `--all-targets` rather than `--lib`, so tests, benches, and the
  extraction tool must also build on the declared floor.
- The 50k benchmark is demoted from a scheduled nightly job and a required release gate to an
  explicit observational command, and its output-verifier script is removed with the jobs. The
  benchmark keeps its internal correctness assertions, and functional correctness remains
  covered by the generated-workbook tests.
- Release publish jobs are re-run safe. The crates.io, npm, and GitHub-release jobs probe for
  what is already live and skip it — the GitHub release also counts its attached bundles, so an
  interrupted asset upload is completed rather than skipped — and the PyPI upload passes
  `skip-existing`. A rerun of a partially published tag therefore converges on fully published
  instead of ending red: the final runs of both prior releases stayed red because their
  already-published registries could only fail again or be rejected by hand.

### Fixed

- A workbook containing an empty sheet was rejected with `xlsx.invalid_worksheet`. Excel and
  every mainstream producer serialize an empty sheet as a self-closing `<sheetData/>`, which
  the reader's required-sheetData rule did not recognize as the element's empty form, so even a
  brand-new workbook saved by Excel failed to open. Empty sheets now read as sheets with no
  cells; a duplicated `<sheetData/>` is still rejected.

### Documentation

- The README carries a Verification section: the public conformance-suite numbers with their
  exact oracle, and the private-audit results (Excel for Mac 16.111 corpus recalculation,
  14-workbook full-span audits, the 250k-formula timing, and the 4-of-4 producer matrix) with
  their provenance and non-distribution stated.

## [0.1.1] - 2026-07-25

### Changed

- Linux wheels target a `manylinux_2_28` (glibc 2.28) baseline instead of `manylinux_2_35`. The
  previous tag came from whichever glibc the build runner happened to ship, and it excluded RHEL 9
  and Amazon Linux 2023 by 0.01 of a glibc version; both fell back to the source distribution,
  which requires a Rust toolchain the installing user rarely has. The baseline is now an explicit
  build input, produced with the same pinned Zig the npm platform packages already use, and the
  resulting platform tag is asserted during release verification rather than assumed.
- Dependency requirements of the published `cellrune` crate are caret ranges instead of `=` pins.
  An exact pin made the crate unresolvable alongside any dependent needing a newer patch of the
  same dependency, with no remedy available to that dependent. Build reproducibility continues to
  come from the committed `Cargo.lock`. Two new scheduled gates bound the ranges: one runs the test
  suite against the declared floors on the minimum supported Rust version, and one runs it against
  the newest compatible graph. Both resolve live rather than from the lockfile, so they are
  scheduled alongside the advisory tier rather than gating pull requests: a failure there means
  upstream moved, not that the commit under test did.

### Fixed

- Quoted external-workbook references such as `'[1]Sheet1'!A1` were reported as supported and
  evaluated to `#REF!`. Because that is a spreadsheet error value rather than an engine issue,
  `IFERROR` could hide it, so a workbook could return a plausible number for a formula the engine
  cannot evaluate. Both the capability scan and evaluation now reject the external prefix. The
  unquoted spelling was already rejected, as the formula lexer has no bracket token. A reference
  built at run time by `INDIRECT` is the deliberate exception: the scan cannot read inside the
  text argument, so it reports the cell as supported, and evaluation answers `#REF!` there as
  Excel does rather than an engine issue the scan never predicted and no caller could recover
  from. An external prefix written as a saved path keeps its drive letter and is reported once,
  not also as a 3-D sheet range the formula does not contain.
- Calculated zeros could be negative. `Iterator::sum` for `f64` folds from `-0.0`, `f64::min` and
  `f64::max` may return either operand when both compare equal, and `Iterator::product` inherits
  the sign of an odd number of negative factors, so `SUM`, `MIN`, `MAX`, `PRODUCT`, and the `*A`
  variants could each report `-0` where Excel reports `0`. The boundary every calculated value
  crosses now normalizes negative zero, rather than each kernel being expected to.
- Formula parse-error details labelled every position as `token N`, but a lexing failure has no
  token stream and was reporting a character offset under that label. Lexing failures now report
  `character N` and parsing failures continue to report `token N`.
- The lambda function group dispatched without inspecting the normalized function name, unlike
  every sibling group. A second function in that group would have silently evaluated as `MAP`.
- A release version bump had to be repeated by hand in more places than the release checklist
  listed, and the ones it missed — a pnpm lockfile, a generated loader shim, the packaged-consumer
  lockfile, and the release verification scripts — fail only during a release run. Every version
  is now derived from the workspace manifest where possible, and a new gate asserts that the
  remaining declarations agree, including the install commands and the supported-version statement
  that ship on the registry project pages. Every check in that gate fails when its pattern matches
  nothing, so a moved file or a regenerated shim cannot report the same green result as a correct
  bump, and the gate has its own tests.
- A Linux wheel could declare a compressed platform-tag set whose first component was a legacy
  alias, which hid a newer glibc requirement behind it from the release verification. Every
  component of the set is now checked against the supported baseline.
- The nightly fuzz tier could not install its fuzzer. `taiki-e/install-action` has no cargo-fuzz
  manifest, the job deliberately disables the binstall fallback, and the schedule had never fired
  since the repository went public, so the defect surfaced only on the tier's first manual
  dispatch. cargo-fuzz is now built from crates.io with `cargo install --locked`, which keeps the
  exact-version pin and the registry checksum verification.
- The npm publish job could not authenticate. Trusted publishing requires npm 11.5.1 to exchange
  the job's OIDC identity for a registry credential, and Node 22 bundles npm 10, which publishes
  without credentials and receives the rejection masked as E404. The nine-package dry-run gate
  cannot catch this because a dry run never authenticates, so the failure surfaced on the first
  real upload — before any package was published, leaving the retry clean. The job now upgrades
  to a pinned npm 11 before publishing.

### Documentation

- The 0.1.0 entry claimed capability detection for unsupported external, structured, and
  spill-postfix formula forms. Only 3-D and data-table forms have dedicated detection; the other
  three surface as `calculation.parse_error`, and structured references and spill-postfix
  references still do after this release. The entry has been corrected.
- Added `docs/NUMERICS.md`, which records where calculated values deliberately differ from Excel,
  the Excel build each statement was measured against, and which function families are unmeasured.
  It is linked absolutely: both published READMEs are also the long description crates.io, PyPI,
  and npm render, none of which resolve a repository-relative path, and a gate now enforces that.
- Installation instructions no longer describe the registry artifacts as pending.

## [0.1.0] - 2026-07-25

### Added

- Bounded `.xlsx` package inspection and reading from paths, byte slices, and `Read + Seek`
  streams, with ZIP, XML, workbook, formula, text, relationship, and allocation limits.
- Immutable sparse workbook snapshots with typed values, formulas, saved results, shared and array
  metadata, defined names, sheet visibility, date systems, number formats, diagnostics,
  and provenance.
- Package-backed `.xlsx` and `.xlsm` documents with exact SHA-256 source identity that preserve
  unknown and unchanged parts without executing macros, following external links, or reading
  host-time inputs.
- Static formula capability scans and deterministic recalculation into a separate owned result
  snapshot, including direct formula cells, legacy-array regions, and dynamic-spill regions.
- A catalog of 278 official Excel-facing function names, comprising 265 official calculation
  kernels and 13 compatibility aliases, plus one non-official OOXML dummy-function marker,
  workbook function-demand reports, and stable sample locations.
- Explicit deterministic inputs for `TODAY()` and `NOW()`, bounded calculation work, stable
  per-formula issue codes, and spreadsheet error values kept distinct from unsupported engine
  capabilities.
- Formula parsing and evaluation for scalar, range, lookup, logical, aggregate, math,
  trigonometric, combinatoric, engineering, information, text, date/time, dynamic-array,
  statistical, and financial function groups.
- Excel-compatible `INDEX` zero row/column references, including scalar implicit intersection,
  reference composition, incremental dependencies, and row, column, or full-rectangle array
  materialization.
- Capability detection for unsupported 3-D and data-table formula forms.
- Verified recalculation writing that binds results to the exact source, updates typed caches,
  removes stale calculation chains, preserves unrelated package content, and supports strict or
  explicit cache-invalidation policy.
- Canonical workbook creation with dynamic-formula authoring, plus preservation-aware
  `WorkbookDraft` editing for typed cells, normal formulas, sheets, defined names, number formats,
  date systems, and calculation properties. Existing document-backed dynamic formulas can be
  recalculated and cache-rewritten, while adding or replacing them fails closed.
- Atomic typed edit batches with semantic revisions, complete rollback on validation failure,
  no-op revision and calculation preservation, and Save As path writes that refuse implicit
  destination replacement.
- SpreadsheetML phonetic annotation inspection and authoring, row/column phonetic-visibility
  handling, and default frozen-pane inspection and authoring under separate presentation revisions.
- Persistent `WorkbookCalculationSession` state with incremental recalculation, conservative full
  fallback, optimistic revision checks, request-owned cooperative cancellation, stale-result
  rejection, and bounded paged result deltas.
- Typed Python 3.10–3.14 bindings built with PyO3 and maturin.
- Typed Node.js 22+ CommonJS and ESM bindings built with napi-rs and exact-version native platform
  packages.
- Explicit, idempotent native-session cleanup through Python context managers and Python/Node.js
  `close()`, including cooperative cancellation of active calculation and stable closed-session
  errors.
- A local stdio-only MCP server with 11 high-level workbook tools, explicit approved roots,
  bounded TTL/LRU sessions, capability-bound bounded input reads, byte-bounded responses and
  resource pagination, cooperative cancellation, and capability-bound atomic Save As behavior.
- Stable validation, read, write, calculation, session, diagnostic, and spreadsheet-value error
  boundaries with machine-readable codes, including a core-owned `ValidationErrorCode` covering
  every current `ValidationError` variant.
- Runnable examples for inspection, capability scanning, error handling, saved-versus-calculated
  values, calculation, canonical authoring, recalculation Save As, dynamic arrays, phonetic
  inspection, and phonetic authoring.
- Generated workbook integration tests, binding conformance tests, MCP protocol tests, fuzz
  targets, MSRV checks, dependency-policy checks, package-consumer verification, and
  cross-platform CI.

### Known limitations

- Public CI uses generated, redistributable workbook fixtures. The private external formula corpus,
  user workbook corpus, and native-producer evidence used during development are not distributed
  with 0.1.0 and are not represented as release gates.

[Unreleased]: https://github.com/emulette/cellrune/compare/v0.1.26...HEAD
[0.1.26]: https://github.com/emulette/cellrune/compare/v0.1.25...v0.1.26
[0.1.25]: https://github.com/emulette/cellrune/compare/v0.1.24...v0.1.25
[0.1.24]: https://github.com/emulette/cellrune/compare/v0.1.23...v0.1.24
[0.1.23]: https://github.com/emulette/cellrune/compare/v0.1.22...v0.1.23
[0.1.22]: https://github.com/emulette/cellrune/compare/v0.1.21...v0.1.22
[0.1.21]: https://github.com/emulette/cellrune/compare/v0.1.20...v0.1.21
[0.1.20]: https://github.com/emulette/cellrune/compare/v0.1.19...v0.1.20
[0.1.19]: https://github.com/emulette/cellrune/compare/v0.1.18...v0.1.19
[0.1.18]: https://github.com/emulette/cellrune/compare/v0.1.17...v0.1.18
[0.1.17]: https://github.com/emulette/cellrune/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/emulette/cellrune/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/emulette/cellrune/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/emulette/cellrune/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/emulette/cellrune/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/emulette/cellrune/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/emulette/cellrune/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/emulette/cellrune/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/emulette/cellrune/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/emulette/cellrune/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/emulette/cellrune/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/emulette/cellrune/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/emulette/cellrune/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/emulette/cellrune/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/emulette/cellrune/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/emulette/cellrune/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/emulette/cellrune/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/emulette/cellrune/releases/tag/v0.1.0
