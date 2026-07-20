use std::fmt;

use crate::dependencies::CellReference;

pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLUMNS: u32 = 16_384;

pub type CellRef = CellReference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRef {
    pub start: CellRef,
    pub end: CellRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisChange {
    Insert { axis: Axis, at: u32, count: u32 },
    Delete { axis: Axis, at: u32, count: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformResult<T> {
    Kept(T),
    Removed,
    RefError,
}

impl<T> TransformResult<T> {
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> TransformResult<U> {
        match self {
            Self::Kept(value) => TransformResult::Kept(transform(value)),
            Self::Removed => TransformResult::Removed,
            Self::RefError => TransformResult::RefError,
        }
    }
}

impl fmt::Display for CellReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.column_absolute {
            formatter.write_str("$")?;
        }
        formatter.write_str(&self.column)?;
        if self.row_absolute {
            formatter.write_str("$")?;
        }
        write!(formatter, "{}", self.row)
    }
}

impl fmt::Display for RangeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.start, self.end)
    }
}

pub fn parse_cell(address: &str) -> Result<CellRef, String> {
    let (cell, consumed) = parse_cell_prefix(address)
        .ok_or_else(|| format!("invalid A1 cell reference `{address}`"))?;
    if consumed != address.len() {
        return Err(format!("invalid A1 cell reference `{address}`"));
    }
    Ok(cell)
}

pub fn parse_range(address: &str) -> Result<RangeRef, String> {
    let (start, start_length) = parse_cell_prefix(address)
        .ok_or_else(|| format!("invalid A1 range reference `{address}`"))?;
    let rest = address
        .get(start_length..)
        .and_then(|rest| rest.strip_prefix(':'))
        .ok_or_else(|| format!("invalid A1 range reference `{address}`"))?;
    let end = parse_cell(rest).map_err(|_| format!("invalid A1 range reference `{address}`"))?;
    if start.row > end.row || column_number(&start.column) > column_number(&end.column) {
        return Err(format!("invalid A1 range reference `{address}`"));
    }
    Ok(RangeRef { start, end })
}

pub fn transform_cell(cell: &CellRef, change: AxisChange) -> TransformResult<CellRef> {
    let (axis, at, count, deleting) = change_parts(change);
    if at == 0 || count == 0 {
        return TransformResult::RefError;
    }

    let coordinate = axis_coordinate(cell, axis);
    let maximum = axis_maximum(axis);
    let transformed = if deleting {
        let Some(end) = at.checked_add(count - 1) else {
            return TransformResult::RefError;
        };
        if coordinate >= at && coordinate <= end {
            return TransformResult::Removed;
        }
        if coordinate > end {
            coordinate.checked_sub(count)
        } else {
            Some(coordinate)
        }
    } else if coordinate >= at {
        coordinate.checked_add(count)
    } else {
        Some(coordinate)
    };

    match transformed {
        Some(value) if value > 0 && value <= maximum => {
            TransformResult::Kept(with_axis_coordinate(cell, axis, value))
        }
        _ => TransformResult::RefError,
    }
}

pub fn transform_range(range: &RangeRef, change: AxisChange) -> TransformResult<RangeRef> {
    let (axis, at, count, deleting) = change_parts(change);
    if at == 0 || count == 0 {
        return TransformResult::RefError;
    }

    let start = axis_coordinate(&range.start, axis);
    let end = axis_coordinate(&range.end, axis);
    let maximum = axis_maximum(axis);
    let (transformed_start, transformed_end) = if deleting {
        let Some(deleted_end) = at.checked_add(count - 1) else {
            return TransformResult::RefError;
        };
        if start >= at && end <= deleted_end {
            return TransformResult::Removed;
        }

        let transformed_start = if start < at {
            start
        } else if start > deleted_end {
            start - count
        } else {
            at
        };
        let transformed_end = if end < at {
            end
        } else if end > deleted_end {
            end - count
        } else {
            at - 1
        };
        (transformed_start, transformed_end)
    } else {
        (
            if start >= at {
                match start.checked_add(count) {
                    Some(value) => value,
                    None => return TransformResult::RefError,
                }
            } else {
                start
            },
            if end >= at {
                match end.checked_add(count) {
                    Some(value) => value,
                    None => return TransformResult::RefError,
                }
            } else {
                end
            },
        )
    };

    if transformed_start == 0
        || transformed_end == 0
        || transformed_start > transformed_end
        || transformed_end > maximum
    {
        return TransformResult::RefError;
    }

    TransformResult::Kept(RangeRef {
        start: with_axis_coordinate(&range.start, axis, transformed_start),
        end: with_axis_coordinate(&range.end, axis, transformed_end),
    })
}

pub(crate) fn parse_cell_prefix(address: &str) -> Option<(CellRef, usize)> {
    let bytes = address.as_bytes();
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

    let column = address[column_start..cursor].to_ascii_uppercase();
    let column_number = column_number(&column);
    let row_absolute = bytes.get(cursor) == Some(&b'$');
    if row_absolute {
        cursor += 1;
    }

    let row_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    let row = address[row_start..cursor].parse::<u32>().ok()?;
    if row == 0 || row > MAX_ROWS || column_number > MAX_COLUMNS {
        return None;
    }

    Some((
        CellRef {
            column,
            row,
            column_absolute,
            row_absolute,
        },
        cursor,
    ))
}

pub(crate) fn column_number(column: &str) -> u32 {
    column.bytes().fold(0, |number, letter| {
        number * 26 + u32::from(letter.to_ascii_uppercase() - b'A' + 1)
    })
}

pub(crate) fn column_name(mut number: u32) -> String {
    let mut letters = Vec::new();
    while number > 0 {
        number -= 1;
        letters.push(char::from(b'A' + (number % 26) as u8));
        number /= 26;
    }
    letters.iter().rev().collect()
}

fn change_parts(change: AxisChange) -> (Axis, u32, u32, bool) {
    match change {
        AxisChange::Insert { axis, at, count } => (axis, at, count, false),
        AxisChange::Delete { axis, at, count } => (axis, at, count, true),
    }
}

fn axis_coordinate(cell: &CellRef, axis: Axis) -> u32 {
    match axis {
        Axis::Row => cell.row,
        Axis::Column => column_number(&cell.column),
    }
}

fn axis_maximum(axis: Axis) -> u32 {
    match axis {
        Axis::Row => MAX_ROWS,
        Axis::Column => MAX_COLUMNS,
    }
}

fn with_axis_coordinate(cell: &CellRef, axis: Axis, coordinate: u32) -> CellRef {
    let mut transformed = cell.clone();
    match axis {
        Axis::Row => transformed.row = coordinate,
        Axis::Column => transformed.column = column_name(coordinate),
    }
    transformed
}
