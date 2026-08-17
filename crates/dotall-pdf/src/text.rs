//! Best-effort page text extraction from PDF content streams.
//!
//! Prefers operator-aware decoding (`Tj` / `TJ` / `'` / `"`) with line breaks from
//! `Td` / `TD` / `T*` / `ET`, then falls back to lopdf's encoding-aware extractor
//! and finally a literal-string scan.

use lopdf::content::Content;
use lopdf::{Document, Object};

/// Extract readable text for one page.
pub fn page_text(document: &Document, page_id: lopdf::ObjectId, number: u32) -> String {
    let content = document.get_page_content(page_id);
    let from_ops = extract_from_operations(&content);
    if !from_ops.is_empty() {
        return from_ops;
    }

    if let Ok(text) = document.extract_text(&[number]) {
        let trimmed = normalize_whitespace(&text);
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    extract_literals(&content)
}

fn extract_from_operations(content: &[u8]) -> String {
    let Ok(decoded) = Content::decode(content) else {
        return String::new();
    };

    let mut texts = Vec::new();
    let mut current = String::new();

    for operation in &decoded.operations {
        match operation.operator.as_str() {
            "Tj" => {
                append_show_operands(&mut current, &operation.operands);
            }
            "TJ" => {
                append_tj(&mut current, &operation.operands);
            }
            "'" => {
                push_line_break(&mut current);
                append_show_operands(&mut current, &operation.operands);
            }
            "\"" => {
                push_line_break(&mut current);
                if let Some(string_operand) = operation.operands.get(2) {
                    append_show_operands(&mut current, std::slice::from_ref(string_operand));
                }
            }
            "Td" | "TD" => {
                if td_starts_new_line(&operation.operands) {
                    push_line_break(&mut current);
                }
            }
            "T*" => push_line_break(&mut current),
            "ET" if !current.is_empty() => {
                texts.push(std::mem::take(&mut current));
            }
            "ET" => {}
            _ => {}
        }
    }
    if !current.is_empty() {
        texts.push(current);
    }

    normalize_whitespace(&texts.join("\n"))
}

fn append_tj(out: &mut String, operands: &[Object]) {
    let Some(Object::Array(items)) = operands.first() else {
        append_show_operands(out, operands);
        return;
    };
    for item in items {
        match item {
            Object::String(_, _) => {
                if let Some(text) = object_text(item) {
                    out.push_str(&text);
                }
            }
            Object::Integer(value)
                if *value <= -100 && !out.ends_with(|ch: char| ch.is_whitespace()) =>
            {
                // Large negative kerning commonly stands in for a word space.
                out.push(' ');
            }
            Object::Real(value)
                if f64::from(*value) <= -100.0 && !out.ends_with(|ch: char| ch.is_whitespace()) =>
            {
                out.push(' ');
            }
            _ => {}
        }
    }
}

fn append_show_operands(out: &mut String, operands: &[Object]) {
    for operand in operands {
        match operand {
            Object::Array(_) => append_tj(out, std::slice::from_ref(operand)),
            _ => {
                if let Some(text) = object_text(operand) {
                    out.push_str(&text);
                }
            }
        }
    }
}

fn object_text(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(decode_pdf_string(bytes)),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

fn decode_pdf_string(bytes: &[u8]) -> String {
    // Content::decode already unescapes literal strings; prefer UTF-8, else lossy.
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) if bytes.len().is_multiple_of(2) && looks_like_utf16_be(bytes) => {
            decode_utf16_be(bytes)
        }
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn looks_like_utf16_be(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xfe, 0xff])
        || bytes
            .chunks_exact(2)
            .take(4)
            .all(|pair| pair[0] == 0 && (pair[1].is_ascii_graphic() || pair[1] == b' '))
}

fn decode_utf16_be(bytes: &[u8]) -> String {
    let start = if bytes.starts_with(&[0xfe, 0xff]) {
        2
    } else {
        0
    };
    let units: Vec<u16> = bytes[start..]
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn td_starts_new_line(operands: &[Object]) -> bool {
    let Some(ty) = operands.get(1).and_then(object_number) else {
        return false;
    };
    // Moving down the page (PDF y-up) usually starts a new visual line.
    ty < -1.0
}

fn object_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

fn push_line_break(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn normalize_whitespace(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Last-resort scan for literal `(...)` strings in raw content bytes.
fn extract_literals(content: &[u8]) -> String {
    let mut texts = Vec::new();
    let mut cursor = 0;
    while cursor < content.len() {
        match content[cursor] {
            b'(' => {
                cursor += 1;
                let mut bytes = Vec::new();
                while cursor < content.len() && content[cursor] != b')' {
                    if content[cursor] == b'\\' && cursor + 1 < content.len() {
                        cursor += 1;
                    }
                    bytes.push(content[cursor]);
                    cursor += 1;
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if !text.trim().is_empty() {
                    texts.push(text);
                }
            }
            b'<' => {
                // Hex string `<48656C6C6F>` — skip dictionary `<<`.
                if content.get(cursor + 1) == Some(&b'<') {
                    cursor += 2;
                    continue;
                }
                cursor += 1;
                let start = cursor;
                while cursor < content.len() && content[cursor] != b'>' {
                    cursor += 1;
                }
                if let Some(decoded) = decode_hex_string(&content[start..cursor])
                    && !decoded.trim().is_empty()
                {
                    texts.push(decoded);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    normalize_whitespace(&texts.join(" "))
}

fn decode_hex_string(hex: &[u8]) -> Option<String> {
    let filtered: Vec<u8> = hex
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if filtered.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(filtered.len().div_ceil(2));
    let mut idx = 0;
    while idx + 1 < filtered.len() {
        let hi = from_hex(filtered[idx])?;
        let lo = from_hex(filtered[idx + 1])?;
        bytes.push((hi << 4) | lo);
        idx += 2;
    }
    if idx < filtered.len() {
        bytes.push(from_hex(filtered[idx])? << 4);
    }
    Some(decode_pdf_string(&bytes))
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_from_operations, extract_literals};

    #[test]
    fn extracts_tj_and_line_breaks_from_td() {
        let content =
            b"BT /F1 12 Tf 20 300 Td (Title) Tj 0 -24 Td (Name) Tj 0 -24 Td (Email) Tj ET";
        let text = extract_from_operations(content);
        assert!(text.contains("Title"));
        assert!(text.contains("Name"));
        assert!(text.contains("Email"));
        assert!(text.matches('\n').count() >= 2);
    }

    #[test]
    fn extracts_tj_array_with_kerning_space() {
        let content = b"BT [(Hello) -200 (World)] TJ ET";
        let text = extract_from_operations(content);
        assert_eq!(text, "Hello World");
    }

    #[test]
    fn extracts_hex_string_literals() {
        let content = b"BT <48656C6C6F> Tj ET";
        let text = extract_from_operations(content);
        assert_eq!(text, "Hello");
    }

    #[test]
    fn literal_fallback_finds_parenthesized_text() {
        let text = extract_literals(b"noise (Vendor Intake Form) more");
        assert_eq!(text, "Vendor Intake Form");
    }
}
