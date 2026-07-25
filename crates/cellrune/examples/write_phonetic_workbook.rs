use std::env;
use std::error::Error;
use std::path::PathBuf;

use cellrune::{
    CalculationOptions, CellAddress, FrozenPane, PhoneticAlignment, PhoneticProperties,
    PhoneticRun, PhoneticTextRange, PhoneticType, PhoneticWriteOptions, RecalculationWriteOptions,
    WorkbookDraft, calculate_workbook, write_xlsx_draft_path,
};

fn main() -> Result<(), Box<dyn Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: write_phonetic_workbook <output.xlsx>")?;
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let runs = vec![
        PhoneticRun::new(PhoneticTextRange::new(0, 2)?, "あした")?,
        PhoneticRun::new(PhoneticTextRange::new(3, 5)?, "がっこう")?,
    ];
    let options = PhoneticWriteOptions::show().with_properties(
        PhoneticProperties::new(0)
            .with_phonetic_type(PhoneticType::Hiragana)
            .with_alignment(PhoneticAlignment::Center),
    );
    draft.set_annotated_text(
        sheet_id,
        CellAddress::from_a1("A1")?,
        "明日は学校へ行く",
        runs,
        options,
    )?;
    draft.set_annotated_text(
        sheet_id,
        CellAddress::from_a1("A2")?,
        "😀A",
        vec![PhoneticRun::new(PhoneticTextRange::new(0, 2)?, "え")?],
        PhoneticWriteOptions::show(),
    )?;
    draft.set_annotated_text(
        sheet_id,
        CellAddress::from_a1("A3")?,
        "か\u{3099}",
        vec![PhoneticRun::new(PhoneticTextRange::new(0, 2)?, "が")?],
        PhoneticWriteOptions::show(),
    )?;
    draft.set_frozen_pane(sheet_id, FrozenPane::new(1, 1)?)?;

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_path(
        &draft,
        &calculation,
        output,
        RecalculationWriteOptions::default(),
    )?;
    Ok(())
}
