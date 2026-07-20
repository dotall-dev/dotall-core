use std::fmt;

use super::address::{
    AxisChange, RangeRef, TransformResult, parse_cell, parse_range, transform_range,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqrefError {
    value: String,
}

impl fmt::Display for SqrefError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid sqref `{}`", self.value)
    }
}

impl std::error::Error for SqrefError {}

/// Validates and transforms a whitespace-separated XML `sqref` attribute.
///
/// Fully deleted ranges are omitted. If no ranges remain, the entire attribute
/// is removed; otherwise transformed ranges are joined by one ASCII space.
pub fn transform_sqref(
    sqref: &str,
    change: AxisChange,
) -> Result<TransformResult<String>, SqrefError> {
    let mut transformed = Vec::new();
    for value in sqref.split_whitespace() {
        let range = parse_sqref_item(value)?;
        match transform_range(&range, change) {
            TransformResult::Kept(range) => {
                if range.start == range.end {
                    transformed.push(range.start.to_string());
                } else {
                    transformed.push(range.to_string());
                }
            }
            TransformResult::Removed => {}
            TransformResult::RefError => return Ok(TransformResult::RefError),
        }
    }

    if transformed.is_empty() {
        Ok(TransformResult::Removed)
    } else {
        Ok(TransformResult::Kept(transformed.join(" ")))
    }
}

fn parse_sqref_item(value: &str) -> Result<RangeRef, SqrefError> {
    if value.contains(':') {
        parse_range(value)
    } else {
        parse_cell(value).map(|cell| RangeRef {
            start: cell.clone(),
            end: cell,
        })
    }
    .map_err(|_| SqrefError {
        value: value.to_owned(),
    })
}
