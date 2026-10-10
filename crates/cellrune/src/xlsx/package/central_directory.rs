use std::io::{self, Read, Seek, SeekFrom};

const END_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
const ZIP64_END_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];
const CENTRAL_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const END_BYTES: usize = 22;
const ZIP64_LOCATOR_BYTES: usize = 20;
const ZIP64_END_BYTES: usize = 56;
const CENTRAL_HEADER_BYTES: u64 = 46;
const SCAN_WINDOW_BYTES: usize = 4 * 1024;

/// Returns the number of entries the ZIP reader will load from the central directory, as
/// declared by the end record it tries first, or `None` when it would find no usable record.
///
/// `ZipArchive::new` reads and keeps every central-directory header before its length can be
/// compared with the entry limit, so the limit has to be held against this declaration instead.
/// The search mirrors the `zip` crate's `spec::find_central_directory` and the retry in
/// `ZipArchive::get_metadata`: end-record signatures are tried from the end of the input
/// backwards, and a record the reader skips for failing a consistency check is skipped here too,
/// so a decoy record cannot stand in for the one the reader loads. The reader also moves on when
/// a central-directory header fails to parse; that is not mirrored, and the length check after
/// opening still covers the record it settles on instead.
pub(super) fn declared_entry_count<R: Read + Seek>(
    reader: &mut R,
    archive_bytes: u64,
) -> io::Result<Option<u64>> {
    let mut end = archive_bytes;
    while let Some(offset) = rfind_signature(reader, END_SIGNATURE, 0, end)? {
        if let Some(entries) = end_record_entry_count(reader, offset, archive_bytes)? {
            return Ok(Some(entries));
        }
        end = offset;
    }
    Ok(None)
}

/// Returns the entry count the reader loads through the end record at `offset`, or `None` when
/// it skips the record.
fn end_record_entry_count<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    archive_bytes: u64,
) -> io::Result<Option<u64>> {
    let record_end = offset + END_BYTES as u64;
    if record_end > archive_bytes {
        return Ok(None);
    }
    let mut record = [0_u8; END_BYTES];
    read_at(reader, offset, &mut record)?;
    let disk = u16::from_le_bytes(field(&record, 4));
    let directory_disk = u16::from_le_bytes(field(&record, 6));
    let entries_on_disk = u16::from_le_bytes(field(&record, 8));
    let entries = u16::from_le_bytes(field(&record, 10));
    let directory_size = u32::from_le_bytes(field(&record, 12));
    let directory_offset = u32::from_le_bytes(field(&record, 16));
    let comment_bytes = u16::from_le_bytes(field(&record, 20));
    // The comment must fit in the input; bytes after it are tolerated.
    if record_end + u64::from(comment_bytes) > archive_bytes {
        return Ok(None);
    }

    if (entries == u16::MAX || directory_size == u32::MAX || directory_offset == u32::MAX)
        && let Some(locator_offset) = offset.checked_sub(ZIP64_LOCATOR_BYTES as u64)
    {
        let mut locator = [0_u8; ZIP64_LOCATOR_BYTES];
        read_at(reader, locator_offset, &mut locator)?;
        if locator[..4] == ZIP64_LOCATOR_SIGNATURE {
            return zip64_entry_count(reader, locator_offset, &locator, archive_bytes);
        }
    }
    // Without a ZIP64 locator the reader loads the count for this disk, and unless the archive
    // is empty it first finds a header signature between the directory offset and this record.
    if disk != directory_disk {
        return Ok(None);
    }
    let directory_offset = u64::from(directory_offset);
    let has_directory = entries == 0
        || (directory_offset < offset
            && find_signature(reader, CENTRAL_HEADER_SIGNATURE, directory_offset, offset)?
                .is_some());
    Ok(has_directory.then_some(u64::from(entries_on_disk)))
}

/// Returns the entry count the reader loads through the ZIP64 locator at `locator_offset`, or
/// `None` when it skips the end record that holds the locator.
fn zip64_entry_count<R: Read + Seek>(
    reader: &mut R,
    locator_offset: u64,
    locator: &[u8; ZIP64_LOCATOR_BYTES],
    archive_bytes: u64,
) -> io::Result<Option<u64>> {
    let directory_disk = u32::from_le_bytes(field(locator, 4));
    let declared_offset = u64::from_le_bytes(field(locator, 8));
    let disks = u32::from_le_bytes(field(locator, 16));
    if declared_offset >= locator_offset || disks > 1 {
        return Ok(None);
    }
    // Data prepended to the archive moves the record past its declared offset, so the reader
    // tries every record signature from there up to the locator.
    let mut start = declared_offset;
    while let Some(offset) = find_signature(reader, ZIP64_END_SIGNATURE, start, locator_offset)? {
        start = offset + 1;
        if offset + ZIP64_END_BYTES as u64 > archive_bytes {
            continue;
        }
        let mut record = [0_u8; ZIP64_END_BYTES];
        read_at(reader, offset, &mut record)?;
        let record_size = u64::from_le_bytes(field(&record, 4));
        let disk = u32::from_le_bytes(field(&record, 16));
        let record_directory_disk = u32::from_le_bytes(field(&record, 20));
        let entries_on_disk = u64::from_le_bytes(field(&record, 24));
        let entries = u64::from_le_bytes(field(&record, 32));
        let directory_offset = u64::from_le_bytes(field(&record, 48));
        // The record must end at the locator, agree with it on the directory's disk, and leave
        // room before itself for a minimal header per declared entry.
        if record_size < 40
            || record_size.saturating_add(12) != locator_offset - offset
            || record_directory_disk != directory_disk
            || offset
                < entries
                    .saturating_mul(CENTRAL_HEADER_BYTES)
                    .saturating_add(directory_offset)
        {
            continue;
        }
        // The reader settles on this record and only then rejects counts or disks it cannot load.
        return Ok(
            (entries_on_disk <= entries && disk == record_directory_disk).then_some(entries),
        );
    }
    Ok(None)
}

/// Returns the offset of the last `signature` lying wholly inside `start..end`.
fn rfind_signature<R: Read + Seek>(
    reader: &mut R,
    signature: [u8; 4],
    start: u64,
    mut end: u64,
) -> io::Result<Option<u64>> {
    let mut window = [0_u8; SCAN_WINDOW_BYTES];
    while end.saturating_sub(start) >= 4 {
        let window_start = end.saturating_sub(SCAN_WINDOW_BYTES as u64).max(start);
        let bytes = &mut window[..(end - window_start) as usize];
        read_at(reader, window_start, bytes)?;
        if let Some(index) = bytes.windows(4).rposition(|bytes| bytes == signature) {
            return Ok(Some(window_start + index as u64));
        }
        // Keep three bytes of overlap for a signature straddling the window boundary.
        end = window_start + 3;
    }
    Ok(None)
}

/// Returns the offset of the first `signature` lying wholly inside `start..end`.
fn find_signature<R: Read + Seek>(
    reader: &mut R,
    signature: [u8; 4],
    mut start: u64,
    end: u64,
) -> io::Result<Option<u64>> {
    let mut window = [0_u8; SCAN_WINDOW_BYTES];
    while end.saturating_sub(start) >= 4 {
        let window_end = start.saturating_add(SCAN_WINDOW_BYTES as u64).min(end);
        let bytes = &mut window[..(window_end - start) as usize];
        read_at(reader, start, bytes)?;
        if let Some(index) = bytes.windows(4).position(|bytes| bytes == signature) {
            return Ok(Some(start + index as u64));
        }
        // Keep three bytes of overlap for a signature straddling the window boundary.
        start = window_end - 3;
    }
    Ok(None)
}

fn read_at<R: Read + Seek>(reader: &mut R, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(bytes)
}

fn field<const N: usize>(record: &[u8], offset: usize) -> [u8; N] {
    let mut bytes = [0; N];
    bytes.copy_from_slice(&record[offset..offset + N]);
    bytes
}
