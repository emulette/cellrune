use std::error::Error;

use cellrune::{ReadOptions, XlsxErrorCode, read_xlsx_bytes};

fn main() -> Result<(), Box<dyn Error>> {
    let Err(error) = read_xlsx_bytes(b"not a zip archive", ReadOptions::default()) else {
        return Err("expected invalid input to be rejected".into());
    };

    println!("stable code: {}", error.code().as_str());
    if let Some(detail) = error.detail() {
        println!("detail: {detail}");
    }

    match error.code() {
        XlsxErrorCode::InvalidZip => {
            println!("caller action: reject the upload, do not retry");
        }
        other => {
            println!("caller action: handle {} on its own terms", other.as_str());
        }
    }

    Ok(())
}
