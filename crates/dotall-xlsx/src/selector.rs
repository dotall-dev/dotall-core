#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellAddress {
    pub row: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Preview,
    Full,
    Sheet {
        name: String,
    },
    Range {
        name: String,
        start: CellAddress,
        end: CellAddress,
    },
    AstRange {
        name: String,
        start: CellAddress,
        end: CellAddress,
    },
    NamedRanges,
    Merges {
        name: String,
    },
}

pub fn parse(kind: &str, value: &str) -> std::result::Result<Selector, String> {
    match kind {
        "full" => Ok(Selector::Full),
        "sheet" if !value.trim().is_empty() => Ok(Selector::Sheet {
            name: value.trim().to_owned(),
        }),
        "range" => {
            parse_range(value).map(|(name, start, end)| Selector::Range { name, start, end })
        }
        "ast_range" => {
            parse_range(value).map(|(name, start, end)| Selector::AstRange { name, start, end })
        }
        "named_ranges" => Ok(Selector::NamedRanges),
        "merges" if !value.trim().is_empty() => Ok(Selector::Merges {
            name: value.trim().to_owned(),
        }),
        "merges" => Err("merges selector requires a sheet name".into()),
        "sheet" => Err("sheet selector requires a sheet name".into()),
        _ => Err(format!("unsupported selector `{kind}`")),
    }
}

fn parse_range(value: &str) -> std::result::Result<(String, CellAddress, CellAddress), String> {
    let (sheet_name, range) = value
        .rsplit_once('!')
        .ok_or_else(|| "range must use Sheet!A1:D20 syntax".to_owned())?;
    if sheet_name.is_empty() {
        return Err("range selector requires a sheet name".into());
    }

    let (start, end) = range.split_once(':').unwrap_or((range, range));
    let start = parse_address(start)?;
    let end = parse_address(end)?;

    Ok((
        sheet_name.to_owned(),
        CellAddress {
            row: start.row.min(end.row),
            col: start.col.min(end.col),
        },
        CellAddress {
            row: start.row.max(end.row),
            col: start.col.max(end.col),
        },
    ))
}

pub fn parse_cell_address(value: &str) -> std::result::Result<CellAddress, String> {
    parse_address(value)
}

fn parse_address(value: &str) -> std::result::Result<CellAddress, String> {
    let value = value.trim().trim_matches('$');
    let letters = value
        .chars()
        .take_while(|character| character.is_ascii_alphabetic())
        .count();
    let (column, row) = value.split_at(letters);
    if column.is_empty()
        || row.is_empty()
        || !row.chars().all(|character| character.is_ascii_digit())
    {
        return Err(format!("invalid cell address `{value}`"));
    }

    let col = column.chars().try_fold(0_u32, |value, character| {
        let uppercase = character.to_ascii_uppercase();
        if !uppercase.is_ascii_uppercase() {
            return Err(());
        }
        Ok(value * 26 + u32::from(uppercase as u8 - b'A' + 1))
    });
    let row = row
        .parse::<u32>()
        .map_err(|_| format!("invalid row in cell address `{value}`"))?;

    match (col, row) {
        (Ok(col), 1..) => Ok(CellAddress { row, col }),
        _ => Err(format!("invalid cell address `{value}`")),
    }
}
