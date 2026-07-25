use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cellrune::CellContent;
use zip::CompressionMethod;
use zip::read::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn formula_at<'a>(
    document: &'a cellrune::XlsxDocument,
    sheet: &str,
    address: &str,
) -> &'a cellrune::FormulaCell {
    let cell = document
        .workbook()
        .sheet_by_name(sheet)
        .expect("sheet")
        .cell_by_a1(address)
        .expect("valid address")
        .expect("cell");
    let CellContent::Formula(formula) = cell.content() else {
        panic!("expected formula cell");
    };
    formula
}

pub(super) fn replace_part_text(source: &[u8], part: &str, from: &str, to: &str) -> Vec<u8> {
    let original = part_text(source, part);
    assert!(
        original.contains(from),
        "fixture mutation target must exist"
    );
    let mut replacements = BTreeMap::new();
    replacements.insert(part.to_owned(), original.replace(from, to));
    rewrite_archive(source, &replacements, &[])
}

pub(super) fn part_text(source: &[u8], part: &str) -> String {
    String::from_utf8(
        archive_parts(source)
            .remove(part)
            .expect("fixture part exists"),
    )
    .expect("fixture part is UTF-8")
}

pub(super) fn rewrite_archive(
    source: &[u8],
    replacements: &BTreeMap<String, String>,
    additions: &[(&str, &[u8])],
) -> Vec<u8> {
    let source_parts = archive_parts(source);
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in source_parts {
            writer
                .start_file(&name, options)
                .expect("start source part");
            if let Some(replacement) = replacements.get(&name) {
                writer
                    .write_all(replacement.as_bytes())
                    .expect("write replacement part");
            } else {
                writer.write_all(&bytes).expect("write source part");
            }
        }
        for (name, bytes) in additions {
            writer.start_file(*name, options).expect("start added part");
            writer.write_all(bytes).expect("write added part");
        }
        writer.finish().expect("finish rewritten fixture");
    }
    output.into_inner()
}

pub(super) fn archive_parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid archive");
    let mut parts = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("archive entry");
        if file.is_dir() {
            continue;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).expect("part bytes");
        assert!(parts.insert(file.name().to_owned(), contents).is_none());
    }
    parts
}

pub(super) struct TemporaryOutput {
    path: PathBuf,
}

impl TemporaryOutput {
    pub(super) fn new(extension: &str) -> Self {
        let sequence = OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "cellrune-recalculation-output-{}-{sequence}.{extension}",
            std::process::id()
        ));
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
