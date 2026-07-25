use super::value::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Prefix,
    IntegerDigits,
    FractionDigits,
    Suffix,
}

#[derive(Debug, Default)]
struct NumberFormat {
    prefix: String,
    integer_zeros: u32,
    integer_placeholders: u32,
    grouping: bool,
    fraction_zeros: u32,
    fraction_placeholders: u32,
    percent_count: u32,
    suffix: String,
}

const UNSUPPORTED_FORMAT_LETTERS: &str = "dmyhsegDMYHSEG?*_[]";

fn compile_format(format: &str) -> Result<NumberFormat, ErrorKind> {
    if format == "@" {
        return Err(ErrorKind::Unsupported);
    }
    let mut compiled = NumberFormat::default();
    let mut section = Section::Prefix;
    let mut characters = format.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            ';' | '@' => return Err(ErrorKind::Unsupported),
            '"' => {
                let mut literal = String::new();
                loop {
                    match characters.next() {
                        Some('"') => break,
                        Some(inner) => literal.push(inner),
                        None => return Err(ErrorKind::Unsupported),
                    }
                }
                match section {
                    Section::Prefix => compiled.prefix.push_str(&literal),
                    _ => {
                        section = Section::Suffix;
                        compiled.suffix.push_str(&literal);
                    }
                }
            }
            '\\' => {
                let Some(escaped) = characters.next() else {
                    return Err(ErrorKind::Unsupported);
                };
                match section {
                    Section::Prefix => compiled.prefix.push(escaped),
                    _ => {
                        section = Section::Suffix;
                        compiled.suffix.push(escaped);
                    }
                }
            }
            '0' | '#' => {
                let is_zero = character == '0';
                match section {
                    Section::Prefix => {
                        section = Section::IntegerDigits;
                        compiled.integer_placeholders += 1;
                        if is_zero {
                            compiled.integer_zeros += 1;
                        }
                    }
                    Section::IntegerDigits => {
                        compiled.integer_placeholders += 1;
                        if is_zero {
                            compiled.integer_zeros += 1;
                        }
                    }
                    Section::FractionDigits => {
                        compiled.fraction_placeholders += 1;
                        if is_zero {
                            compiled.fraction_zeros += 1;
                        }
                    }
                    Section::Suffix => return Err(ErrorKind::Unsupported),
                }
            }
            '.' => match section {
                Section::IntegerDigits => section = Section::FractionDigits,
                Section::Prefix => {
                    section = Section::FractionDigits;
                }
                _ => return Err(ErrorKind::Unsupported),
            },
            ',' => {
                if section == Section::IntegerDigits
                    && matches!(characters.peek(), Some('0') | Some('#'))
                {
                    compiled.grouping = true;
                } else {
                    return Err(ErrorKind::Unsupported);
                }
            }
            '%' => {
                compiled.percent_count += 1;
                match section {
                    Section::Prefix => compiled.prefix.push('%'),
                    _ => {
                        section = Section::Suffix;
                        compiled.suffix.push('%');
                    }
                }
            }
            _ if UNSUPPORTED_FORMAT_LETTERS.contains(character) => {
                return Err(ErrorKind::Unsupported);
            }
            _ => match section {
                Section::Prefix => compiled.prefix.push(character),
                _ => {
                    section = Section::Suffix;
                    compiled.suffix.push(character);
                }
            },
        }
    }
    if compiled.integer_placeholders == 0 && compiled.fraction_placeholders == 0 {
        return Err(ErrorKind::Unsupported);
    }
    Ok(compiled)
}

pub fn format_number(value: f64, format: &str) -> Result<String, ErrorKind> {
    let compiled = compile_format(format)?;
    let mut scaled = value;
    for _ in 0..compiled.percent_count {
        scaled *= 100.0;
    }
    if !scaled.is_finite() {
        return Err(ErrorKind::Num);
    }
    let negative = scaled < 0.0;
    let magnitude = scaled.abs();
    let factor = 10_f64.powi(compiled.fraction_placeholders as i32);
    let scaled_units = (magnitude * factor).round();
    if scaled_units >= 9.007_199_254_740_992e15 {
        return Err(ErrorKind::Num);
    }
    let units = scaled_units as u64;
    let unit_text = units.to_string();
    let fraction_len = compiled.fraction_placeholders as usize;
    let padded = if unit_text.len() <= fraction_len {
        format!(
            "{}{}",
            "0".repeat(fraction_len + 1 - unit_text.len()),
            unit_text
        )
    } else {
        unit_text
    };
    let split = padded.len() - fraction_len;
    let mut integer_part = padded[..split].to_owned();
    let mut fraction_part = padded[split..].to_owned();
    while integer_part.len() < compiled.integer_zeros as usize {
        integer_part.insert(0, '0');
    }
    if integer_part == "0" && compiled.integer_zeros == 0 && compiled.fraction_placeholders > 0 {
        integer_part.clear();
    }
    let optional_fraction = (compiled.fraction_placeholders - compiled.fraction_zeros) as usize;
    for _ in 0..optional_fraction {
        if fraction_part.ends_with('0') {
            fraction_part.pop();
        } else {
            break;
        }
    }
    if compiled.grouping {
        let mut grouped = String::new();
        for (index, digit) in integer_part.chars().enumerate() {
            if index > 0 && (integer_part.len() - index) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(digit);
        }
        integer_part = grouped;
    }
    let mut output = String::new();
    if negative && (units > 0 || compiled.integer_zeros > 0) {
        output.push('-');
    }
    output.push_str(&compiled.prefix);
    output.push_str(&integer_part);
    if !fraction_part.is_empty() {
        output.push('.');
        output.push_str(&fraction_part);
    }
    output.push_str(&compiled.suffix);
    Ok(output)
}
