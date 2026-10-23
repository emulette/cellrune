# Error codes

Every CellRune failure carries a stable dotted `code` for programmatic handling, and every
calculated cell CellRune cannot calculate carries a stable issue `code`. A code keeps its meaning
across releases; a patch release may add codes but never renames or removes one. Messages are for
people and may be reworded.

The same code reaches every surface:

| Surface | Where the code is |
| --- | --- |
| Rust | the `code()` of the error, such as `XlsxReadError::code()`, as a typed enum with `as_str()` |
| Interop | `InteropError::code()`, with the lower-layer code also in `details.source_code` |
| Python | `CellRuneError.code`, with `CellRuneError.kind` and `CellRuneError.details` |
| Node.js | `CellRuneError.code`, with `kind` and `details` |
| MCP | the `code` of a tool's structured error result, with `kind` and `details` |

A failure that exceeded a resource limit also carries its figures in `details.limit`, and an
unavailable result that exceeded a calculation limit names it in `limit`; see the
[README](../README.md).

## Rust core

These codes come from the `cellrune` crate. Interop, Python, Node.js, and MCP report them unchanged as the error `code`, or as the `code` of an unavailable result, and repeat them in `details.source_code`.

### Reading a package (`XlsxErrorCode`)

`XlsxReadError::code()` when a package cannot be opened or trusted. Limit failures also report `observed`, and phonetic run limits the limit `name`.

| Code | Meaning |
| --- | --- |
| `xlsx.io` | The input could not be read or sought. |
| `xlsx.invalid_zip` | ZIP structures are malformed. |
| `xlsx.archive_too_large` | Input archive bytes exceed the configured limit. |
| `xlsx.too_many_entries` | ZIP entry count exceeds the configured limit. |
| `xlsx.entry_too_large` | One uncompressed entry exceeds the configured limit. |
| `xlsx.total_uncompressed_too_large` | Total uncompressed bytes exceed the configured limit. |
| `xlsx.declared_size_mismatch` | An entry produced a different byte count than its central directory declared. |
| `xlsx.compression_ratio_exceeded` | An entry exceeds the configured compression ratio. |
| `xlsx.overlapping_entries` | Compressed byte ranges overlap. |
| `xlsx.encrypted_entry` | An encrypted entry cannot be safely read without credentials. |
| `xlsx.unsupported_compression` | An entry uses a compression method outside the XLSX reader subset. |
| `xlsx.invalid_part_name` | A package part name is malformed or escapes the package root. |
| `xlsx.duplicate_part` | Two ZIP entries normalize to the same package part. |
| `xlsx.missing_part` | A required package part is missing. |
| `xlsx.invalid_xml` | Required XML is malformed. |
| `xlsx.xml_depth_exceeded` | Required XML exceeds the configured nesting depth. |
| `xlsx.xml_attributes_exceeded` | One XML element exceeds the configured attribute count. |
| `xlsx.forbidden_xml_construct` | DTD or another forbidden XML construct is present. |
| `xlsx.invalid_content_types` | `[Content_Types].xml` is incomplete or inconsistent. |
| `xlsx.invalid_relationships` | A Relationship part is incomplete or inconsistent. |
| `xlsx.missing_workbook_relationship` | No internal Office Document relationship identifies the workbook. |
| `xlsx.duplicate_workbook_relationship` | More than one internal Office Document relationship identifies a workbook. |
| `xlsx.external_workbook_relationship` | The Office Document relationship points outside the package. |
| `xlsx.invalid_relationship_target` | A Relationship target cannot be normalized safely. |
| `xlsx.missing_worksheet_relationship` | The workbook has no internal worksheet relationship. |
| `xlsx.unsupported_content_type` | A required part has a content type outside the supported XLSX subset. |
| `xlsx.invalid_workbook` | Workbook metadata is malformed or inconsistent with relationships. |
| `xlsx.too_many_sheets` | Workbook sheet count exceeds the configured limit. |
| `xlsx.missing_sheet_relationship` | Workbook sheet metadata references no matching worksheet relationship. |
| `xlsx.invalid_shared_strings` | Shared-string XML or its declared counts are invalid. |
| `xlsx.too_many_shared_strings` | Shared-string count exceeds the configured limit. |
| `xlsx.shared_string_too_large` | One decoded shared string exceeds the configured limit. |
| `xlsx.total_shared_strings_too_large` | Combined decoded shared strings exceed the configured limit. |
| `xlsx.total_cell_text_too_large` | Text materialized into cells exceeds the configured workbook-wide byte limit. |
| `xlsx.invalid_styles` | Styles XML is malformed or inconsistent. |
| `xlsx.invalid_style_index` | A cell references a missing cell-format record. |
| `xlsx.invalid_worksheet` | Worksheet XML structure is malformed. |
| `xlsx.invalid_cell_reference` | A worksheet cell reference is malformed or outside Excel bounds. |
| `xlsx.too_many_cells_in_sheet` | One worksheet exceeds the configured cell-element limit. |
| `xlsx.too_many_cells` | The workbook exceeds the configured total cell-element limit. |
| `xlsx.unsupported_cell_type` | A literal cell type is outside the supported XLSX reader subset. |
| `xlsx.invalid_cell_value` | A literal cell value does not match its declared type. |
| `xlsx.invalid_defined_name` | Workbook defined-name metadata is malformed. |
| `xlsx.too_many_defined_names` | Workbook defined-name count exceeds the configured limit. |
| `xlsx.formula_too_large` | Formula text exceeds the configured decoded byte limit. |
| `xlsx.total_formula_bytes_too_large` | Materialized formula text exceeds the configured workbook-wide byte limit. |
| `xlsx.invalid_formula_metadata` | Formula container metadata is malformed or inconsistent. |
| `xlsx.invalid_cell_metadata` | Cell metadata or a referenced metadata index is malformed. |
| `xlsx.invalid_phonetic_metadata` | Phonetic run, property, or visibility metadata is malformed. |
| `xlsx.too_many_phonetic_runs` | A phonetic run count exceeds the configured limit. |
| `xlsx.phonetic_text_too_large` | One decoded phonetic string exceeds the configured limit. |
| `xlsx.total_phonetic_text_too_large` | Combined decoded phonetic strings exceed the configured limit. |
| `xlsx.too_many_annotated_cells` | Annotated cell references exceed the configured limit. |
| `xlsx.invalid_frozen_pane` | Frozen-pane or sheet-view metadata is malformed. |
| `xlsx.too_many_merged_ranges` | Merged-range declarations exceed the configured limit. |
| `xlsx.too_many_tables` | Referenced table parts exceed the configured limit. |
| `xlsx.too_many_table_columns` | One table definition exceeds the configured column-count limit. |
| `xlsx.table_name_too_large` | A table, display, or column name exceeds the configured byte limit. |
| `xlsx.too_many_table_filter_items` | One table definition exceeds the configured filter-item count limit. |
| `xlsx.table_filter_text_too_large` | Filter and sort attribute text in one table exceeds the configured byte limit. |

### Writing a package (`XlsxWriteErrorCode`)

`XlsxWriteError::code()` when a verified package cannot be written.

| Code | Meaning |
| --- | --- |
| `xlsx.write.io` | The output sink or filesystem operation failed. |
| `xlsx.write.invalid_package_plan` | The immutable package plan is internally inconsistent. |
| `xlsx.write.source_identity_mismatch` | A write plan or calculation belongs to a different input package. |
| `xlsx.write.stale_semantic_revision` | A calculation predates the current semantic workbook revision. |
| `xlsx.write.incomplete_calculation` | Strict output was requested but one or more required results are unavailable. |
| `xlsx.write.unsupported_result_materialization` | A typed calculation result cannot be materialized safely. |
| `xlsx.write.output_kind_mismatch` | The requested output kind conflicts with the package content. |
| `xlsx.write.unsupported_preservation` | Required package content cannot be preserved without loss. |
| `xlsx.write.conflicting_part_operation` | More than one operation targets the same package part. |
| `xlsx.write.dangling_relationship` | A generated relationship has no valid target. |
| `xlsx.write.invalid_generated_xml` | Generated XML does not satisfy the writer's own validation contract. |
| `xlsx.write.destination_exists` | The destination exists and replacement was not explicitly enabled. |
| `xlsx.write.atomic_replace_failed` | Explicit atomic replacement could not be completed. |
| `xlsx.write.resource_limit_exceeded` | A configured write resource budget was exceeded. |
| `xlsx.write.output_verification_failed` | Reopening or validating the completed output failed. |

### Workbook validation (`ValidationErrorCode`)

`ValidationError::code()` when an edit, name, address, or workbook invariant is rejected.

| Code | Meaning |
| --- | --- |
| `validation.row_out_of_range` | A row index is outside the supported Excel range. |
| `validation.column_out_of_range` | A column index is outside the supported Excel range. |
| `validation.cell_address_invalid` | A cell address is not valid A1 notation. |
| `validation.range_start_after_end` | A range start is after its end. |
| `validation.sheet_id_zero` | A sheet identifier is zero. |
| `validation.sheet_name_empty` | A sheet name is empty. |
| `validation.sheet_name_too_long` | A sheet name exceeds Excel's length limit. |
| `validation.sheet_name_invalid_character` | A sheet name contains a forbidden character. |
| `validation.sheet_name_apostrophe_boundary` | A sheet name begins or ends with an apostrophe. |
| `validation.duplicate_sheet_id` | A workbook contains a duplicate sheet identifier. |
| `validation.duplicate_sheet_name` | A workbook contains a duplicate case-insensitive sheet name. |
| `validation.duplicate_cell` | A sheet contains a duplicate cell address. |
| `validation.defined_name_empty` | A defined name is empty. |
| `validation.defined_name_too_long` | A defined name exceeds Excel's length limit. |
| `validation.defined_name_control_character` | A defined name contains a control character. |
| `validation.defined_name_unknown_sheet` | A sheet-scoped defined name references an unknown sheet. |
| `validation.duplicate_defined_name` | A workbook contains a duplicate defined name in one scope. |
| `validation.non_finite_number` | A numeric cell contains NaN or infinity. |
| `validation.formula_empty` | Formula text is empty. |
| `validation.xlsx_formula_has_leading_equals` | Stored XLSX formula text includes a leading equals sign. |
| `validation.user_formula_missing_leading_equals` | User formula text omits a leading equals sign. |
| `validation.source_id_empty` | A source identifier is empty. |
| `validation.diagnostic_code_invalid` | A diagnostic code does not follow the stable code grammar. |
| `validation.provider_name_empty` | A provenance provider name is empty. |
| `validation.provider_version_empty` | A provenance provider version is empty. |
| `validation.diagnostic_message_empty` | A diagnostic message is empty. |
| `validation.unknown_sheet_id` | A draft operation references an unknown sheet. |
| `validation.cell_not_found` | A draft operation references a missing sparse cell. |
| `validation.sheet_id_exhausted` | No additional nonzero sheet identifier can be allocated. |
| `validation.last_visible_sheet` | A workbook edit would hide its last visible sheet. |
| `validation.semantic_revision_exhausted` | The semantic revision counter is exhausted. |
| `validation.presentation_revision_exhausted` | The presentation revision counter is exhausted. |
| `validation.phonetic_range_empty` | A phonetic range is empty or reversed. |
| `validation.phonetic_range_out_of_bounds` | A phonetic range exceeds its base text. |
| `validation.phonetic_range_splits_surrogate` | A phonetic range boundary splits a UTF-16 surrogate pair. |
| `validation.phonetic_runs_out_of_order` | Phonetic runs are unordered or overlapping. |
| `validation.phonetic_text_empty` | A phonetic run contains no text. |
| `validation.phonetic_text_invalid_character` | Phonetic text contains a character forbidden by XML 1.0. |
| `validation.phonetics_require_text_cell` | Phonetic annotations target a non-text cell. |
| `validation.phonetic_font_id_unsupported` | A phonetic annotation references an unsupported font record. |
| `validation.annotated_text_replacement_required` | A normal value edit would discard an existing annotation. |
| `validation.array_formula_part_edit` | A cell edit targets a legacy array formula range away from its anchor. |
| `validation.frozen_rows_out_of_range` | A frozen row count cannot be represented within worksheet bounds. |
| `validation.frozen_columns_out_of_range` | A frozen column count cannot be represented within worksheet bounds. |
| `validation.font_name_invalid` | A font name is empty, too long, or contains a control character. |
| `validation.format_value_out_of_range` | A font size, color tint, column width, row height, indent, or text rotation is out of range. |
| `validation.built_in_number_format_id` | A built-in number format uses a custom-format identifier. |
| `validation.custom_number_format_id` | A custom number format uses a reserved built-in identifier. |
| `validation.number_format_code_empty` | A custom number format code is empty. |
| `validation.table_id_zero` | A table identifier is zero. |
| `validation.table_column_id_zero` | A table column identifier is zero. |
| `validation.table_name_empty` | A table name is empty. |
| `validation.table_name_too_long` | A table name exceeds Excel's length limit. |
| `validation.table_name_invalid_character` | A table name contains a forbidden character. |
| `validation.table_name_reference_conflict` | A table name is an A1 or R1C1 reference. |
| `validation.duplicate_table_display_name` | A workbook contains a duplicate case-insensitive table display name. |
| `validation.duplicate_table_id` | A workbook contains a duplicate table identifier. |
| `validation.duplicate_table_programmatic_name` | One worksheet contains a duplicate programmatic table name. |
| `validation.table_display_name_conflicts_with_defined_name` | A table display name conflicts with a defined name. |
| `validation.table_columns_empty` | A table declares no columns. |
| `validation.table_column_count_mismatch` | A table's column count disagrees with its range width. |
| `validation.table_column_name_empty` | A table column name is empty. |
| `validation.table_header_requires_text` | A table header cell edit is not a text column name. |
| `validation.table_column_name_too_long` | A table column name exceeds Excel's length limit. |
| `validation.table_column_name_invalid_character` | A table column name contains a character forbidden by XML 1.0. |
| `validation.table_column_name_space_boundary` | A table-column authoring name begins or ends with an ASCII space. |
| `validation.duplicate_table_column_name` | A table contains a duplicate case-insensitive column name. |
| `validation.duplicate_table_column_id` | A table contains a duplicate column identifier. |
| `validation.overlapping_tables` | Two tables on one worksheet have overlapping ranges. |
| `validation.table_row_counts_exceed_range` | A table's header and totals rows exceed its range height. |
| `validation.invalid_table_totals_metadata` | A table column contains inconsistent totals metadata. |
| `validation.unknown_table_id` | A draft operation references an unknown table ID. |
| `validation.unknown_table_column_id` | A draft operation references an unknown table column ID. |
| `validation.table_data_rows_reversed` | A table resize declares reversed data rows. |
| `validation.table_resize_header_underflow` | A table resize cannot place its header above the first data row. |
| `validation.table_materialization_collision` | Table materialization would overwrite non-equivalent worksheet content. |
| `validation.unsupported_table_authoring_metadata` | Table metadata cannot be safely authored. |
| `validation.table_totals_cell_occupied` | A table widening adds a column whose totals-row cell is not empty. |
| `validation.sheet_auto_filter_not_found` | A draft operation targets a sheet without a sheet-level auto-filter. |
| `validation.sheet_auto_filter_overlaps_table` | A sheet-level auto-filter and a table would overlap. |
| `validation.column_widening_range_invalid` | A column widening changes the row span or does not contain the current range. |
| `validation.column_widening_overlap` | Widened columns overlap a merged range, legacy array formula, or declared spill range. |
| `validation.formula_rewrite_parse_failed` | A target-related formula could not be parsed for typed rewriting. |
| `validation.table_range_overlap` | A table range overlaps a merged range or legacy array formula. |
| `validation.table_range_invalid` | A table range drops its header row, leaves the table's columns, or holds no row below the header. |
| `validation.defined_name_rename_shadowed` | A defined name in the other scope already uses the new name. |
| `validation.unknown_defined_name` | No defined name has the supplied name in the supplied scope. |
| `validation.sheet_position_out_of_range` | A sheet position is outside the tab order. |
| `validation.structural_edit_empty` | A row or column insertion inserts none. |
| `validation.content_shifted_off_sheet` | An insertion would shift nonblank cells off the sheet. |
| `validation.merged_range_split` | A structural edit would move or overwrite part of a merged range. |
| `validation.table_header_shift` | A structural edit would delete, move, or overwrite a table's header row. |
| `validation.table_data_rows_deleted` | A deletion would remove every data row of a table, which Excel keeps one of. |
| `validation.table_split` | A structural edit would delete, move, or overwrite part of a table or its totals row. |
| `validation.moved_range_overwrites_references` | A range move would paste over cells that a formula outside the moved cells reads. |
| `validation.moved_range_splits_rule` | A range move would move part of a data validation whose formula reads relative to its cells. |
| `validation.copy_destination_shape` | A copy destination does not repeat the source range a whole number of times. |
| `validation.copy_unsupported_formula` | A copy would copy a formula with a spill reference, a legacy array formula, or a data-table formula. |

### Calculation sessions (`SessionErrorCode`)

`SessionError::code()` for revision, batch, cancellation, and stale-state failures of a `WorkbookCalculationSession`.

| Code | Meaning |
| --- | --- |
| `session.revision_mismatch` | The caller supplied an outdated expected revision. |
| `session.calculation_uninitialized` | Incremental calculation was requested before a complete calculation state existed. |
| `session.state_revision_mismatch` | Retained calculation state does not match the workbook revision it claims. |
| `session.incremental_unsafe` | Forced incremental calculation cannot prove a safe impact boundary. |
| `session.stale_result` | A completed calculation belongs to older workbook or calculation state and was not installed. |
| `session.limits_invalid` | One or more session resource limits were configured as zero. |
| `session.edit_batch_empty` | An edit batch contained no operation. |
| `session.edit_batch_limit_exceeded` | An edit batch exceeded its configured operation budget. |
| `session.evaluation_limit_exceeded` | One calculation pass exceeded its configured evaluated-cell budget. |
| `session.delta_limit_exceeded` | A result delta exceeded its configured changed-cell budget. |
| `session.delta_cursor_expired` | A requested delta cursor predates the retained history. |
| `session.delta_page_limit_exceeded` | A requested delta page exceeded its configured limit. |
| `session.cancelled` | Cooperative cancellation stopped a calculation before installation. |
| `session.formula_rewrite_limit_exceeded` | A whole-workbook typed formula rewrite exceeded its configured budget. |
| `session.table_materialization_limit_exceeded` | A table resize, or the cell formats a row or column insertion copies into the added lines, exceeded the configured worksheet-materialization budget. |
| `session.transaction_consumed` | A completed transaction was already installed, discarded, or found stale. |
| `session.transaction_cursor_invalid` | A transaction detail cursor belongs to another report or section. |
| `session.transaction_detail_limit_exceeded` | A transaction report exceeded its configured retained-detail budget. |
| `session.transaction_resource_limit_exceeded` | A transaction phase exceeded its configured resource budget. |

### Targeted calculation (`TargetCalculationErrorCode`)

`TargetCalculationError::code()`. Limit failures also report `requested_cell_count` or `evaluated_count`.

| Code | Meaning |
| --- | --- |
| `calculation.target.empty_targets` | The request has no targets. |
| `calculation.target.sheet_not_found` | A target names a sheet that does not exist. |
| `calculation.target.invalid_limits` | A configured limit is zero. |
| `calculation.target.target_limit_exceeded` | The request exceeds its target or result limit. |
| `calculation.target.evaluation_limit_exceeded` | The request exceeds its formula work limit. |
| `calculation.target.cancelled` | The caller cancelled the request. |
| `calculation.target.stale_result` | The session changed before the result was returned. |

### CSV (`CsvErrorCode`)

`CsvError::code()` when a CSV file cannot be read or a sheet cannot be exported.

| Code | Meaning |
| --- | --- |
| `csv.invalid_delimiter` | The delimiter is not one ASCII character other than a quote or a line break. |
| `csv.read.invalid_utf8` | The input is not UTF-8. |
| `csv.read.unterminated_quote` | A quoted field is not closed before the input ends. |
| `csv.read.invalid_quote` | A closing quote is followed by text other than a delimiter or line break. |
| `csv.read.too_many_rows` | The input has more rows than a worksheet holds. |
| `csv.read.too_many_columns` | The input has more columns than a worksheet holds. |
| `csv.read.too_many_cells` | The input has more cells than the read limit allows. |
| `csv.read.field_too_long` | A field is longer than 32,767 UTF-16 code units. |
| `csv.read.invalid_value` | The workbook rejects a value or the sheet name. |
| `csv.write.unknown_sheet` | The exported sheet is not in the workbook. |
| `csv.write.stale_calculation` | The calculation belongs to another workbook revision. |
| `csv.write.unavailable_result` | A formula in the exported sheet has no calculated value. |

### Defined-name analysis (`DefinedNameAnalysisErrorKind`)

`DefinedNameAnalysisError::kind()` when a defined-name inspection cannot complete.

| Code | Meaning |
| --- | --- |
| `defined_name.unknown_current_sheet` | The requested current sheet is absent. |
| `defined_name.resource_limit` | Analysis exceeded a configured resource limit. |
| `defined_name.cancelled` | Cooperative cancellation was requested. |

### Display text (`DisplayFormatError`)

`DisplayFormatError::code()` when `format_display_text` cannot render a value.

| Code | Meaning |
| --- | --- |
| `display_format.unsupported` | The format code uses a construct whose Excel display has not been established, such as a `*` fill, or the number needs more digits than the format engine renders. |

### Calculation issues (`CalculationIssueCode`)

`CalculationIssue::code()`: not an error but the `code` of an unavailable result, which leaves other formulas calculating. A resource-limit issue also names its `limit`.

| Code | Meaning |
| --- | --- |
| `calculation.missing_formula_text` | The workbook contains formula metadata without formula text. |
| `calculation.parse_error` | Formula text is outside the supported grammar. |
| `calculation.unsupported_function` | The parsed formula calls a function without an implemented kernel. |
| `calculation.unsupported_name` | A referenced defined name cannot be resolved or evaluated. |
| `calculation.unsupported_sheet_range` | A reference spans a 3-D sheet range, for example `Sheet1:Sheet3!A1`. |
| `calculation.unsupported_structured_reference` | A structured table reference reached a context without resolvable table geometry. This legacy classification remains stable for serialized reports. Supported structured references now calculate normally or produce the corresponding Excel error. |
| `calculation.unsupported_expression` | A parsed expression is not supported by the current engine. |
| `calculation.resource_limit_exceeded` | Formula parsing, dependency scheduling, or array evaluation exceeded a configured limit. |
| `calculation.volatile_input_missing` | A supported volatile function lacks an explicit deterministic input. |
| `calculation.circular_reference` | The formula belongs to a dependency cycle. |
| `calculation.blocked_by_upstream` | An upstream formula did not produce a value. |

## Interop

These codes come from `cellrune-interop`, the contract Python, Node.js, and MCP share, for failures of the transport layer itself: input it cannot convert, session lifecycle, pagination, previews, and serialization.

| Code | Meaning |
| --- | --- |
| `interop.calculation.target.invalid_payload` | Targeted calculation payload is invalid. |
| `interop.session.closed` | Workbook session is closed. |
| `interop.session.unavailable` | Workbook session state is unavailable. |
| `interop.session.poisoned` | Workbook session state is unusable after a native panic. |
| `interop.sheet.not_found` | Workbook does not contain the requested sheet. |
| `interop.page.limit_exceeded` | Page size exceeds the interop limit. |
| `interop.page.byte_limit_exceeded` | Range page cannot contain its first position within the byte limit. |
| `interop.page.offset_out_of_range` | Page offset exceeds the requested range. |
| `interop.page.offset_invalid` | Page offset must be a non-negative integer. |
| `interop.page.limit_invalid` | Page limit must be a non-negative integer within the unsigned 32-bit range. |
| `interop.page.byte_limit_invalid` | Page byte limit must be a non-negative integer within the unsigned 64-bit range. |
| `interop.calculation.required` | Calculate the current workbook revision before saving, exporting a formula sheet as CSV, or reading precedents and dependents. |
| `interop.value.excel_error_invalid` | Excel error value is not recognized. |
| `interop.change.invalid` | Workbook change failed transport validation. |
| `interop.sheet.id_exhausted` | Sheet identifier allocation failed. |
| `interop.calculation.request_id_exhausted` | Calculation request identifier is exhausted. |
| `interop.change.payload_invalid` | Edit batch payload is invalid. |
| `interop.session.revision_or_cursor_invalid` | Revision or cursor must be an unsigned 64-bit integer. |
| `interop.calculation.mode_invalid` | Recalculation mode must be auto, incremental, or full. |
| `interop.calculation.arithmetic_semantics_invalid` | Arithmetic semantics must be excel_near_zero or ieee_754. |
| `interop.calculation.financial_solver_semantics_invalid` | Financial solver semantics must be excel_iteration_budget or extended_search. |
| `interop.display_format.request_invalid` | Display format request is invalid. |
| `interop.calculation.locale_profile_invalid` | Locale profile must be an object whose locale and display_language are en-US or ko-KR. |
| `interop.value.number_invalid` | Number must be an integer or floating-point value; booleans are not accepted. |
| `interop.limit.invalid` | Resource limit must be greater than zero. |
| `interop.limit.payload_invalid` | Resource limits must map known limit names to positive integers. |
| `interop.workbook.archive_limit_invalid` | Archive byte limit must be greater than zero. |
| `interop.defined_name.sheet_identity_invalid` | Defined-name analysis returned an unknown sheet identity. |
| `interop.preview.not_found` | Workbook preview is not published in this session. |
| `interop.preview.cursor_invalid` | Preview cursor does not belong to this preview. |
| `interop.preview.response_limit_exceeded` | Preview response cannot contain one complete detail item within the byte limit. |
| `interop.preview.id_exhausted` | Workbook preview identifier is exhausted. |
| `interop.preview.section_invalid` | Preview detail section is not recognized. |
| `interop.dto.serialization` | Interop DTO serialization failed. |

## MCP server

These codes come from `cellrune-mcp` for configuration, filesystem roots, resident sessions, response limits, resources, and cancellation. An interop failure keeps its own code.

| Code | Meaning |
| --- | --- |
| `mcp.config.root_required` | At least one allowed workbook root is required. |
| `mcp.config.root_invalid` | Allowed workbook root is not an accessible directory. |
| `mcp.config.limit_invalid` | MCP server limit is outside the supported range. |
| `mcp.path.absolute_required` | Workbook path must be absolute. |
| `mcp.path.outside_root` | Workbook path is outside every allowed root. |
| `mcp.path.invalid` | Workbook path cannot be resolved safely. |
| `mcp.path.input_not_file` | Input workbook path is not a regular file. |
| `mcp.input.byte_limit_exceeded` | Input workbook exceeds the configured byte limit. |
| `mcp.output.overwrite_disallowed` | Server policy does not allow replacing workbooks. |
| `mcp.session.not_found` | Workbook session does not exist. |
| `mcp.session.expired` | Workbook session expired. |
| `mcp.session.busy` | Workbook session is busy. |
| `mcp.session.cache_full` | All bounded workbook session slots are busy. |
| `mcp.session.id_exhausted` | Workbook session identifier is exhausted. |
| `mcp.session.unavailable` | Workbook session cache is unavailable. |
| `mcp.response.byte_limit_exceeded` | MCP response exceeds the configured byte limit. |
| `mcp.recalculation.response_byte_limit_exceeded` | Recalculation was installed, but its delta response exceeds the configured byte limit. |
| `mcp.response.serialization_failed` | MCP response serialization failed. |
| `mcp.worker.failed` | Blocking workbook operation failed. |
| `mcp.resource.not_found` | MCP resource does not exist. |
| `mcp.resource.cursor_invalid` | MCP resource cursor is invalid. |
| `mcp.operation.cancelled` | MCP workbook operation was cancelled. |

## Node.js binding

The Node.js wrapper validates arguments and the native payloads it receives before the interop
layer sees them, and reports these codes itself.

| Code | Meaning |
| --- | --- |
| `interop.input.invalid` | An argument failed runtime validation; `details.detail` names it. |
| `interop.binding.protocol` | The native binding returned a value the wrapper could not read. |
| `interop.error.serialization` | A native error could not be serialized for the wrapper. |
