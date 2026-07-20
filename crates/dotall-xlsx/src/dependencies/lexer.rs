use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellReference {
    pub column: String,
    pub row: u32,
    pub column_absolute: bool,
    pub row_absolute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaReference {
    pub sheet: Option<String>,
    pub start: CellReference,
    pub end: Option<CellReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FormulaToken {
    Reference(FormulaReference),
    NamedRange { name: String },
}

pub fn lex(formula: &str) -> Vec<FormulaToken> {
    Scanner::new(formula).collect()
}

pub fn references(formula: &str) -> Vec<FormulaReference> {
    lex(formula)
        .into_iter()
        .filter_map(|token| match token {
            FormulaToken::Reference(reference) => Some(reference),
            FormulaToken::NamedRange { .. } => None,
        })
        .collect()
}

struct Scanner<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> Scanner<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }
}

impl Iterator for Scanner<'_> {
    type Item = FormulaToken;

    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.input.len() {
            let start = self.cursor;
            let character = self.input[start..].chars().next()?;
            if character == '"' {
                self.cursor = skip_string(self.input, start);
                continue;
            }

            if !is_identifier_boundary_before(self.input, start) {
                self.cursor += character.len_utf8();
                continue;
            }

            if let Some(consumed) = skip_external_reference(&self.input[start..]) {
                self.cursor = start + consumed;
                continue;
            }

            if let Some((reference, consumed)) = parse_reference(&self.input[start..])
                && is_identifier_boundary_after(&self.input[start + consumed..])
                && !self.input[start + consumed..].trim_start().starts_with('(')
            {
                self.cursor = start + consumed;
                return Some(FormulaToken::Reference(reference));
            }

            if let Some((name, consumed)) = parse_identifier(&self.input[start..])
                && is_identifier_boundary_after(&self.input[start + consumed..])
                && !is_builtin_identifier(name)
                && !self.input[start + consumed..].trim_start().starts_with('(')
            {
                self.cursor = start + consumed;
                return Some(FormulaToken::NamedRange {
                    name: name.to_owned(),
                });
            }

            self.cursor += character.len_utf8();
        }
        None
    }
}

fn skip_string(input: &str, start: usize) -> usize {
    let bytes = input.as_bytes();
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            if bytes.get(cursor + 1) == Some(&b'"') {
                cursor += 2;
                continue;
            }
            return cursor + 1;
        }
        cursor += 1;
    }
    bytes.len()
}

fn skip_external_reference(input: &str) -> Option<usize> {
    if input.starts_with('[') {
        return skip_bracketed_external_reference(input);
    }

    if input.starts_with('\'') {
        return skip_quoted_external_reference(input);
    }

    None
}

fn skip_bracketed_external_reference(input: &str) -> Option<usize> {
    let workbook_length = skip_bracketed_workbook(input)?;
    let after_workbook = &input[workbook_length..];
    let sheet_length = skip_sheet_prefix(after_workbook)?;
    let after_sheet = workbook_length + sheet_length;
    let (_, cell_length) = parse_cell(&input[after_sheet..])?;
    let mut consumed = after_sheet + cell_length;
    if input[consumed..].starts_with(':') {
        let (_, end_length) = parse_cell(&input[consumed + 1..])?;
        consumed += 1 + end_length;
    }
    Some(consumed)
}

fn skip_bracketed_workbook(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }

    let mut cursor = 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b']' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

fn skip_quoted_external_reference(input: &str) -> Option<usize> {
    let rest = input.strip_prefix('\'')?;
    if !rest.starts_with('[') {
        return None;
    }

    let bytes = rest.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\'' {
            if bytes.get(cursor + 1) == Some(&b'\'') {
                cursor += 2;
                continue;
            }
            if bytes.get(cursor + 1) != Some(&b'!') {
                return None;
            }
            let after_sheet = 1 + cursor + 2;
            let (_, cell_length) = parse_cell(&input[after_sheet..])?;
            let mut consumed = after_sheet + cell_length;
            if input[consumed..].starts_with(':') {
                let (_, end_length) = parse_cell(&input[consumed + 1..])?;
                consumed += 1 + end_length;
            }
            return Some(consumed);
        }
        cursor += 1;
    }
    None
}

fn skip_sheet_prefix(input: &str) -> Option<usize> {
    let (_, name_length) = parse_identifier(input)?;
    if name_length > 0 && input[name_length..].starts_with('!') {
        return Some(name_length + 1);
    }
    None
}

fn parse_reference(input: &str) -> Option<(FormulaReference, usize)> {
    let (sheet, sheet_length) = parse_sheet(input)?;
    let (start, start_length) = parse_cell(&input[sheet_length..])?;
    let mut consumed = sheet_length + start_length;
    let end = if input[consumed..].starts_with(':') {
        let (end, end_length) = parse_cell(&input[consumed + 1..])?;
        consumed += 1 + end_length;
        Some(end)
    } else {
        None
    };
    Some((FormulaReference { sheet, start, end }, consumed))
}

fn parse_sheet(input: &str) -> Option<(Option<String>, usize)> {
    if let Some(rest) = input.strip_prefix('\'') {
        let bytes = rest.as_bytes();
        let mut cursor = 0;
        let mut name = String::new();
        while cursor < bytes.len() {
            if bytes[cursor] == b'\'' {
                if bytes.get(cursor + 1) == Some(&b'\'') {
                    name.push('\'');
                    cursor += 2;
                    continue;
                }
                if bytes.get(cursor + 1) == Some(&b'!') {
                    return Some((Some(name), cursor + 3));
                }
                return None;
            }
            let character = rest[cursor..].chars().next()?;
            name.push(character);
            cursor += character.len_utf8();
        }
        return None;
    }

    let (name, name_length) = parse_identifier(input).unwrap_or(("", 0));
    if name_length > 0 && input[name_length..].starts_with('!') {
        return Some((Some(name.to_owned()), name_length + 1));
    }
    Some((None, 0))
}

fn parse_cell(input: &str) -> Option<(CellReference, usize)> {
    let bytes = input.as_bytes();
    let mut cursor = 0;
    let column_absolute = bytes.first() == Some(&b'$');
    if column_absolute {
        cursor += 1;
    }

    let column_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic) && cursor - column_start < 3 {
        cursor += 1;
    }
    if cursor == column_start || bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }

    let column = input[column_start..cursor].to_ascii_uppercase();
    let row_absolute = bytes.get(cursor) == Some(&b'$');
    if row_absolute {
        cursor += 1;
    }
    let row_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == row_start {
        return None;
    }

    let row = input[row_start..cursor].parse::<u32>().ok()?;
    let column_number = column.bytes().try_fold(0_u32, |total, letter| {
        total
            .checked_mul(26)?
            .checked_add(u32::from(letter - b'A' + 1))
    })?;
    if row == 0 || row > 1_048_576 || column_number > 16_384 {
        return None;
    }

    Some((
        CellReference {
            column,
            row,
            column_absolute,
            row_absolute,
        },
        cursor,
    ))
}

fn parse_identifier(input: &str) -> Option<(&str, usize)> {
    let mut characters = input.char_indices();
    let (_, first) = characters.next()?;
    if !is_identifier_start(first) {
        return None;
    }

    let mut end = first.len_utf8();
    for (index, character) in characters {
        if !is_identifier(character) {
            break;
        }
        end = index + character.len_utf8();
    }
    Some((&input[..end], end))
}

fn is_identifier_boundary_before(input: &str, position: usize) -> bool {
    !input[..position]
        .chars()
        .next_back()
        .is_some_and(is_identifier)
}

fn is_identifier_boundary_after(input: &str) -> bool {
    !input.chars().next().is_some_and(is_identifier)
}

fn is_identifier_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

fn is_identifier(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
}

fn is_builtin_identifier(identifier: &str) -> bool {
    identifier.eq_ignore_ascii_case("TRUE") || identifier.eq_ignore_ascii_case("FALSE")
}

#[cfg(test)]
mod tests {
    use super::{CellReference, FormulaReference, FormulaToken, lex};

    fn cell(column: &str, row: u32, column_absolute: bool, row_absolute: bool) -> CellReference {
        CellReference {
            column: column.into(),
            row,
            column_absolute,
            row_absolute,
        }
    }

    fn reference(
        sheet: Option<&str>,
        start: CellReference,
        end: Option<CellReference>,
    ) -> FormulaToken {
        FormulaToken::Reference(FormulaReference {
            sheet: sheet.map(str::to_owned),
            start,
            end,
        })
    }

    #[test]
    fn lexes_local_cell_reference() {
        assert_eq!(
            lex("=A1"),
            vec![reference(None, cell("A", 1, false, false), None)]
        );
    }

    #[test]
    fn preserves_absolute_reference_flags() {
        assert_eq!(
            lex("=$B$12"),
            vec![reference(None, cell("B", 12, true, true), None)]
        );
    }

    #[test]
    fn lexes_local_range() {
        assert_eq!(
            lex("=A1:B2"),
            vec![reference(
                None,
                cell("A", 1, false, false),
                Some(cell("B", 2, false, false)),
            )]
        );
    }

    #[test]
    fn lexes_unquoted_sheet_reference() {
        assert_eq!(
            lex("=Sheet1!A1"),
            vec![reference(Some("Sheet1"), cell("A", 1, false, false), None,)]
        );
    }

    #[test]
    fn lexes_quoted_sheet_range() {
        assert_eq!(
            lex("='My Sheet'!A1:B2"),
            vec![reference(
                Some("My Sheet"),
                cell("A", 1, false, false),
                Some(cell("B", 2, false, false)),
            )]
        );
    }

    #[test]
    fn lexes_named_ranges_separately() {
        assert_eq!(
            lex("=TaxRate * A1"),
            vec![
                FormulaToken::NamedRange {
                    name: "TaxRate".into(),
                },
                reference(None, cell("A", 1, false, false), None),
            ]
        );
    }

    #[test]
    fn ignores_references_inside_escaped_string_literals() {
        assert_eq!(
            lex(r#"="A1 ""TaxRate""" & B2"#),
            vec![reference(None, cell("B", 2, false, false), None)]
        );
    }

    #[test]
    fn ignores_non_references_and_function_names() {
        assert_eq!(
            lex("=SUM(A1) + LOG10(B2) + TRUE"),
            vec![
                reference(None, cell("A", 1, false, false), None),
                reference(None, cell("B", 2, false, false), None),
            ]
        );
    }

    #[test]
    fn ignores_external_workbook_references() {
        assert_eq!(lex("=[Other.xlsx]Inputs!A1"), Vec::<FormulaToken>::new());
        assert_eq!(
            lex("='[Other.xlsx]Inputs'!A1:B2"),
            Vec::<FormulaToken>::new()
        );
        assert_eq!(
            lex("=[Other.xlsx]Inputs!A1 + A1"),
            vec![reference(None, cell("A", 1, false, false), None)]
        );
    }
}
