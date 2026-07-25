#![no_main]

use cellrune::{ReadOptions, read_xlsx_bytes};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = read_xlsx_bytes(data, ReadOptions::default());
});
